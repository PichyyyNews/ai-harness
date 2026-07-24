# Agentic Tool-Loop Upgrade — Full Detailed Spec

Complete redesign of `pipeline_architecture.svg`'s single-shot router into a real multi-step agentic loop — the architecture Claude and ChatGPT actually run on. This document specifies the structure, the exact techniques used, and the concrete outcomes this upgrade must produce, matched directly against what was asked for.

---

## 0. Desired Outcomes (definition of done)

Every item below restates a specific requirement, so this upgrade has a concrete bar to be checked against rather than a vague "feels smarter":

| Requirement (as stated) | What "done" looks like |
|---|---|
| "มีวงจรในการ Loop เป็นของระบบ แบบที่ Claude ทำ" — a real loop, like Claude | The model can go: reason → call tool → see result → reason → call another tool → ... → final answer, all within one user turn, without returning control to a separate classifier at any step |
| "ถ้าต้องถามก็ไปใช้ช้อยมาถามผู้ใช้" — ask via choice UI when needed | Asking the user is a **tool the model calls on its own judgment**, not a pre-classification branch decided before the model even reasons about the request |
| "ต้องค้นหาก็ไปค้นหา ไปใช้ API หรืออื่นๆ" — search/use APIs as needed | Web search and every structured API (weather, currency, stocks, Wikipedia, etc.) are tools available to the model at every step of the loop, callable in any order, any combination |
| "ไม่ใช่การ hardcode hard prompt ดัก" — not keyword-triggered hardcoding | Zero `if message.contains("...")`-style logic anywhere in the decision path. The only thing deciding behavior is the model's own tool-call output, governed by tool descriptions, not string matching |
| "ให้ AI ตัดสินใจและใช้ tool ที่หลากหลาย" — AI decides, uses varied tools | Confirmed via test: the same underlying loop mechanism handles a plain greeting, a factual search question, a multi-part question, and an ambiguous request — no separate code path per case |
| "แบบใช้ tool ต่อกัน" — chain tools together | A single user turn can trigger 2+ sequential (or parallel) tool calls before the final answer, demonstrated with a real multi-step test case |
| "1 prompt ไม่จำเป็นต้อง 1 output ควรทำเป็น batch" — not 1:1 prompt→output | The loop supports N inference round-trips per user turn (bounded by a max-iteration cap), not a fixed one-shot request/response |
| "lib ไหนช่วยเรื่องนี้ได้ก็ดี" — library support welcome | A concrete library recommendation is given (§6), with a clear low-risk starting point and a higher-capability upgrade path |
| "แบบคิดเป็นระบบ...แบบ Claude AI ChatGPT" — thought of as a coherent system | The full request lifecycle (§2) is specified end to end, not just the loop in isolation — memory, retrieval, and the UI event contract all have a defined place in the new structure |

---

## 1. What's Wrong With the Current Structure (recap, for context)

`planner::decide() -> ToolRoute` is a **single classification call** producing one of `Answer | CallTool | AskUserChoice`, and whichever branch is chosen executes exactly once with no path back to reconsider. Two structural problems, not just a tuning problem:

1. **No loop.** After Branch B (`CallTool`) runs, there's no mechanism to decide "given this result, do I need another tool, or to ask something, before I can answer." It's a 1-hop dispatch, not an agentic loop.
2. **It's still a hardcoded classifier**, just phrased as a router function instead of a keyword list. `planner::decide()` pre-decides the *shape* of the whole turn before the model has even reasoned about the specific request in front of it. This is the same category of problem as the English-only keyword lists found earlier (`confirmed-bugs-and-fixes.md`) — a bespoke decision layer standing in for the model's own judgment.

The fix removes this decision layer entirely rather than improving it.

---

## 2. Target Architecture — Full Request Lifecycle

