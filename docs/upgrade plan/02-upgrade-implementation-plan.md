# Upgrade Implementation Plan — Fixing Root Causes A-D

Concrete fixes for `01-root-cause-audit.md`, ordered by dependency (some fixes make later ones trivial or unnecessary). Every function touched must add structured logging per the requirement in §5 — this is not optional polish, it's what prevents the next debugging session from requiring another 2MB log dump to diagnose.

---

## Fix A — Force Native, Grammar-Constrained Tool Calling (highest priority)

**Goal:** the model's tool calls arrive as valid structured JSON every time, by construction — not "usually, with a 4-format recovery parser as backup."

### A1. Verify the model and server configuration first
- Confirm `llama-server` is launched with `--jinja`
- Confirm the loaded model has a tool-calling-capable chat template (Qwen2.5, Llama 3.1/3.3, Mistral-Nemo are known-good; verify whichever model is currently default)
- Run the isolated `curl` test from `agentic-tool-loop-architecture.md` §5 directly — confirm the raw HTTP response contains a proper `tool_calls` array in `message.tool_calls`, not text that has to be scraped out of `message.content`
- **If this test fails even in isolation, no amount of Rust-side parsing fixes it** — the fix is a model/config change, not more parser code

### A2. Use grammar-constrained decoding for the tool-call JSON itself
- llama.cpp supports GBNF grammars / JSON-schema-constrained sampling — when the model is in "must produce a tool call" state, constrain decoding so it is **structurally impossible** to produce malformed JSON (missing braces, truncated strings, wrong field names). This eliminates the need for "JSON Auto-Repair" entirely — you can't produce broken JSON if the grammar won't let you.
- This is the single change most directly responsible for fixing "tools stop working when context changes" — grammar constraints don't degrade with context length the way free-text pattern-following does.

### A3. Demote the text-tag parser to a rare, loudly-logged fallback — don't delete outright yet
- Keep `parse_text_tool_call`'s tag-unpacking logic only as a last-resort path for the (should become rare) case where structured output genuinely isn't available
- **Every time this fallback path fires, log it as a warning-level event** with the raw model output attached — this turns "tools silently stopped working" into "here's a dashboard of exactly which malformed outputs are still happening and how often," which is the visibility needed to know when this fallback can be removed entirely
- Track a metric: `tool_call_parse_method` (native vs fallback vs failed) per call — if fallback/failed rates aren't trending to near-zero after A1/A2, that's a signal the chosen model itself is the problem, not the parsing code

---

## Fix B — Re-Anchor Memory on Every Loop Iteration, Not Just Once

**Goal:** the short-term constraint "reminder" block stays immediately before the latest message on *every* hop of the agent loop, not just hop 1.

### B1. Move reminder injection inside the loop, not before it
Current (inferred from docs): reminder inserted once, before `run_agentic_loop` starts.
Required: reminder computed and inserted **fresh, at the end of the message list, on every iteration** right before calling `request_chat_completion()`:

```rust
async fn run_agentic_loop(mut state: AgentLoopState, tools: &[ToolDefinition]) -> AgentLoopOutcome {
    loop {
        // Build the ACTUAL request messages fresh each iteration:
        // base messages (system + memory primary + history + tool round-trips so far)
        // + a freshly-appended reminder block, always last, always right before the model call
        let request_messages = with_fresh_reminder(&state.messages, &state.session_id);

        let response = llama_server::chat_completion(&request_messages, tools, true).await;
        // ... rest of loop as before, but `state.messages` (the persisted/growing log)
        // stays separate from `request_messages` (what's actually sent this hop)
    }
}
```
The distinction matters: `state.messages` is the durable, growing conversation log (gets persisted to SQLite as-is); `request_messages` is what's actually sent to the model *this hop*, with the reminder freshly re-appended at the end every time. Don't conflate the two — persisting the repeated reminder block into permanent history would bloat storage for no benefit.

### B2. Test this explicitly
Add a test scenario: state a short-term constraint, then force a multi-hop tool-using turn (e.g. a question requiring 2-3 sequential tool calls), and confirm the constraint is still honored in the final answer — this is the exact scenario that was silently broken before.

