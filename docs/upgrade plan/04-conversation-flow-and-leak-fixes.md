# Amendment — Tool-Call Leak, Missing Enhancer, and Clarification Flow

New concrete evidence (screenshots) on top of `01-root-cause-audit.md` / `02-upgrade-implementation-plan.md` / `03-verification-checklist.md`. These are specific, fixable bugs — not new architecture, refinements to the same fixes already planned.

---

## New Bug F — Raw Tool-Call Syntax Leaking Into Displayed Message Content

**Evidence:** the screenshot shows `<|tool_call|>call:ask_user_clarification{options:[...], question:"..."}` rendered as a normal chat message, **directly above** a correctly-formed `InteractiveChoiceBox` with the right Thai options. Both appear — meaning the tag *was* successfully matched by the fallback text-parser (options/question extracted correctly, choice UI built correctly), but the raw matched span was never stripped from the `content` field that got persisted/streamed to the frontend as message text.

**This is a distinct bug from Root Cause A** (which was about *failing* to recognize tool-call syntax at all). Here recognition partially succeeded — the fix is narrower and more urgent, since it's the most visibly broken thing a user can see:

**Fix:** wherever `parse_text_tool_call` (or the native `tool_calls` path) successfully matches a tool call out of a text response, the matched span **must be excised from `content`** before that message is saved to `messages.content` or streamed to the frontend. Concretely:
```rust
if let Some((tool_call, matched_span)) = try_parse_tool_call(&raw_text) {
    let visible_content = raw_text.replace(matched_span, "").trim().to_string();
    // visible_content is what gets streamed/persisted as this turn's user-facing text —
    // it should typically be EMPTY when the whole response was a tool call, not a
    // partial leftover fragment of the tag syntax
    ...
}
```
**Test directly:** assert that no message ever persisted to `messages.content` contains the substrings `<|tool_call|>`, `call:`, or `<function=` — run this as a standing invariant check across a real test conversation that deliberately triggers `ask_user_clarification`, not just a one-off manual check.

This bug should be fixed **before** the deeper Root Cause A fix (native grammar-constrained calling) ships, since it's independently visible and embarrassing even while A is being worked on — patch this one first as a quick, isolated win.

---

## New Bug G — `prompt_enhancer::enhance_prompt` Appears to Have Dropped Out of the Pipeline

**Symptom described:** phrasing/tone quality regressed ("แค่เปลี่ยนวิธีการพูด...ก็เอ๋อ") — consistent with a preprocessing step that used to normalize/clean input before it reached memory assembly and the loop no longer running.

**Likely cause:** during the agent-loop refactor (adding `agent_loop.rs`, the 15-tool catalog, etc.), the entry point for a chat turn may have changed, and `prompt_enhancer::enhance_prompt` — originally called from the old single-shot pipeline — may no longer be wired into the new agentic-loop entry path. This is a classic refactor regression: the function still exists and presumably still passes any unit tests it has in isolation, but nothing calls it anymore from the live request path.

**Fix:**
1. Grep the current codebase for actual call sites of `enhance_prompt` — confirm whether it's invoked from `commands/engine.rs`'s current `generate_chat` handler at all
2. If it's dead code (defined but never called from the live path), re-wire it back into Step 2 of the pipeline, before memory assembly (matching the original `01_system_architecture.md` ordering: enhancer → memory injection → loop)
3. Add a log line confirming this step ran for every turn (per the logging requirement in `02-upgrade-implementation-plan.md` §5) — so a future regression like this is visible in logs immediately instead of only showing up as "the AI feels dumber" weeks later

---

## New Bug H — Clarification Tool Over-Triggers on Greetings, Doesn't Chain, Skips Search After Resolving

Three related symptoms from the same root issue: **the tool description and system directives for `ask_user_clarification` don't distinguish "genuinely ambiguous request" from "any short message," and nothing nudges the model toward multi-round narrowing or grounding after clarification resolves.**

### H1 — Fires on plain greetings (should never happen)
**Fix — rewrite the tool description** with explicit negative and positive guidance, since the description is the actual mechanism controlling when the model calls it (per `agentic-tool-loop-architecture.md` §4):
```
"ask_user_clarification": "Ask the user a clarifying question with 2-4 options
ONLY when they have stated a request that is genuinely too broad or ambiguous
to act on usefully. Do NOT call this for greetings, acknowledgements, or
simple conversational openers — respond to those directly and wait for the
user's actual request. Example of when to use: user says 'อยากรู้เรื่องเทคโนโลยี'
(broad topic, needs narrowing). Example of when NOT to use: user says 'สวัสดี'
(a greeting — just greet back)."
```
Concrete examples inside the tool description (both positive and negative) are doing real work here — this is prompt engineering, test it against exactly the failing case from the screenshot (greeting → should get a plain greeting back, zero tool calls).

