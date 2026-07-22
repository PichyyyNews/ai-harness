# AI Harness — Master Plan

Consolidated build plan covering everything designed so far: app architecture, GPU/engine handling, context & generation quality, chat UX, session persistence, and the web-grounding intelligence layer. This is the single reference document — each section links to its detailed companion file for implementation-level specifics.

---

## 0. Product Summary

A cross-platform (macOS/Windows/Linux) desktop app, built with **Tauri + React/Vite**, that lets users run open-source LLMs locally with **one installer, zero external dependencies**. Users pick a model from an in-app picker, the app downloads it, and chat runs entirely on-device — with an optional silent web-grounding layer for factual questions. UI reuses the existing `pichyy-next-ui-component` library, styled as a flowing-text conversational interface (not chat bubbles).

**Priority context:** this is the first of three planned projects (see original prioritization) — chosen because it has the smallest scope, fastest feedback loop, and builds the audience that later projects (dev social platform) will need.

---

## 1. Core App & Engine

**Companion file:** `overall-plan.md`

- **Shell:** Tauri (Rust backend + native webview), React + Vite frontend (not Next.js)
- **UI components:** ported from `pichyy-next-ui-component` (skip Next.js-specific parts, keep design tokens + component set)
- **Inference:** `llama-cpp-v3` Rust crate — runtime backend switching (CPU/CUDA/Vulkan/SYCL), auto-downloads matching pre-built binaries, no manual per-platform binary bundling needed
- **Model format:** GGUF, downloaded from Hugging Face at runtime (not bundled in installer)
- **Flow:** install → first-launch model picker → download selected GGUF → in-process inference via `llama-cpp-v3` → stream tokens to frontend via Tauri events (no local HTTP server needed)

**Build order (from this file):**
1. Tauri shell + ported UI components, static screens
2. Wire `llama-cpp-v3`, hardcoded model path, CPU backend, inference working end-to-end
3. Download manager + model picker UI
4. GPU backend switching + hardware detection
5. Polish: progress states, error handling, GPU offload slider

---

## 2. GPU Acceleration & Reliability

**Companion files:** `gpu-acceleration-plan.md`, `gpu-fallback-system.md`

- Backend mapping: Metal (Mac) / CUDA → Vulkan (Windows+Nvidia) / Vulkan (AMD/Intel) / CPU (universal fallback)
- GPU offload controlled via layer-offload setting, exposed as a user-adjustable slider later
- **Production-grade fallback system** (this is the piece directly addressing the "incomplete CUDA runtime" bug hit during dev):
  - **Priority chain per platform**, tried top to bottom until one verifies successfully
  - **Verification before trust**: checksum + file completeness check + a bounded-time dry-run load, not just "the file exists"
  - **Retry-once-then-fallback**: corrupted/incomplete downloads get one re-download attempt before the system moves to the next tier
  - **Cache the known-good backend** per hardware fingerprint + app version, so this detection doesn't re-run every launch — only invalidates on app update, driver change, or explicit user "re-detect"
  - **User override** always available in settings ("Force CPU mode")
  - Structured logging of every tier attempt/outcome for debugging failure patterns across users
  - Version-pin backend releases rather than always pulling "latest"

**Build order:** CPU backend first → GPU backend switching → hardware detection defaults → GPU offload UI slider (last, cosmetic layer on top of the working fallback system).

---

## 3. Context Management & Generation Quality

**Companion files:** `context-length-handling.md`, `generation-quality-repetition-guard.md`

Both of these run **entirely automatically and invisibly** — no token counters, no "Continue" buttons, no context-full warnings shown to the user.

### Context window handling
- Backend-only token budget tracking using the model's real tokenizer
- Silent sliding window: drop oldest non-system messages before hitting the limit
- Background auto-summarization: compact dropped history into a persistent "conversation memory" block instead of just discarding it, run opportunistically when idle
- Dynamic `n_predict` sizing based on remaining budget after truncation
- Silent auto-continue on `finish_reason: length` — up to a capped number of continuations, each re-checked against the budget

