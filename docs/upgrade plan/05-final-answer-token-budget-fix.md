# Amendment — Bug J: Final-Answer Token Budget Starved by Prior Reasoning Hops

New finding on top of `04-conversation-flow-and-leak-fixes.md`. Screenshot evidence: a plain greeting response ("สวัสดีครับ นิวส์...") gets cut off mid-word ("...หรือประ") after the UI shows **"Ran reasoning steps · 2 steps"** before the visible answer. A greeting-length response should never hit a token limit under normal budgeting — this points to a second, distinct bug sitting alongside Bug I (missing auto-continuation).

---

## The Bug

Auto-continuation (Bug I) explains *why a truncated response doesn't get completed*. It does not explain *why a ~40-word greeting response was truncated in the first place*. Two hops of reasoning/tool-call activity happened before the final answer — if the per-request token budget (`n_predict`/`max_tokens`) is being calculated once for the whole multi-hop turn rather than freshly for each hop, the final hop that actually produces the user-visible answer could be left with only a small fraction of the intended budget, having "spent" most of it accounting for the earlier reasoning/tool-call hops that never even reach the user.

**Likely location:** wherever the agent loop (`agent_loop.rs`) or `context_manager.rs` computes the token budget for `request_chat_completion()` — if this calculation subtracts prior hops' token usage from a single shared budget rather than giving the final answer-producing hop its own full, independent budget, this is exactly the failure mode in the screenshot.

---

## Fix

**Separate the token budget for reasoning/tool-call hops from the token budget for the final answer hop.** These are different kinds of output with different needs:
- Reasoning/tool-call hops: typically short (a tool name + arguments, or brief internal reasoning) — a small budget is fine and appropriate
- The final answer hop: needs its own full, independent `n_predict` allocation (e.g. the documented 4096 from `02_agentic_loop_and_execution.md`), **not** whatever remains after earlier hops consumed part of a shared pool

```rust
// Wrong (likely current): one shared budget decremented across hops
let remaining_budget = total_budget - tokens_used_so_far;
request_chat_completion(&messages, tools, max_tokens: remaining_budget)

// Right: each hop gets its own appropriately-sized budget
let hop_budget = if is_final_answer_hop {
    FINAL_ANSWER_MAX_TOKENS   // e.g. 4096, independent of prior hops
} else {
    REASONING_HOP_MAX_TOKENS  // e.g. 512 — tool calls/reasoning don't need much
};
request_chat_completion(&messages, tools, max_tokens: hop_budget)
```

**Note this is somewhat in tension with the overall context-window budget** (`context_manager.rs`'s dynamic allocation across memory/history/evidence, per `context-length-handling.md`) — that budget governs total *context* (input) size, which is a different constraint from *per-hop output* token limits. Don't conflate the two: a hop can legitimately have a small remaining context window available while still being entitled to a full output-token budget for whatever context space it does have. Verify these two budgets are tracked as genuinely separate numbers in the code, not accidentally sharing one variable.

---

## Verification

- Log `max_tokens` (or equivalent `n_predict`) actually sent per hop, tagged with `iteration` and `is_final_answer_hop` — per the logging requirement in `02-upgrade-implementation-plan.md` §5
- Reproduce the exact screenshot scenario (a greeting that triggers 2 reasoning/tool hops before the final answer) and confirm the final hop's logged `max_tokens` is the full intended value (e.g. 4096), not a small leftover fraction
- Add this as an explicit item in `03-verification-checklist.md`: **a short final answer preceded by multiple reasoning hops must never truncate** — test this specifically, since it's a different failure mode than a genuinely long answer hitting a real limit

## Relationship to Bug I (auto-continuation)

Both should be fixed, but fixing Bug J (the budget-starvation cause) directly addresses *this specific screenshot's* symptom — a short answer shouldn't need continuation at all once it has its own proper budget. Bug I (restoring auto-continuation) remains necessary independently for the case of a **genuinely long** answer that legitimately exceeds even a full, correctly-sized budget — that's a real, valid scenario continuation is still needed for, separate from this budget-starvation bug.
