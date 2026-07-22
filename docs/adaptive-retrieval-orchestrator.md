# Adaptive Retrieval Orchestrator — Silent Mode (Detailed Spec)

This ties together `grounding-faithfulness-plan.md` and `source-expansion-plan.md` into one concrete algorithm, with exact integration points in the existing codebase. **This runs entirely silently** — no status line, no "checking Wikipedia" narration, no visible stage indicators anywhere in the UI. The user sends a question and gets a sharp, well-grounded answer back with no exposed mechanics in between. The intelligence shows up in the *quality and judgment* of the answer itself, not in a visible process.

This version goes deeper than the earlier summary: concrete data types, a real scoring formula, concurrency/timeout handling, worked examples, and a testing plan — detailed enough to implement directly from.

---

## 1. Where This Plugs Into the Existing Codebase

| File | Change | Role |
|---|---|---|
| `src-tauri/src/web_search/orchestrator.rs` | **new** | Owns the full plan → retrieve → judge → refine → synthesize → generate → verify loop. Single entry point called from the chat command handler instead of calling `manager.rs` directly. |
| `src-tauri/src/web_search/query.rs` | modify | Extend the existing search/no-search boolean decision into a `QueryPlan` producer (see §2.1). |
| `src-tauri/src/web_search/source_router.rs` | **new** | Pattern-matches each sub-question to a source hint (Wikipedia / weather / currency / stocks / sports / news / package-registry / general-web). |
| `src-tauri/src/web_search/sources/wikipedia.rs`, `weather.rs`, `currency.rs`, `sports.rs`, `news.rs`, `registry.rs` | **new**, grouped under a `sources/` submodule | Each implements a shared `SourceProvider` trait (see §2.3) alongside the existing `searxng.rs`/`duckduckgo.rs`. |
| `src-tauri/src/web_search/manager.rs` | modify | Demoted from top-level entry point to `search_and_rank(sub_question) -> RankedEvidence`, called once per sub-question by the orchestrator rather than once per user turn. |
| `src-tauri/src/web_search/bm25.rs` | modify | Add a second-stage semantic reranker (§2.2) applied to the top-N BM25 candidates before they reach the orchestrator's judge stage. |
| `src-tauri/src/engine/context_manager.rs` | modify | New `allocate_proportional(budget, Vec<SubQuestionEvidence>, weights)` entry point, replacing the single-blob evidence allocation. |
| `src-tauri/src/engine/faithfulness.rs` | **new** | Post-generation atomic-claim verification (§2.6), called once after generation completes. |
| Frontend | none | No new events, no new components. See §3. |

---

## 2. Data Structures & Stage Detail

### 2.1 Stage 0 — Plan

```rust
struct SubQuestion {
    id: Uuid,
    text: String,                     // the sub-question itself
    source_hint: SourceHint,          // classification result
    depends_on: Option<Uuid>,         // rare: sequential dependency between sub-questions
}

enum SourceHint {
    Wikipedia,
    Weather { location_text: String },
    Currency { from: String, to: String },
    StockOrCrypto { ticker: String },
    Sports { teams_or_league: String },
    News,
    PackageRegistry { ecosystem: Ecosystem, package: String },
    GeneralWeb,
}

struct QueryPlan {
    original_query: String,
    sub_questions: Vec<SubQuestion>,
    is_compound: bool,                // true if decomposition actually split into >1
}
```

```
fn plan_query(user_query, conversation_context) -> QueryPlan {
    // Step 1: cheap heuristic first — look for conjunctions/multiple question marks/
    // enumerated asks ("and", "also", multiple "?", "compare X to Y") before paying
    // for a full decomposition model call. Only escalate to a model-based decomposition
    // call if the heuristic is ambiguous (e.g. long query, multiple distinct entities).
    if is_trivially_simple(user_query):
        sub_questions = [SubQuestion { text: user_query, .. }]
    else if heuristic_suggests_compound(user_query):
        sub_questions = decompose_via_lightweight_model_call(user_query, conversation_context)
    else:
        sub_questions = [SubQuestion { text: user_query, .. }]

    for sub_q in sub_questions:
        sub_q.source_hint = source_router::classify(sub_q.text)

    return QueryPlan { original_query: user_query, sub_questions, is_compound: sub_questions.len() > 1 }
}
```