### H2 — Only ever asks once, doesn't narrow progressively
**Desired behavior** (as described): topic → clarify to subtopic → clarify to specific angle → then answer/search. The loop architecture (`agentic-tool-loop-architecture.md` §2, suspend/resume) already supports multiple clarification rounds structurally — nothing in the loop design caps clarification to one round. If it's stopping after one round in practice, the cause is a **decision-quality problem, not a structural cap**: the model, after receiving the first answer, is choosing to answer/write immediately rather than judging whether a second narrowing round would help.

**Fix — add an explicit system directive** (goes in the consolidated system prompt from `02-upgrade-implementation-plan.md` Fix C):
```
"When a user's request is broad, you may ask MULTIPLE rounds of clarifying
questions in sequence — narrow from general topic, to subtopic, to specific
angle — before producing a full answer. Prefer narrowing over guessing when
a request could reasonably go in several different directions."
```
**Test:** a genuinely broad opener ("อยากรู้เรื่องเทคโนโลยี") should be able to produce 2 sequential clarification rounds in a scripted test before the loop proceeds to a final answer — confirm this actually happens, not just that the directive text exists.

### H3 — Skips search, writes a long answer purely from parametric memory
**Evidence:** the Crusades example — after (a broken/leaky) clarification exchange, the assistant produces a long, structured historical analysis with zero search calls, zero citations — pure model-knowledge output on a topic that's exactly the kind of thing `search_web` and the grounding pipeline (`05_web_search_rag_pipeline.md`) exist for.

**Fix — add a directive nudging toward grounding for this class of question**, consistent with the abstention/grounding-preference framing from `grounding-faithfulness-plan.md`:
```
"For factual, historical, current-events, or analytical deep-dive questions,
prefer calling search_web to ground your answer in real sources before
writing a long response — especially once the user's specific interest has
been narrowed down via clarification. Don't default to writing from memory
alone when grounding tools are available and relevant."
```
**Test:** repeat the Crusades scenario after this directive ships — confirm `search_web` gets called (visible in logs per the logging requirement) before the long-form answer is produced, and that the answer includes source references.

---

## New Bug I — Auto-Continuation ("ต่อข้อความเมื่อ context เต็ม") Regressed

**Symptom:** previously working, now "หายไปเลย" (completely gone) — not degraded, gone. This is a strong signal of the same class of bug as G: **a feature that exists in code but is no longer reachable from the current live request path**, most likely because the auto-continuation logic (`is_incomplete_text` / the "Seamless Stitching" loop from `02_agentic_loop_and_execution.md`) was written against the *old* single-shot response path, and the *new* agentic-loop's final-answer emission (`LoopStepResult::FinalAnswer` in `agentic-tool-loop-architecture.md` §3) may bypass it entirely.

**Fix:**
1. Confirm directly: is the auto-continuation check (`is_incomplete_text`, or its replacement per `02-upgrade-implementation-plan.md` Fix D1's `finish_reason` swap) actually called on the path that produces `LoopStepResult::FinalAnswer` inside the new agent loop? Or only on a now-dead code path?
2. Re-wire it into the loop's final-answer handling specifically: right before returning `FinalAnswer(text)` to Step 6, check `finish_reason` — if `Length`, run the continuation sub-loop (same seamless-stitching approach as before, capped at 3 continuations per the original design) before actually returning
3. Add this as an explicit item to `03-verification-checklist.md`: force a response that hits the token limit deliberately (e.g. ask for a very long detailed output) and confirm continuation fires and stitches correctly — this exact scenario should have been caught by an existing test if one existed; if it wasn't caught, that's itself a signal to add a regression test for "does the currently-live pipeline path actually invoke every previously-working feature" going forward, not just testing each feature in isolation

---

## Why These Are All the Same Underlying Lesson

Bugs F, G, and I share a pattern: **features that were built and previously worked, silently stopped being invoked from the live path after the agent-loop refactor.** This is refactor regression, not a design flaw in any of them individually. The single most valuable process fix here — beyond patching each bug — is: **after any refactor that changes the main request-handling entry point, explicitly re-verify every previously-shipped feature is still wired into the new path**, rather than assuming "the function still exists, so it still works." The structured logging requirement from `02-upgrade-implementation-plan.md` §5 is what makes this checkable going forward — if the enhancer, the continuation logic, and the tool-call content-stripping all log their own execution, a missing log line for any of them on a real turn is an immediate, obvious signal rather than a slowly-noticed "feels off" regression.

## Updated Priority Order (merges with the existing plan)
1. **Bug F** (content leak) — fastest, most visible, fix immediately
2. **Bug G** (missing enhancer) — likely a one-line re-wire once located
3. **Bug I** (missing continuation) — likely a one-line re-wire once located, same class as G
4. **Root Cause A** (native tool-calling) — as originally planned, still the deepest fix
5. **Bug H** (clarification tool description + directives) — prompt-engineering pass, test-driven against the exact scenarios described here
6. Everything else per the original `02-upgrade-implementation-plan.md` rollout order