### Repetition/degeneration guard (the emoji-loop bug)
- **Root causes addressed:** weak sampling defaults, chat template/stop-token mismatches, and (critically) the auto-continue system above blindly extending an already-looping response
- **Sampling defaults:** `repeat_penalty` ~1.05–1.1, DRY sampling enabled (`dry_multiplier` ~0.8), `min_p` 0.05–0.1, tuned sampler order — shipped as app defaults, not hidden in advanced settings
- **Chat template correctness:** verified via the same dry-run load step used in the GPU fallback system — confirm a model actually terminates generation naturally before trusting it
- **Real-time circuit breaker:** watches the token stream live for exact-token or short n-gram repetition; on detection, aborts immediately, trims the repeated tail, tags `finish_reason: repetition_detected`
- **Critical integration:** auto-continue (above) must check this new `finish_reason` value and **never** continue a repetition-flagged response — this is what prevents the two systems from compounding into an unbounded loop like the screenshot

**Shared data structure:** both modules share one `FinishReason` enum (`Stop` / `Length` / `RepetitionDetected`) as the single source of truth for continuation decisions.

**Build order:** sampling defaults + chat template verification first (prevention) → repetition circuit breaker (detection) → wire the auto-continue guard last, since it depends on the `RepetitionDetected` tag existing first.

---

## 4. Chat UI/UX

**Companion file:** `chat-ui-ux-plan.md`

- **Flowing text layout** — no chat bubble cards; speaker labels + whitespace instead of colored containers, reads more like a shared document than SMS
- **Floating scroll-to-latest button** — "sticky scroll" pattern: auto-follows new tokens only if the user is already at the bottom; if they've scrolled up, auto-scroll stops and the button appears instead of yanking their view
- **Collapsible "thinking" summary** — collapsed, faded by default; requires capturing reasoning separately from the final answer and generating a short summary of it
- **Code block copy button** — hover-revealed, per-block, with syntax highlighting and a brief "Copied!" confirmation
- **Live generation status** — *only applies to local UI feedback for things the user should see (e.g. "writing response")* — explicitly does **not** apply to the web-grounding orchestrator (§5), which runs fully silently per the latest revision

**Build order:** flowing text layout first (foundation) → scroll button + copy button (self-contained) → live status wiring (where applicable) → collapsible thinking summary (most involved, needs reasoning-capture prompting work).

---

## 5. Chat Session Management (Local Persistence)

**Companion file:** `chat-session-management-plan.md`

- **Storage:** SQLite (`harness.db`) in the Tauri app data directory — same directory as downloaded models
- **Schema:** `sessions` (id, title, timestamps, model_id, conversation_memory) + `messages` (id, session_id, role, content, thinking_summary/full, finish_reason, sequence)
- **Sidebar:** Recents list sorted by recency, "New chat" button, search, per-item rename/delete — matches the reference screenshot pattern
- **Auto-generated titles:** short background model call after the first response summarizes the conversation into a title (same "opportunistic background call" pattern as conversation-memory compaction)
- **Session restore:** loading a session restores its `conversation_memory` into the context manager (§3) so background summarization continues correctly, and checks the session's `model_id` against the currently loaded model

**Build order:** SQLite schema + CRUD commands → sidebar list/new-chat/load → auto-title generation → search/rename/delete → full context-manager restore integration.

---

## 6. Web-Grounding Intelligence Layer (Silent Mode)

**Companion files:** `grounding-faithfulness-plan.md`, `source-expansion-plan.md`, `adaptive-retrieval-orchestrator.md`

This is the most substantial addition — it upgrades the existing search-first retrieval pipeline (already implemented per the project handoff docs) into an adaptive, self-checking system. **Runs entirely silently — no status line, no visible stage narration, no thinking-summary exposure for any of this.** The user just gets a noticeably better answer.

### 6a. Faithfulness (reduce hallucination in already-retrieved evidence)
- Post-generation claim-by-claim check against retrieved source text (cheap semantic similarity, not a full second LLM pass per claim)
- Flagged unsupported claims trigger a silent corrective regeneration before the response is shown as final
- Abstention prompting: the model is instructed to say when sources don't cover something, rather than filling gaps from parametric memory
- Cross-provider corroboration: agreement between the two existing search providers (searxng/duckduckgo) is a free confidence signal

