# Amendment — Bug J Precise Fix (Corrects the Ineffective Prior Attempt)

Supersedes the fix description reported for Bug J. That change (adding `max_tokens: 4096` at the `agent_loop.rs` call site) has no effect, because `context_manager.rs`'s `dynamic_budget()` hard-clamps every request's response budget to `MAX_RESPONSE_TOKENS = 1_536` regardless of what's requested. This document is the actual fix, based on the real `context_manager.rs` source provided.

---

## Confirmed Root Cause (from source, not inference)

```rust
const MAX_RESPONSE_TOKENS: u32 = 1_536;
```

```rust
fn dynamic_budget(context_size: u32, requested_tokens: u32, has_web: bool, has_memory: bool) -> ContextBudget {
    ...
    let response_tokens = requested_tokens
        .clamp(MIN_RESPONSE_TOKENS, MAX_RESPONSE_TOKENS)   // <-- 4096 request becomes 1536, always
        .min(response_cap)
        .max(minimum_response);
    ...
}
```

No matter what `agent_loop.rs` passes as `requested_tokens`, this function ceilings it at 1,536. **The reported fix (requesting 4096 at the call site) cannot have changed real behavior**, since this clamp sits downstream of that call and overrides it unconditionally.

**Second, compounding issue**, further down in `prepare_request`:
```rust
let remaining = context.saturating_sub(budget.safety_margin + actual_prompt_tokens);
let max_tokens = remaining.clamp(MIN_RESPONSE_TOKENS, MAX_RESPONSE_TOKENS).min(budget.response_tokens);
```
Even the 1,536 ceiling is a *maximum*, not a guarantee — if `actual_prompt_tokens` is large (accumulated system directives, memory blocks, tool schemas, and critically **the prior reasoning/tool-call hops' messages sitting in `history`**), `remaining` can be squeezed down toward `MIN_RESPONSE_TOKENS = 256`. A ~60-80 word Thai greeting cut off is consistent with a budget in roughly that range — this matches the screenshot exactly.

**Third — the real structural question:** does `agent_loop.rs` even call `generate_with_recovery` (the function shown, which already has *correct* continuation logic using real `FinishReason::Length` checks, not heuristic punctuation-matching)? If the agent loop has its own separate, simpler request logic that doesn't route through this well-built function, that's the same "good subsystem became unreachable dead code after a refactor" pattern as Bugs G and I. **Verify this before anything else** — if `generate_with_recovery` isn't in the call path at all, none of the following fixes matter until it's wired back in.

---

## Fix Plan

### Step 0 — Confirm the call path (do this first, five minutes)
Grep the codebase for call sites of `generate_with_recovery`. Confirm `agent_loop.rs`'s final-answer-producing hop actually calls it. If it doesn't — that's the real Bug J, and the fix is re-wiring the call, not tuning constants. If it does call it, proceed to Step 1.

### Step 1 — Add a hop-type parameter that reaches `dynamic_budget()` itself
The budget function needs to know whether it's sizing a reasoning/tool hop or the final-answer hop — this can't live only in `agent_loop.rs`, since `dynamic_budget()` is where the clamp actually happens:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HopKind {
    Reasoning,
    FinalAnswer,
}

const REASONING_MAX_RESPONSE_TOKENS: u32 = 1_024;   // tool-call JSON / brief reasoning
const FINAL_ANSWER_MAX_RESPONSE_TOKENS: u32 = 4_096; // the answer the user actually reads

fn dynamic_budget(
    context_size: u32,
    requested_tokens: u32,
    has_web: bool,
    has_memory: bool,
    hop_kind: HopKind,                    // NEW
) -> ContextBudget {
    let response_ceiling = match hop_kind {
        HopKind::Reasoning => REASONING_MAX_RESPONSE_TOKENS,
        HopKind::FinalAnswer => FINAL_ANSWER_MAX_RESPONSE_TOKENS,
    };
    let safety_margin = (context_size.saturating_mul(SAFETY_MARGIN_PERCENT) / 100).max(128);
    let usable = context_size.saturating_sub(safety_margin);
    let minimum_response = (context_size / 10).clamp(MIN_RESPONSE_TOKENS, 640);
    let response_cap = (context_size.saturating_mul(32) / 100).clamp(minimum_response, response_ceiling);
    let response_tokens = requested_tokens
        .clamp(MIN_RESPONSE_TOKENS, response_ceiling)      // uses the hop-specific ceiling now
        .min(response_cap)
        .max(minimum_response);
    ...
}
```

Thread `hop_kind` through `generate_with_recovery` and `prepare_request` as a new parameter — every call site that currently calls these two functions needs to pass `HopKind::Reasoning` or `HopKind::FinalAnswer` explicitly. There is no reasonable default here; make it a required parameter (not `Option` with a fallback) so a forgotten call site fails to compile rather than silently defaulting to the wrong budget.

### Step 2 — Make sure `actual_prompt_tokens` doesn't include stale reasoning-hop content bloating the final hop's prompt
Separately from the response-token ceiling: if prior reasoning/tool-call round-trip messages (tool_calls + tool_results from earlier hops in the same turn) are staying in `history` and counted toward `actual_prompt_tokens` for the final-answer hop's budget calculation, that shrinks `remaining` unnecessarily. Confirm whether those intermediate tool round-trip messages are:
   (a) necessary for the final hop to see (usually yes — the model needs the tool results to answer), vs.
   (b) verbose in a way that could be condensed (e.g. a tool's raw JSON result could be summarized before being kept in context for the final hop, rather than kept verbatim)
   This is a secondary optimization — Step 1's fix (separating the response-token ceiling from the reasoning ceiling) is the primary fix and should resolve the reported symptom on its own; only pursue this if short answers are still getting squeezed after Step 1 ships.

### Step 3 — Structured logging for verification
```rust
tracing::info!(
    hop_kind = ?hop_kind,
    requested_tokens,
    response_ceiling,
    actual_prompt_tokens,
    final_max_tokens = max_tokens,
    "prepared chat request"
);
```
Log this on every call to `prepare_request` — confirms, per the logging requirement already established, exactly what ceiling and final `max_tokens` value was used for every hop of every turn, without needing to reproduce a bug manually to diagnose it again.

---

## Verification

- [ ] Confirmed `generate_with_recovery` is actually in the live call path from `agent_loop.rs` (Step 0)
- [ ] Log output for the exact greeting scenario from the screenshot shows `hop_kind = FinalAnswer` and `final_max_tokens` at or near 4096, not ~256-1536
- [ ] Reproduce the screenshot scenario directly (a greeting triggering 2 reasoning hops beforehand) — confirm the final answer no longer truncates
- [ ] Confirm reasoning/tool-call hops still get a smaller, appropriate budget (1024) — this isn't about giving everything unlimited tokens, only about giving the *final* hop its own correct, unshrunk allocation
- [ ] Re-run the existing `reserves_nonzero_memory_budget_under_context_pressure` and `memory_floor_is_never_removed` tests already in the file — confirm they still pass after `hop_kind` is threaded through (these tests will need updating to pass a `HopKind` argument, since the function signature changes)
- [ ] Add a new test: `final_answer_hop_gets_full_ceiling_regardless_of_reasoning_hops` — construct a `history` with 2 prior reasoning/tool-call round-trip messages, call `dynamic_budget` with `HopKind::FinalAnswer`, assert `response_tokens` is not reduced by their presence beyond what prompt-token accounting alone would require