---

## Fix C — Consolidate the System Prompt Into One Ordered, Auditable Assembly

### C1. Audit `tools/mod.rs`'s actual directive content first
Read what "System Core Directives Injection" actually contains before assuming it's a problem — confirm whether it overlaps or conflicts with the memory blocks or tool descriptions.

### C2. If overlap/sprawl is confirmed, consolidate into one function
Single system-prompt assembly function with an explicit, documented section order (matching `tiered-memory-system.md` §4's precedent): long-term facts → mid-term session goals → core tool-use directives → short-term constraints (always last, right before the user/tool content). One function, one place to look, one place to log what was actually assembled.

---

## Fix D — Replace Remaining Hardcoded Heuristics

### D1. `is_incomplete_text` → replace with `finish_reason` check
```rust
// Before: guessing from punctuation (English/Thai-only, fragile)
// After: the API already tells you definitively
let needs_continuation = matches!(response.finish_reason, FinishReason::Length);
```
This is strictly more reliable and works in every language, since it's based on the actual token-limit signal from the engine, not a guess about sentence-ending punctuation. Delete the punctuation list entirely — it's now dead code.

### D2. "Option Safety Fallback" → keep only as a rare last-resort, log every occurrence
Once A2 (grammar-constrained tool calls) is in place, `options` being empty on `ask_user_clarification` should become structurally near-impossible (it's a `required` field in the JSON schema). Keep the 4-category fallback as a defensive last resort only, but log a warning every time it actually fires — a nonzero rate after Fix A ships is itself a signal something's still wrong upstream.

---

## 5. Mandatory Structured Logging (applies across all fixes above)

Every function touched by this upgrade — and ideally every major function in `agent_loop.rs`, `tools/mod.rs`, `engine/memory/*.rs`, and `web_search/*.rs` — must emit structured log entries, not ad hoc `println!`/silent execution. Use the `tracing` crate (Rust-standard for this):

**Minimum required fields per log entry:**
- `session_id`, and `iteration` number where applicable (agent loop hops)
- Function/span name (via `#[tracing::instrument]` on each function — this alone gives entry/exit + duration for free)
- Outcome: success/failure, and on failure, the actual error, not just "failed"
- For tool calls specifically: `tool_name`, `parse_method` (native/fallback/failed — from Fix A3), `duration_ms`
- For memory assembly: which tiers contributed content, token counts per tier, whether the reminder block was successfully appended this hop (from Fix B)

**Practical setup:**
```rust
use tracing::{instrument, info, warn, error};

#[instrument(skip(state), fields(session_id = %state.session_id, iteration = state.iteration))]
async fn run_agentic_loop_iteration(state: &mut AgentLoopState) -> LoopStepResult {
    // tracing::instrument automatically logs entry, exit, duration, and any error return
    ...
}
```
- Write logs to a rotating local file (`tracing-appender` crate handles rotation) in the app data directory, not just stdout — so logs survive and are inspectable after the fact without needing to run the app attached to a terminal
- This directly solves the meta-problem from this session: diagnosing "why doesn't the app work" required uploading a 2MB raw dev-agent transcript and grep-ing for function bodies. With proper structured logging, the **app's own runtime logs** should be sufficient to answer "did the tool call parse natively or fall back?", "was the reminder block present this hop?", "did search actually get invoked?" — without needing to inspect source code at all.

---

## Rollout Order

1. **A1** (verify model/config) — do this literally first, five minutes, before writing any code; if it fails, everything else waits on a model/config decision
2. **A2 + A3** (grammar constraints + demoted fallback with logging) — the core reliability fix
3. **Logging (§5)** — add this alongside A, not after — you need visibility *while* validating A actually worked, not just afterward
4. **B1 + B2** (memory re-anchoring + test) — independent of A, can be done in parallel by a second work stream if available
5. **D1** (`finish_reason` swap) — trivial, do anytime, no dependencies
6. **C1 + C2** (prompt consolidation) — do after A/B are stable, since it's an audit-then-maybe-refactor step, lower urgency
7. **D2** (option fallback logging) — do last, since its expected near-zero rate is itself a verification signal that A worked
