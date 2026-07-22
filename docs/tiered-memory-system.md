# Tiered Memory System — Short / Mid / Long-Term (Flagship Feature)

Three-layer memory architecture so the app behaves like it genuinely knows the user and the work — not just "remembers the last N messages." Runs entirely silently (per the orchestrator's silent-mode precedent) — no UI exposure of internal memory state during chat, except one optional settings panel for transparency/trust (§7). This is designed as the app's core differentiator, so the long-term layer gets the deepest treatment.

---

## Why Layered (not one flat memory blob)

A single sliding-window/summarization scheme (already planned in `context-length-handling.md`) has a critical flaw for this use case: **an important rule stated early in a long conversation can get silently dropped or diluted once it's compacted away with everything else.** If a user says "don't use semicolons in code you write me" on message 3 of a 200-message session, that instruction must survive to message 200 — it can't be treated the same as ordinary chat content that's fine to summarize away. Splitting memory into three purpose-built layers, each with different persistence rules and injection guarantees, solves this directly.

| Layer | Scope | Survives sliding-window truncation? | Survives session end? |
|---|---|---|---|
| **Short-term** | Current turn/task, active rules & constraints | Yes — never truncated, injected every turn | No — session-scoped |
| **Mid-term** | Goals, plans, project structure for this session | Yes — this *is* what compaction produces | No (by default) — but see §6 for promotion to long-term |
| **Long-term** | Who the user is, across all sessions | N/A (not part of the sliding window at all) | Yes — persists indefinitely, consolidated over time |

---

## Layer 1 — Short-Term Memory (Active Rules & Immediate Context)

### What it holds
- Explicit rules/constraints the user has stated ("don't do X", "always respond in Thai", "for this project, never suggest Y")
- Immediate task focus (what the current exchange is actually about, right now)
- Urgent one-off instructions ("for the next answer only, be extremely brief")

### Data model
```rust
struct ActiveConstraint {
    id: Uuid,
    session_id: Uuid,
    text: String,                  // normalized instruction, e.g. "avoid semicolons in generated code"
    scope: ConstraintScope,        // Session | ThisTurnOnly
    created_at: DateTime,
    superseded_by: Option<Uuid>,   // if a later message contradicts/updates this one
}

enum ConstraintScope {
    Session,       // "from now on", "always", "for this project" — persists whole session
    ThisTurnOnly,  // "just this once", "for this answer" — auto-expires after one generation
}
```

### Extraction (runs after every user turn, cheap)
- Lightweight pattern/heuristic classifier first (imperative phrasing: "don't", "always", "never", "from now on", "for this project") — cheap, no model call needed for the common case
- Escalate to a small model-based classification only when the heuristic is ambiguous, similar cost-gating approach as the orchestrator's query decomposition
- On detecting a new constraint that **contradicts** an existing active one, mark the old one `superseded_by` the new — never silently keep two conflicting rules both active

### Injection guarantee
- All `Session`-scoped active constraints for the current session are **always** included in the system prompt, in a **fixed, reserved token budget** separate from the conversation-history/evidence budget in `context_manager.rs` — this budget is small (constraints are short) but non-negotiable; it is never trimmed by the sliding window or context-pressure logic
- `ThisTurnOnly` constraints are injected once, then discarded immediately after that generation completes

### Integration point
`src-tauri/src/engine/memory/short_term.rs` (new) — sits alongside `context_manager.rs`, exposes `active_constraints(session_id) -> Vec<ActiveConstraint>` that `context_manager.rs` calls when composing every prompt, before it does its normal history-budget allocation.

---

## Layer 2 — Mid-Term Memory (Goals, Plans, Project Structure)

### What it holds
Structured (not prose-blob) tracking of what the current session is actually trying to accomplish:
```rust
struct SessionMemory {
    session_id: Uuid,
    goals: Vec<Goal>,
    decisions: Vec<Decision>,
    open_questions: Vec<String>,
    plan_steps: Vec<PlanStep>,
}

struct Goal {
    description: String,
    status: GoalStatus,           // Active | Achieved | Abandoned
}

struct Decision {
    what: String,                 // "using SQLite for local storage"
    why: Option<String>,          // "no server process needed, good Rust support"
    turn_ref: usize,              // which message this came from, for traceability
}

struct PlanStep {
    description: String,
    status: StepStatus,           // Pending | InProgress | Done
}
```

This upgrades the flat `conversation_memory: TEXT` field already in the `sessions` table (per `chat-session-management-plan.md`) into structured JSON — still one column, but the app can now reason over it (e.g. "3 plan steps done, 2 pending") instead of just re-reading prose.

### Extraction
- Runs as a background/opportunistic compaction step — same trigger point already planned for conversation-memory summarization (idle time between messages, or right before the sliding window would need to drop old turns)
- The compaction call is prompted to extract structure, not just summarize prose: "what goals, decisions, and plan steps are evident from this conversation so far" — output parsed into the `SessionMemory` struct above
- Updates are **merges, not overwrites** — new extraction runs reconcile against the existing `SessionMemory` (mark completed goals as `Achieved`, append new decisions, don't duplicate ones already recorded)

### Injection
- A compact rendering of `SessionMemory` (a few lines, not the full structured JSON) goes into the system prompt every turn, in its own modest reserved budget — smaller than short-term's constraint budget priority-wise, but still guaranteed a slice rather than competing directly with raw conversation history for space
- This is what lets the model stay coherent about "we already decided X" or "you're 3 steps into the plan we made" even after the raw messages describing that have long since been dropped from the sliding window

### Integration point
`src-tauri/src/engine/memory/mid_term.rs` (new), replacing the current plain-text conversation-memory logic referenced in `context-length-handling.md` §3 — same trigger points, richer output structure.

---

## Layer 3 — Long-Term Memory (Cross-Session, Personalized) — Detailed

This is the flagship layer: the app should feel like it's building an actual understanding of the user over time, not starting cold every session.

### 3.1 What gets stored

```rust
struct LongTermFact {
    id: Uuid,
    category: FactCategory,
    content: String,                    // normalized, durable statement
    source_session_id: Uuid,
    confidence: f32,                    // how sure the extraction was
    created_at: DateTime,
    last_confirmed_at: DateTime,        // bumped when re-observed in a later session
    superseded_by: Option<Uuid>,        // points to a newer fact that replaced this one
}

enum FactCategory {
    Preference,          // "prefers concise answers", "likes dark mode UIs"
    CommunicationStyle,  // "writes in Thai, casual tone", "wants code comments in English"
    RecurringProject,    // "building 'AI Harness' desktop app, Tauri + Rust"
    RecurringTopic,      // "frequently asks about local LLM inference"
    SkillLevel,          // "comfortable with Rust, learning GPU programming"
}
```

**Durability test for what qualifies:** a candidate fact only gets stored if it would plausibly still matter in 3+ months — a one-off mention ("I'm debugging a weird error today") doesn't qualify; a stated preference, an ongoing named project, or a consistent communication pattern does. This filter is what keeps long-term memory from becoming a noisy dump of everything ever said.

**Explicitly excluded categories**, even though storage is fully local (no cloud, no account) — this is a deliberate product-trust decision, not a technical limitation:
- Health/medical information
- Political or religious views
- Relationship/family details beyond what's operationally relevant (e.g. "building an app with a friend" is fine; personal details about that friend are not)
- Anything the user explicitly asks not to be remembered

### 3.2 Extraction pipeline

- Runs **silently, in the background**, triggered at natural session-end points (app closed, session switched, or a long idle gap) — never mid-conversation, so it doesn't compete for latency with the live chat
- Two-pass approach:
  1. **Candidate extraction**: a lightweight pass over the session's `SessionMemory` (mid-term, already structured) plus a scan of the raw messages, proposing candidate `LongTermFact` entries
  2. **Durability + dedup filter**: each candidate is checked against the durability test above, and against existing `LongTermFact` rows for the same category — if a near-duplicate exists, bump `last_confirmed_at` on the existing one instead of creating a new row; if it contradicts an existing fact (e.g. a stated preference changed), mark the old one `superseded_by` the new rather than deleting history outright
- This mirrors the same "cheap heuristic first, escalate only when needed" cost discipline used elsewhere (orchestrator's decomposition, short-term's constraint detection) — most sessions produce zero or a handful of new durable facts, not a flood

### 3.3 Retrieval — the part that has to be smart, not just "dump everything"

At the start of a new session (and optionally, for very long sessions, periodically mid-session), retrieve only the **relevant slice** of long-term memory — never the whole profile:

```
fn retrieve_relevant_long_term(new_session_first_message, user_id) -> Vec<LongTermFact> {
    candidates = all_active_facts_for(user_id)     // excludes superseded ones
    scored = candidates.map(|fact| (fact, semantic_similarity(fact.content, new_session_first_message)))
    // Always include CommunicationStyle facts regardless of topic similarity — tone/style
    // preferences apply to every conversation, not just topically-related ones
    always_include = candidates.filter(|f| f.category == CommunicationStyle)
    topically_relevant = scored.filter(|(_, score)| score > RELEVANCE_THRESHOLD).top_k(K)
    return dedup(always_include + topically_relevant)
}
```

- `K` should be small (e.g. 5-8 facts) — the goal is targeted personalization, not maximum recall. Flooding the prompt with every known fact about the user both wastes context budget and risks the model over-fitting responses around irrelevant personal details ("bringing up your dog" when the user asked a coding question)
- This selective-retrieval principle is the difference between "feels like it knows me" and "feels like it's reciting a dossier at me" — the latter reads as creepy/performative rather than smart

### 3.4 Cross-session topic recall ("what did we discuss about X before")

- Alongside atomic `LongTermFact` rows, also persist a **compact per-session summary** (a few sentences, generated at the same session-end extraction point) in a `session_summaries` table, separate from the full message history already in `sessions`/`messages`
- When the current conversation references something that sounds like a past topic ("the game project I mentioned before", "like we discussed"), do a semantic search over `session_summaries` (not the full message archive — too expensive/noisy) to surface the 1-2 most relevant past sessions, and only then optionally pull specific messages from that session's full history if genuinely needed
- This is what lets the app say something coherent about a project mentioned three weeks and many sessions ago, without holding all of that in every prompt by default

### 3.5 Consolidation & decay (keeping this from growing unbounded)

- Periodic (e.g. weekly, or after N new facts accumulate) background consolidation pass:
  - Merge near-duplicate facts within the same category into a single, better-phrased entry
  - When many small related facts accumulate (e.g. five separate observations that all point to "prefers minimal explanations"), collapse them into one higher-level fact and mark the originals `superseded_by` it — a form of hierarchical compaction, same underlying idea as mid-term's compaction but applied across sessions instead of within one
  - Facts not re-confirmed (`last_confirmed_at`) in a very long time (e.g. 6+ months) can be down-weighted in retrieval scoring rather than deleted outright — old-but-true facts shouldn't vanish, but shouldn't outrank recently-confirmed ones either

### 3.6 Personalization application
- `CommunicationStyle` facts feed directly into a system-prompt style directive (verbosity, formality, language preference) applied to every generation — silently, no "personalizing for you" announcement
- `SkillLevel` facts adjust technical depth defaults (e.g. don't over-explain Rust basics to a user whose long-term profile shows they're already comfortable with it)
- `RecurringProject`/`RecurringTopic` facts are what feed the source-router-style "oh, this is probably about the AI Harness project again" inference when a new session's first message is ambiguous

### Integration points
- `src-tauri/src/engine/memory/long_term.rs` (new) — extraction, consolidation, retrieval
- New SQLite tables: `long_term_facts`, `session_summaries` (both in the same `harness.db` used by `chat-session-management-plan.md`)
- Retrieval call happens once per new session load (hooks into the session-restore flow already planned in `chat-session-management-plan.md`), not per-turn — keep this cheap

---

## 4. Full Prompt Composition (how all three layers + orchestrator evidence combine)

```
System prompt assembly order (each with its own reserved token budget):
1. Long-term: relevant personalization facts (small, top-K)         [Layer 3]
2. Mid-term: current session's goal/plan summary                    [Layer 2]
3. Short-term: active constraints — non-negotiable, always included [Layer 1]
4. Recent conversation history (sliding window, per context-length-handling.md)
5. Retrieved web evidence, if the orchestrator ran for this turn (per adaptive-retrieval-orchestrator.md)
```

Reserved-budget ordering matters: constraints (Layer 1) and long-term communication style are cheap and non-negotiable, so they're allocated first and never trimmed. Conversation history and web evidence are what flexes to fill whatever budget remains.

---

## 5. Data Schema Summary (SQLite additions)

```sql
CREATE TABLE active_constraints (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    scope TEXT NOT NULL,                 -- 'session' | 'turn_only'
    created_at TEXT NOT NULL,
    superseded_by TEXT
);

-- sessions.conversation_memory upgraded from TEXT prose to structured JSON (goals/decisions/plan_steps)
-- no new table needed, just a schema/format change to the existing column

CREATE TABLE long_term_facts (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    content TEXT NOT NULL,
    source_session_id TEXT REFERENCES sessions(id),
    confidence REAL NOT NULL,
    created_at TEXT NOT NULL,
    last_confirmed_at TEXT NOT NULL,
    superseded_by TEXT
);

CREATE TABLE session_summaries (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    summary TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

---

## 6. Mid-Term → Long-Term Promotion (edge case worth handling)

A single long session can itself contain durable, cross-session-worthy information (e.g. a multi-hour project-planning conversation). Rather than waiting strictly for "session end," the long-term extraction pass (§3.2) can also trigger opportunistically within a very long session once its mid-term `SessionMemory` has accumulated enough stable, unchanging goals/decisions — treating "this goal hasn't changed across the last N compaction cycles" as itself a durability signal worth promoting early.

---

## 7. Trust & Transparency (recommended, optional UI)

Even though this is designed to run silently during chat (consistent with the orchestrator's silent-mode precedent), a **fully invisible, unreviewable personal memory store is a different trust category than silent search retrieval** — users should be able to see and correct what's remembered about them, even if it's never surfaced proactively mid-conversation:
- A settings panel ("Memory") listing current `LongTermFact` entries in plain language, grouped by category
- Ability to delete individual facts or clear everything
- This is the same design tension every long-term-memory product faces — recommended as a v2 feature once the core three-layer system (above) is working, not a blocker for the initial build

---

## 8. Build Order

1. **Short-term (Layer 1)** — smallest, most self-contained, immediately fixes the "rule stated early gets forgotten" failure mode
2. **Mid-term structured upgrade (Layer 2)** — builds directly on the conversation-memory compaction already planned in `context-length-handling.md`, just restructures its output
3. **Long-term extraction + storage (Layer 3, §3.1-3.2)** — get facts being captured and stored correctly before worrying about retrieval sophistication
4. **Long-term retrieval (§3.3)** — relevance-scored, capped `top_k` injection into new sessions
5. **Cross-session topic recall (§3.4)** — session summaries + semantic search over them
6. **Consolidation/decay (§3.5)** — once there's enough real data volume to need it
7. **Trust/transparency settings panel (§7)** — recommended before any public release, given the personal nature of what's being stored

## 9. Testing Notes
- Seed a test fixture with a deliberately long, multi-topic fake session and verify: constraints stated early survive to the end (Layer 1), the goal/plan structure correctly reflects what was actually decided (Layer 2), and running extraction on it produces sensible, non-duplicated `LongTermFact` candidates that pass the durability filter (Layer 3)
- Test the contradiction/supersession path explicitly: state a preference, later state the opposite, confirm only the newer one is active and retrievable
- Test retrieval selectivity: confirm an unrelated new session's first message does *not* pull in topically-irrelevant long-term facts, only `CommunicationStyle` (always-included) and genuinely relevant ones