```
┌─────────────────────────────────────────────────────────────────────┐
│ 1. User Prompt Entry (unchanged)                                     │
│    Frontend sends raw message to generate_chat                      │
└───────────────────────────────┬───────────────────────────────────────┘
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 2. Background Enhancer (unchanged — still a fast pre-pass)          │
│    prompt_enhancer::enhance_prompt — light intent structuring only; │
│    NOT a routing decision, just cleanup/normalization if useful     │
└───────────────────────────────┬───────────────────────────────────────┘
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 3. Context & Memory Assembly (unchanged)                            │
│    short-term constraints + mid-term session memory + relevant      │
│    long-term facts + recent history, assembled once as the          │
│    STARTING message list the loop below begins from                │
└───────────────────────────────┬───────────────────────────────────────┘
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 4. AGENTIC TOOL LOOP  ◄─────────────────────────────────┐            │
│    (replaces the old Step 4 router + Steps 5A/5B/5C)     │            │
│                                                            │            │
│    a. Send messages + tool_definitions to llama-server    │            │
│       (--jinja, tools=[...], parallel_tool_calls=true)     │            │
│    b. Model responds: final_text  OR  tool_calls[]         │            │
│    c. IF final_text  → exit loop, go to Step 6              │            │
│    d. IF tool_calls[] → execute each (concurrently where   │            │
│       independent), append tool_result messages            │            │
│    e. iteration += 1; if iteration > MAX_HOPS → force a     │            │
│       final answer using whatever evidence exists so far     │            │
│    f. loop back to (a) ──────────────────────────────────────┘            │
└───────────────────────────────┬───────────────────────────────────────┘
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 5. Special case: ask_user_clarification tool called                 │
│    → emit ai-interaction-request (existing event, unchanged)        │
│    → loop SUSPENDS (not exits) awaiting the user's choice            │
│    → user's answer is appended as a tool_result and the loop        │
│      resumes from step 4a — this is a suspend/resume, not a         │
│      separate multi-turn "scoping loop" outside the main loop        │
└───────────────────────────────┬───────────────────────────────────────┘
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│ 6. Final Answer Generation & Streaming (unchanged)                  │
│    generate_with_recovery() -> EventStream, same as today            │
└─────────────────────────────────────────────────────────────────────┘
```

**What's removed:** the `ToolRoute` enum and `planner::decide()` as a pre-classification gate. **What's added:** the loop in Step 4, and a suspend/resume mechanism for the clarification tool instead of the old separate "Multi-Round Scoping Loop" side-path — clarification becomes one case handled *inside* the same loop, not a structurally different branch.

---

## 3. Core Data Structures

```rust
struct ToolDefinition {
    name: String,
    description: String,           // this is what the model reads to decide WHEN to call it —
                                    // treat this as prompt engineering, not documentation
    parameters_schema: serde_json::Value,  // JSON Schema
}

enum LoopStepResult {
    FinalAnswer(String),                        // model produced text, no tool calls
    ToolCalls(Vec<RequestedToolCall>),           // model wants 1+ tools executed
}

struct RequestedToolCall {
    id: String,             // model-assigned call id, needed to match results back correctly
    name: String,
    arguments: serde_json::Value,
}

struct ToolResult {
    call_id: String,        // must match RequestedToolCall.id
    content: String,        // result serialized for the model to read
    is_error: bool,
}

struct AgentLoopState {
    messages: Vec<ChatMessage>,     // grows every iteration: assistant tool_calls + tool_results appended
    iteration: u32,
    max_iterations: u32,            // e.g. 8
    session_id: Uuid,
}
```

```rust
async fn run_agentic_loop(mut state: AgentLoopState, tools: &[ToolDefinition]) -> String {
    loop {
        let response = llama_server::chat_completion(&state.messages, tools, parallel: true).await;

        match parse_response(response) {
            LoopStepResult::FinalAnswer(text) => {
                return text;   // exits the loop — Step 6 takes over
            }
            LoopStepResult::ToolCalls(calls) => {
                state.messages.push(ChatMessage::assistant_tool_calls(&calls));

                // execute independent calls concurrently; sequential only if one call's
                // arguments plausibly depend on another (rare — most parallel calls are independent)
                let results: Vec<ToolResult> = execute_tools_concurrently(&calls, &state.session_id).await;

                for result in &results {
                    state.messages.push(ChatMessage::tool_result(result));
                }

                state.iteration += 1;
                if state.iteration >= state.max_iterations {
                    // force closure: ask the model one more time WITHOUT tools available,
                    // so it must answer with whatever evidence has been gathered so far
                    return force_final_answer(&state).await;
                }
                // loop continues — back to top, model sees the new tool results
            }
        }
    }
}
```