**Cost control note:** decomposition itself costs a small model call. Gate it behind the cheap heuristic first — most queries are simple and shouldn't pay this cost. Track this as a metric (`decomposition_triggered_rate`) in local logs to catch the heuristic being too aggressive.

### 2.2 Stage 1 — Retrieve

```rust
trait SourceProvider {
    fn matches(&self, hint: &SourceHint) -> bool;
    async fn fetch(&self, sub_q: &SubQuestion) -> Result<RawEvidence, SourceError>;
}

struct RawEvidence {
    chunks: Vec<EvidenceChunk>,
    source_kind: SourceKind,           // Dedicated(Wikipedia) | Dedicated(Weather) | Web
}

struct EvidenceChunk {
    text: String,
    source_url: String,
    source_title: String,
    host: String,                      // for cross-provider/host diversity checks
}
```

```
fn retrieve_for(sub_q, timeout_budget) -> RawEvidence {
    if let Some(provider) = source_router::dedicated_provider_for(sub_q.source_hint):
        match provider.fetch(sub_q).with_timeout(DEDICATED_SOURCE_TIMEOUT):    // e.g. 2s
            Ok(result) if result.chunks.is_not_empty() -> {
                if source_hint is inherently self-sufficient (weather, currency, stock):
                    return result   // no need to also hit general web search
                // else (e.g. Wikipedia): supplement with a light general search too
                web = manager::search_and_rank(sub_q, max_results = 3)
                return merge(result, web)
            }
            Err(_) or empty -> fall through to general web search below

    return manager::search_and_rank(sub_q, max_results = 6)   // existing per-sub-question pipeline
}
```

**Timeout handling is explicit and mandatory** — a dedicated source that hangs must not stall the whole request. `DEDICATED_SOURCE_TIMEOUT` (~2s) is short because these are meant to be fast structured lookups; if a dedicated provider is slow, treat that as a failure and fall through to general web search rather than waiting.

**Concurrency:** all `sub_questions` in a `QueryPlan` retrieve in parallel (`tokio::join_all` or similar) — there's no reason to serialize independent sub-questions. Only serialize if `depends_on` is set (rare — e.g. "who is the current CEO of the company that makes X" needs X resolved before the CEO lookup).

### 2.3 Stage 2 — Judge (scoring detail)

```rust
struct Confidence {
    relevance: f32,     // 0.0-1.0, from semantic reranker
    agreement: f32,     // 0.0-1.0, cross-source/cross-host corroboration
    coverage: f32,      // 0.0-1.0, does evidence address the sub-question's key terms
    combined: f32,       // weighted sum, see below
}

const W_RELEVANCE: f32 = 0.5;
const W_AGREEMENT: f32 = 0.2;
const W_COVERAGE: f32  = 0.3;
const REFINEMENT_THRESHOLD: f32 = 0.55;   // below this, trigger one refinement
const WEAK_THRESHOLD: f32 = 0.4;          // below this even after refinement, tag as `weak`
```

```
fn judge_sufficiency(sub_q, evidence) -> Confidence {
    relevance = semantic_rerank_top_score(sub_q.text, evidence.chunks)   // bm25.rs new stage
    agreement = distinct_host_count(evidence.chunks) >= 2
                  ? corroboration_score(evidence.chunks)                  // do independent hosts agree?
                  : 0.5   // neutral score when only one source, not penalized outright
                          // (a single strong Wikipedia hit shouldn't be marked "low agreement")
    coverage  = key_term_coverage(sub_q.text, evidence.chunks)            // cheap term-overlap check,
                                                                          // not a full semantic pass —
                                                                          // this stage runs per sub-question
                                                                          // and must stay fast
    combined = W_RELEVANCE * relevance + W_AGREEMENT * agreement + W_COVERAGE * coverage
    return Confidence { relevance, agreement, coverage, combined }
}
```

**Why `agreement` is neutral (not zero) for single-source evidence:** a single authoritative source (e.g. Wikipedia, or an official docs page) shouldn't be penalized just because there was nothing to cross-check against — that would bias the system toward always preferring noisier multi-source web results over a single clean authoritative one. Agreement is a *bonus* signal, not a requirement.

