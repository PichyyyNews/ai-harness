# Verification Checklist — Is This Actually Fixed?

Go through this literally, item by item, after implementing `02-upgrade-implementation-plan.md`. Every item should be checkable from logs or a direct test — not "feels better." Check items off only when you have concrete evidence (a log line, a passing test, a reproduced scenario), not on general impression.

---

## A — Native Tool-Calling Reliability

- [ ] Isolated `curl` test against `llama-server --jinja` returns a proper `tool_calls` array in `message.tool_calls` (not text requiring parsing)
- [ ] Confirmed current default model has a genuine tool-calling chat template (Qwen2.5 / Llama 3.1+3.3 / Mistral-Nemo, or verified equivalent)
- [ ] Grammar-constrained decoding is active for tool-call generation (not relying on the model's unconstrained free-text output)
- [ ] `tool_call_parse_method` metric logged for every tool call; over a 20-turn real test conversation, **native** parse method accounts for ≥95% of calls (fallback/failed near-zero)
- [ ] Deliberately push a conversation to near-full context window, then trigger a tool call — confirm it still parses natively (this is the exact scenario that broke before — long context degrading tool reliability)
- [ ] Every fallback-parser hit is logged at warning level with the raw offending model output attached — confirmed by triggering one deliberately (e.g. temporarily misconfigure the template) and checking the log

## B — Memory Recency in Multi-Hop Loops

- [ ] State a short-term constraint ("don't use emoji", or equivalent), then trigger a question requiring 2-3 sequential tool calls — confirm the constraint is still honored in the final answer
- [ ] Log confirms the reminder block is freshly appended (present, non-empty) on **every** loop iteration, not just iteration 1 — checked directly from log output across a multi-hop test
- [ ] Confirm `state.messages` (persisted history) does NOT contain duplicated reminder-block spam from every hop — only `request_messages` (what's sent per-hop) should carry the repeated reminder, not what's saved to SQLite
- [ ] Mid-term session goals still influence behavior correctly after a multi-hop tool-using turn (test: reference "the plan we made earlier" after a turn involving 2+ tool calls)

## C — System Prompt Consolidation

- [ ] `tools/mod.rs`'s actual directive content has been read and confirmed either (a) non-overlapping with memory blocks — no action needed, or (b) overlapping/sprawling — consolidated into one ordered assembly function
- [ ] If consolidated: one function is the single place that assembles the full system prompt; log output shows each section (long-term / mid-term / directives / short-term) and its token count per request

## D — Residual Heuristics Replaced

- [ ] `is_incomplete_text`'s punctuation-list check is deleted; auto-continuation now triggers strictly from `finish_reason == Length`
- [ ] Auto-continuation tested and confirmed still works correctly (doesn't over-trigger on complete answers, doesn't under-trigger on truncated ones) across at least 2 different languages
- [ ] "Option Safety Fallback" (4 hardcoded categories) still exists as a defensive last resort, but its fire rate is logged and confirmed near-zero across real test usage after Fix A shipped

## E — Structured Logging Coverage

- [ ] `tracing` (or equivalent) is wired up and writing to a rotating local log file in the app data directory
- [ ] Every function in `agent_loop.rs` involved in the loop's core decision path is instrumented (entry/exit/duration/outcome visible in logs)
- [ ] Memory assembly (`engine/memory/*.rs`) logs which tiers contributed content and token counts, per request
- [ ] Tool dispatch (`tools/mod.rs`) logs tool name, arguments (sanitized if sensitive), duration, success/failure for every call
- [ ] **Meta-test:** without opening any source file, can you answer "did search_web get called on this turn, and did the reminder block make it into the prompt" purely by reading the app's own log file for that session? If yes, the logging requirement is actually met; if you still need to grep source code to answer that, it isn't.

## F — End-to-End Outcome Tests (ties back to what was originally asked for)

- [ ] Plain greeting → no tools called, direct answer, single hop
- [ ] Clear factual/current-info question → `search_web` called, result actually reflected in the final answer (not just retrieved and ignored)
- [ ] Genuinely ambiguous request → `ask_user_clarification` fires with real (non-fallback) options, loop suspends, resumes correctly after user's choice
- [ ] Multi-part question needing 2+ different tools (e.g. weather + a calculation) → both tools called in sequence within one turn, both results reflected in the final answer
- [ ] Same multi-tool scenario repeated after a long conversation history (context pressure) → still works — this is the specific regression that was reported as broken
- [ ] Cross-lingual test: repeat the ambiguous-request and constraint-adherence tests in a language other than Thai/English — confirm behavior holds (validates Fix D1 and the broader move away from hardcoded per-language heuristics)
- [ ] Full 20+ turn real conversation, mixing greetings, searches, clarifying questions, and multi-tool turns — confirm no point in the conversation where tools "stop working" or memory "disappears"

---

## If Something's Still Failing After All This

Go back to `01-root-cause-audit.md` and check whether the failure matches one of the four root causes exactly, or whether it's a **new** failure mode — if new, it needs its own root-cause analysis (using the same discipline: find the actual code path, don't guess) rather than being folded into these existing fixes. Resist the urge to add another hardcoded special case as a quick patch — that's the exact pattern that created Root Causes A and D in the first place.