**Why `force_final_answer` matters:** without a hard cap and a graceful exit, a model that keeps calling tools (e.g. stuck re-searching) could loop indefinitely. Capping iterations and then removing tool access on the final forced call guarantees termination while still producing a usable answer from whatever was gathered — this is the concrete mechanism preventing "smart loop" from becoming "runaway loop."

---

## 4. Full Tool Catalog

Every existing capability, plus the clarification tool, defined once as data — no branch-specific code path per capability in the dispatch logic itself, just a lookup by tool name at execution time:

```rust
fn tool_catalog() -> Vec<ToolDefinition> {
    vec![
        tool!("search_web", "Search the web for current or factual information not already known.",
            { "query": "string" }),

        tool!("get_weather", "Get current weather conditions for a location.",
            { "location": "string" }),

        tool!("get_currency_rate", "Get the current exchange rate between two currencies.",
            { "from": "string", "to": "string" }),

        tool!("get_stock_price", "Get the current price of a stock or crypto ticker.",
            { "ticker": "string" }),

        tool!("search_wikipedia", "Look up a definitional, biographical, or encyclopedic fact.",
            { "topic": "string" }),

        tool!("get_system_status", "Report current engine/model/hardware status.", {}),

        tool!("list_models", "List locally available or downloadable models.", {}),

        tool!("ask_user_clarification",
            "Ask the user a clarifying question with 2-4 selectable options, ONLY when the \
             request genuinely cannot be answered without this information.",
            { "question": "string", "options": "array<string>" }),
    ]
}
```

Each tool's `description` field is the actual mechanism controlling when the model uses it — this is prompt engineering, and should be iterated on based on real test failures (a tool called too eagerly or too rarely is a description problem first, before it's treated as a model-capability problem).

### Where existing subsystems plug in as tool implementations (not loop logic)
- `search_web` → internally runs the full plan→retrieve→judge→refine sequence from `adaptive-retrieval-orchestrator.md`. The outer loop never sees that complexity — it calls `search_web`, gets a result string back, done.
- `ask_user_clarification` → internally emits the existing `ai-interaction-request` event and suspends the loop (§2, Step 5) until a frontend response arrives.
- The rest (`get_weather`, `get_currency_rate`, etc.) → thin wrappers around the dedicated source providers already planned in `source-expansion-plan.md`.

---

## 5. Prerequisite: Model Tool-Calling Support

This entire redesign has a hard dependency: **the active local model must have a tool-calling-compatible chat template.** Verify before writing any loop code:

- Known-good: **Llama 3.1/3.3, Llama 3.2, Qwen2.5, Mistral-Nemo** — native tool-call chat template formats
- Fallback: llama.cpp's "generic" tool-call style works with unsupported templates but is less token-efficient and less reliable
- **Test this in isolation first**, directly against `llama-server --jinja` with `curl`, one trivial tool, before writing any Rust orchestration:
```bash
llama-server --jinja -m <model>.gguf
curl http://localhost:8080/v1/chat/completions -d '{
  "messages": [{"role":"user","content":"What is 2+2, and what is the weather in Bangkok?"}],
  "tools": [{"type":"function","function":{"name":"get_weather","description":"Get weather","parameters":{"type":"object","properties":{"location":{"type":"string"}},"required":["location"]}}}]
}'
```
  Confirm the response actually contains a `tool_calls` block referencing `get_weather` with `location: "Bangkok"`, not a hallucinated text answer pretending to know the weather. **If this basic test fails, the model itself is the blocker — no amount of harness code fixes a model that can't emit valid tool calls.**

---

## 6. Library Options — Detailed

### Option A — Extend current stack directly (recommended starting point)
- Keep `llama-server` as-is, add `--jinja` + tool defs to existing request-building code
- Implement `run_agentic_loop` (§3) directly in `commands/engine.rs`, replacing the current `planner::decide()` dispatch
- **Effort:** contained — the loop itself is maybe 100-150 lines of Rust; most of the work is defining tool schemas and wiring existing capabilities as callable functions matching them
- **Risk:** low — no new dependency, behavior is fully inspectable in code you already own