**Refinement branch:**
```
if confidence.combined < REFINEMENT_THRESHOLD:
    refined_sub_q = refine_query(sub_q)   // widen terms, try synonym, or force GeneralWeb
                                            // even if original hint was a dedicated source
    evidence = retrieve_for(refined_sub_q, timeout_budget)
    confidence = judge_sufficiency(refined_sub_q, evidence)   // exactly one retry, no loop
    if confidence.combined < WEAK_THRESHOLD:
        sub_q.evidence_quality = EvidenceQuality::Weak
    else:
        sub_q.evidence_quality = EvidenceQuality::Adequate
else:
    sub_q.evidence_quality = EvidenceQuality::Strong
```

### 2.4 `refine_query` — concrete strategies (tried in this order, first success wins)
1. **Broaden**: drop the most specific/narrow term from the sub-question (e.g. drop a qualifier clause)
2. **Synonym substitution**: swap a low-frequency term for a more common synonym (helps when the source's wording differs from the query's wording)
3. **Force general web**: if the original hint was a dedicated source that came back thin, retry with `SourceHint::GeneralWeb` explicitly, bypassing the dedicated provider entirely
4. Only one of these strategies is applied per retry (not all three) — pick based on *why* the score was low: low `coverage` → broaden; low `relevance` → synonym substitution; dedicated source returned empty/thin → force general web

### 2.5 Stage 3 — Synthesize (context allocation detail)

```
fn allocate_context(sub_questions_with_evidence) -> PromptContext {
    total_budget = context_manager::available_evidence_budget()   // existing dynamic n_ctx-based calc

    // Weight inversely to evidence_quality: a Weak sub-question doesn't need MORE space
    // just because it struggled — it needs the model to know it's weak (via a marker),
    // not to be given a bigger chunk of context to compensate.
    // Weight instead by how much genuinely relevant material was found — Strong evidence
    // with lots of good material gets proportionally more room than a Weak sub-question
    // that only turned up a thin snippet, since padding weak evidence with more tokens
    // doesn't make it more true.
    weights = [min(evidence.chunk_count, MAX_CHUNKS_PER_SUBQ) * confidence.combined
               for (evidence, confidence) in sub_questions_with_evidence]

    return context_manager::allocate_proportional(total_budget, sub_questions_with_evidence, weights)
}
```

### 2.6 Stage 5 — Verify (faithfulness detail)

```
fn faithfulness_check(response, merged_evidence) -> FinalResponse {
    claims = extract_atomic_claims(response)             // sentence-level split is enough, don't
                                                            // over-engineer a full claim-extraction model
    for claim in claims:
        support_score = max(embedding_similarity(claim, chunk) for chunk in merged_evidence.chunks)
        if support_score < CLAIM_SUPPORT_THRESHOLD:        // e.g. 0.6
            claim.flagged = true

    if any claim.flagged:
        corrected = regenerate_with_correction_note(response, flagged_claims, merged_evidence)
        // one correction pass only — if still flagged after this, ship the corrected version
        // anyway rather than looping; log it for review, don't block the user indefinitely
        return corrected
    return response
}
```

---

## 3. Explicitly No UI Exposure

- No status events for planning, retrieval, source selection, judgment/refinement, or verification
- No thinking-summary content surfaces sub-question breakdown, source choice, or confidence scores
- The frontend's existing generic "generating…" state is the only visible indicator — nothing new is added to the streaming/event contract between backend and frontend for this feature
- Internally: log every stage transition (which source, timing, confidence scores, whether refinement/correction fired) to local structured logs for your own tuning — never surface to chat UI

---

## 4. Full Loop (concrete pseudocode)

```
async fn answer(user_query, conversation_context) -> FinalResponse {
    let plan = plan_query(user_query, conversation_context);                      // §2.1

    let sub_results: Vec<SubQuestionResult> = plan.sub_questions
        .into_iter()
        .map(|sub_q| async move {
            let evidence = retrieve_for(&sub_q, DEDICATED_SOURCE_TIMEOUT).await;   // §2.2
            let confidence = judge_sufficiency(&sub_q, &evidence);                 // §2.3
            let (final_evidence, final_confidence, quality) = if confidence.combined < REFINEMENT_THRESHOLD {
                let refined = refine_query(&sub_q, &confidence);                   // §2.4
                let evidence2 = retrieve_for(&refined, DEDICATED_SOURCE_TIMEOUT).await;
                let confidence2 = judge_sufficiency(&refined, &evidence2);
                let quality = if confidence2.combined < WEAK_THRESHOLD { Weak } else { Adequate };
                (evidence2, confidence2, quality)
            } else {
                (evidence, confidence, Strong)
            };
            SubQuestionResult { sub_q, evidence: final_evidence, confidence: final_confidence, quality }
        })
        .collect_concurrently()                                                    // parallel, joined
        .await;

    let context = allocate_context(&sub_results);                                  // §2.5
    let response = generate(&user_query, &context, &sub_results).await;            // Stage 4 — existing pipeline,
                                                                                     // now with weak-evidence markers
                                                                                     // injected into the system prompt
    let final_response = faithfulness_check(response, &context).await;             // §2.6
    final_response   // only this crosses back to the frontend
}
```

