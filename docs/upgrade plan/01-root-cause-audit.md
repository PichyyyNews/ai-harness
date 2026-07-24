# Root Cause Audit — Why the Agentic System Still Breaks

Audit of the actual implemented Aphelion system (from the 7 provided docs) against the intended design. Verdict: **the big-picture architecture was actually built correctly** (3-tier memory, 15-tool catalog, agent loop, adaptive RAG pipeline all exist and largely match the plan). The failures trace to a small number of specific, fixable root causes — not a wrong overall approach.

---

## Root Cause A — Tool Calls Are Recovered From Free Text, Not Produced as Structured Output (THE primary bug)

**Evidence, direct from `02_agentic_loop_and_execution.md`:**

> "Local models (like DeepSeek, Qwen, Llama 3) often emit tool calls wrapped in text tags or with stringified JSON inside single arguments. The parser handles all variations: `<|tool_call|>call:tool_name\n{...}\n<tool_call|>`, `call:tool_name{...}`, `<function=tool_name>{...}</function>`, Standard OpenAI JSON `message.tool_calls`... Stringified JSON Unpacking... JSON Auto-Repair (appends `}` or `"]}`)..."

This is the smoking gun. A system built on genuine native tool-calling (llama.cpp `--jinja` + grammar-constrained JSON schema output) **does not need** four different text-tag formats, stringified-JSON unpacking, or JSON auto-repair for truncated braces. Those are all recovery mechanisms for a model that is **guessing at tool-call syntax in free-form text**, not being **constrained** to emit valid structured output. This is functionally identical, one layer deeper, to the exact problem the whole redesign was meant to eliminate: instead of hardcoded string-matching deciding *when* to call a tool, there's now hardcoded string-matching deciding *whether the model's attempt at a tool call can be salvaged at all*.

**Why this explains "พอเปลี่ยนบริบท LLM ก็ใช้ tool ไม่ได้แล้ว" exactly:** free-text tool-call emission is *inherently less reliable as context grows* — longer conversations, more accumulated tool round-trips, and more memory content all increase the chance the model drifts from whatever exact text pattern the parser anticipates. The regex/tag parser can only recover **known** malformations; a novel one (a slightly different tag spacing, a missing bracket in an unanticipated place, a model that switches formatting mid-conversation) silently fails, and from the user's perspective, "tools stopped working" — with no error, just silent failure to parse.

**This is not a small bug to patch — it's the foundational reliability gap** everything else sits on top of. Fixing it properly (§ in the upgrade plan) is the single highest-leverage change available.

---

## Root Cause B — Memory's "Recency Placement" Breaks the Moment the Agent Loop Runs More Than One Hop

**Evidence, from `04_memory_and_persistence.md`:**

> "The `reminder` block (active constraints only) is inserted immediately preceding the latest user message for maximum model salience."

This placement strategy is correct **for a single-shot request**. But the agent loop (`02_agentic_loop_and_execution.md`) appends new messages every iteration — assistant tool-call messages, then tool-result messages — **after** the point where the reminder block was inserted. Once the loop runs a second hop, the memory reminder is no longer "immediately preceding the latest message" — it's now buried under one or more rounds of tool-call/tool-result messages. The exact salience mechanism designed to make memory *matter* (`memory-weight-and-background-agent-plan.md` §2.2, "sandwich placement") **silently stops working specifically on multi-hop tool-using turns** — which, ironically, are the turns where the system is doing the most "intelligent" work and therefore the ones where staying grounded in memory/constraints matters most.

**This directly explains** "ระบบความทรงจำ...ก็หายไปเลย ไม่มาเชื่อมต่อกันอีก" — memory doesn't literally disappear from the database, it disappears from **effective model attention** the moment a turn involves tool use, because nothing re-anchors it near the end of the growing message list on each loop iteration.

---

## Root Cause C — System Prompt Sprawl (Unverified, Flagged for Review)

`01_system_architecture.md` lists memory injection (Step 3, itself 3 sub-blocks) AND a separate "System Core Directives Injection" (Step 4, `tools/mod.rs`) as distinct injected blocks. Combined with per-tool descriptions in the `tools` schema and the memory `primary`/`reminder` split, there are at least **4-5 separate instruction-bearing blocks** competing for the model's attention on every request. The docs don't show `tools/mod.rs`'s actual directive content, so this can't be confirmed as a bug from the docs alone — but it's a real risk pattern worth auditing directly: multiple, possibly overlapping or even contradictory system-level instructions generally dilute compliance with all of them, compared to one coherent, well-ordered block.

---

## Root Cause D — The Same Brittle-Heuristic Pattern Recurs Elsewhere (not just in routing)

Two more hardcoded pattern-matches found in the provided docs, same disease as the routing/keyword-list bugs found earlier, just in different subsystems:

1. **`is_incomplete_text`** (`02_agentic_loop_and_execution.md`) checks whether a response ends with `.`, `!`, `?`, `ครับ`, `ค่ะ`, `]`, `}`, `)` to decide whether to auto-continue. This is a hardcoded, English+Thai-only punctuation list — the exact same category of bug as the earlier query-routing/constraint-extraction issues, just recurring in the auto-continuation subsystem. It will misfire on any other language, and worse, **the API already provides a definitive, language-agnostic signal for this** (`finish_reason == "length"` vs `"stop"`) that makes the entire heuristic unnecessary — this is a fixable-in-one-line problem, not a hard one, once identified.

2. **"Option Safety Fallback"** (`02_agentic_loop_and_execution.md`) hardcodes 4 fixed Thai category options whenever `ask_user_clarification`'s extracted `options` array comes back empty. This is a symptom, not a cause — it's a patch reacting to Root Cause A (malformed tool-call JSON losing the `options` field during text-parsing recovery). Once native structured tool-calling is fixed, a required-field JSON schema makes an empty `options` array structurally impossible in the first place, and this fallback becomes a rare last-resort rather than something apparently common enough to have been explicitly patched for.

---

## What's Actually Working Well (don't rebuild these)

- **3-tier memory schema and separation of concerns** (`04_memory_and_persistence.md`) — the tier boundaries and SQLite schema are sound
- **`pending_interactions` table + `AgentLoopOutcome::Completed | SuspendedForUserChoice`** — the suspend/resume design for clarification questions is correctly modeled
- **Adaptive web search pipeline** (`05_web_search_rag_pipeline.md`) — source routing, parallel multi-provider search, BM25+semantic reranking, and confidence-gated query expansion (threshold 0.55) all match the intended design closely and don't need architectural rework — they need to actually *get invoked reliably*, which is a symptom of Root Cause A, not a flaw in the pipeline itself
- **Frontend event contract** (`06_frontend_and_ui_components.md`) — `ai-interaction-request` → `InteractiveChoiceBox.tsx` → `resolve_pending_interaction` is a clean, correctly-modeled round trip

## Summary — Priority Order

1. **Root Cause A (tool-call reliability)** — fix first, everything downstream depends on tools actually firing consistently
2. **Root Cause B (memory recency in multi-hop loops)** — fix second, directly restores "memory feels connected" during tool use
3. **Root Cause D (residual heuristics)** — fix alongside A/B, small isolated changes
4. **Root Cause C (prompt sprawl)** — audit `tools/mod.rs` directly to confirm/deny, consolidate if confirmed