### Option B — Adopt `rig-core` + `rig-llama-cpp`
- `rig-core`: mature Rust LLM agent framework, built-in tool-calling loop via a `.tool()` builder — handles the request/response/execute/re-prompt cycle for you
- `rig-llama-cpp`: dedicated Rig provider for local GGUF models via llama.cpp — streaming, tool calling, and reasoning support, purpose-built for exactly this local-inference setup
- `rig-mcp`: MCP (Model Context Protocol) support built in — the same tool-server standard Claude uses, meaning external MCP servers could be plugged in as tools with minimal custom code, rather than hand-writing a Rust module per new capability indefinitely
- **Effort:** larger — requires adapting `EngineState` and command structure to Rig's abstractions
- **Risk:** medium — real dependency, but a mature, widely-used one (7.6k+ stars, production users as of mid-2026)

### Recommendation
Ship **Option A** first — it proves the loop mechanics and the model's tool-calling reliability with minimal new surface area, and directly delivers everything in the §0 outcomes table. Revisit **Option B** specifically when/if MCP-server integration or a much larger, frequently-changing tool catalog becomes the priority — that's where Rig's abstraction pays for itself; it isn't required to hit the core "Claude-like loop" behavior being asked for here.

---

## 7. Interaction With Everything Already Planned

- **`adaptive-retrieval-orchestrator.md`** — its plan→retrieve→judge→refine sequence becomes the internal implementation of the `search_web` tool. No conflict; it slots in as-is.
- **`tiered-memory-system.md` / `memory-weight-and-background-agent-plan.md`** — memory assembly (Step 3) still runs once before the loop starts, unchanged. The loop doesn't touch memory directly.
- **`language-agnostic-classification-plan.md`** — this redesign **removes the need** for the `needs_search`/`is_constraint` routing classifiers specifically, since that decision is now made by the model's own tool-calling judgment rather than a separate classifier. The embedding/Tier-1 infrastructure isn't wasted — it still has a role in short-term constraint *extraction* (a structured-extraction task distinct from routing), but its routing responsibility goes away entirely.
- **`confirmed-bugs-and-fixes.md`** — the Thai-language keyword-matching bugs found there become moot for search/greeting routing specifically, since there's no more keyword-based routing to have that bug in. (The constraint-extraction bug in `short_term.rs` is a separate, still-relevant fix — extraction is not the same task as routing.)

---

## 8. Testing & Validation Plan (mapped to §0 outcomes)

| Test | Validates |
|---|---|
| Isolated `curl` tool-call test against `llama-server --jinja` (§5) | Model can emit valid tool calls at all — the hard prerequisite |
| "What's the weather in Bangkok and convert 32°C to Fahrenheit" | Multi-tool chaining within one turn (§0 row: "ใช้ tool ต่อกัน") |
| Plain greeting ("สวัสดี") | Loop exits with `FinalAnswer` on the first iteration, no tool called — proves the loop doesn't over-trigger tools now that there's no keyword-based skip logic to get it wrong |
| Genuinely ambiguous request (e.g. "help me plan a trip") | `ask_user_clarification` fires, loop suspends correctly, resumes correctly after the user's choice arrives |
| A request answerable from existing conversation context alone | `ask_user_clarification` does NOT fire — precision check, not just recall |
| Force a tool to return an error | Loop handles `is_error: true` gracefully, model can retry with adjusted arguments or a different tool rather than crashing |
| Deliberately force `iteration >= max_iterations` (e.g. mock a tool that always requests another call) | `force_final_answer` produces a real answer from partial evidence rather than hanging or erroring |

---

## 9. Migration Notes

- `ToolRoute` enum and `planner::decide()` can be deleted once the loop is verified working — don't keep both paths running in parallel long-term, that reintroduces exactly the kind of dual-decision-layer confusion this redesign is meant to eliminate
- The `<InteractiveChoiceBox />` frontend component and `ai-interaction-request` event contract need **no changes** — only the backend trigger for emitting that event moves from a router branch to a tool-call handler
- Existing tests written against the old router (if any) should be replaced with the tool-call-based tests in §8, not patched to fit the old structure