---

## 5. Worked Examples (for intuition / test-case seeding)

**Example A — simple factual, single source, no refinement needed**
Query: "What year was the Eiffel Tower completed?"
- Plan: 1 sub-question, hint = Wikipedia
- Retrieve: Wikipedia hit, clean infobox-style fact
- Judge: relevance high, coverage high, agreement neutral (single source) → `combined` well above threshold, no refinement
- Generate: answers directly, no hedging
- Total extra latency over a non-grounded answer: roughly one fast API call

**Example B — compound question, mixed source types**
Query: "What's the weather in Chiang Mai right now, and who's currently the mayor?"
- Plan: 2 sub-questions — [weather hint], [Wikipedia/general hint]
- Retrieve (parallel): weather API returns current conditions immediately; mayor lookup goes to Wikipedia, may be thin if the article is outdated
- Judge: weather sub-question scores high immediately; mayor sub-question might score low if Wikipedia's info looks stale relative to conversation context → triggers one refinement, forced to `GeneralWeb` to catch a more recent news source
- Synthesize: weather gets a small context slice (dense structured evidence, most of it is directly usable), mayor gets a larger slice (needed more material to resolve)
- Generate: answers both parts; if the mayor lookup still came back weak even after refinement, the model hedges specifically on that part ("as of the most recent information available...") while stating the weather plainly

**Example C — dedicated source misses, falls back cleanly**
Query: "current price of a fictional/obscure ticker that doesn't exist"
- Plan: 1 sub-question, hint = StockOrCrypto
- Retrieve: dedicated stock API returns empty/error
- Falls through to general web search per §2.2's fallback branch
- Judge: general web search likely also comes back thin/irrelevant → refinement broadens the query once
- If still weak: response should state uncertainty plainly rather than fabricating a price — this is exactly what the abstention prompting (from `grounding-faithfulness-plan.md` §4) is for

---

## 6. Testing Strategy

- **Unit-level:** test `judge_sufficiency`'s scoring function directly against hand-crafted evidence sets with known expected outcomes (clearly-strong, clearly-weak, borderline) to catch threshold regressions early
- **Integration-level:** replay a fixed set of real queries (simple factual, compound, dedicated-source-miss, ambiguous) through the full `answer()` loop in a test harness, asserting on: which source(s) were actually called, whether refinement fired when expected, and final response doesn't contain any leaked internal markers (sub-question IDs, confidence numbers, etc. accidentally reaching the output)
- **Latency regression:** track p50/p95 latency per query type (simple vs. compound) in CI or local benchmarking — since nothing is narrated to the user, latency regressions are invisible until someone notices the app "feels slower," so this needs to be measured proactively rather than caught by user complaints
- **Silent-failure audit:** specifically test that when every retrieval path fails (dedicated source down, general web search down), the system still produces a coherent response (falls back to the model's own knowledge with an honest caveat) rather than an error or hang — since there's no UI path to show a retrieval error to the user in silent mode, backend must handle total failure gracefully by itself

---

## 7. Tuning Notes

- Scoring weights (`W_RELEVANCE`, `W_AGREEMENT`, `W_COVERAGE`) and thresholds (`REFINEMENT_THRESHOLD`, `WEAK_THRESHOLD`, `CLAIM_SUPPORT_THRESHOLD`) are starting points, not final values — instrument them via local logging and adjust from real usage patterns rather than guessing correctly upfront
- Keep refinement capped at exactly one retry per sub-question — no exceptions, no recursive refinement chains
- Because nothing is narrated, **latency is the main cost this design trades against quality** — watch p95 response time closely once this ships; if compound questions start feeling sluggish, the first lever to pull is tightening `DEDICATED_SOURCE_TIMEOUT` and the decomposition heuristic (§2.1) before touching the judge/refine logic itself