### 6b. Source expansion (broader + more authoritative coverage)
- Adaptive retrieval depth: more pages/results for broad or comparative questions, fewer for simple lookups
- Query decomposition: compound questions get split into sub-questions, retrieved and evidenced separately, then merged
- **New dedicated source modules** (each a small file alongside `searxng.rs`/`duckduckgo.rs`): Wikipedia/Wikidata (definitional/biographical), weather API, currency/exchange rate API, stock/crypto price API, sports scores, news API, and dev-specific sources (npm/crates.io/PyPI registries, official docs)
- Source router decides which module(s) to call per query pattern, falling back to general web search when no dedicated source matches or returns thin results

### 6c. Adaptive Retrieval Orchestrator (ties 6a + 6b together)
The actual algorithm, silent end to end:

```
Plan → decompose query into sub-questions, tag each with a source hint
Retrieve → call dedicated source or general search per sub-question
Judge → score evidence sufficiency (relevance + cross-source agreement + coverage)
         low confidence → one silent re-retrieval with a refined query
Synthesize → merge evidence into context budget, weighted by what each sub-question needed
Generate → produce response with confidence-aware hedging built into the model's own wording
Verify → faithfulness check (6a) against merged evidence before finalizing
```

- Refinement is capped at one retry per sub-question — never an unbounded loop
- Internally logs every stage's decisions locally for debugging/tuning, but none of it reaches the chat UI
- Because nothing is narrated, response latency has to be watched closely in testing — a silent multi-second wait reads very differently than the same wait with a status line

**Exact codebase integration points** (new/modified Rust files): `orchestrator.rs` (new, owns the loop), `query.rs` (modified, emits structured `QueryPlan`), `source_router.rs` (new), per-source modules (new), `manager.rs` (modified, becomes per-sub-question fetch+rank), `bm25.rs` (modified, adds semantic reranking stage), `context_manager.rs` (modified, accepts per-sub-question evidence), `faithfulness.rs` (new, post-generation verification).

**Build order (priority within this section):**
1. Faithfulness post-generation check (6a) — works with existing retrieval as-is
2. Abstention prompting (6a) — cheap, pairs with the above
3. Cross-provider corroboration (6a) — near-zero extra cost
4. Wikipedia integration (6b) — broadest win, simplest dedicated source
5. Weather/currency APIs (6b) — narrow, well-defined
6. Semantic reranking on top of BM25 (6b)
7. Query decomposition + adaptive re-retrieval (6c orchestrator) — the bigger structural piece, builds on all of the above
8. Remaining dedicated sources (news, sports, package registries) — opportunistic, once the source-router pattern exists

---

## Full File Index

| File | Covers |
|---|---|
| `overall-plan.md` | Core app architecture, tech stack, UI component reuse |
| `gpu-acceleration-plan.md` | GPU backend approach (`llama-cpp-v3`), offload control |
| `gpu-fallback-system.md` | Production-grade backend detection/verification/fallback |
| `context-length-handling.md` | Automatic, silent context window & response-length management |
| `generation-quality-repetition-guard.md` | Repetition/loop detection, sampling defaults, auto-continue guard |
| `chat-ui-ux-plan.md` | Flowing text layout, scroll button, thinking summary, copy button |
| `chat-session-management-plan.md` | SQLite persistence, sidebar sessions, auto-titling |
| `grounding-faithfulness-plan.md` | Post-generation fact verification, abstention, corroboration |
| `source-expansion-plan.md` | Broader retrieval depth, dedicated real-time/Wikipedia sources |
| `adaptive-retrieval-orchestrator.md` | The silent plan→retrieve→judge→synthesize→generate→verify loop |
| `master-plan.md` | This file — consolidated overview and build order |

---

## Suggested Overall Build Sequence (across everything)

1. **Foundation:** Core app shell + engine (§1) working end-to-end on CPU, one hardcoded model
2. **Reliability:** GPU fallback system (§2) — get this right early since everything else depends on the engine actually starting reliably
3. **Quality baseline:** Sampling defaults + repetition guard (§3) — fix generation quality before building more features on top of a potentially-looping engine
4. **Usability:** Chat UI (§4) + session persistence (§5) — makes the app actually usable day-to-day
5. **Automatic context handling (§3 continued):** sliding window, summarization, auto-continue — layer this in once the base chat experience is solid
6. **Intelligence layer (§6):** faithfulness → source expansion → full orchestrator — this is the most complex piece and benefits from everything above already being stable
