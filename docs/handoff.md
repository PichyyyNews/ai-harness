# AI Harness — Handoff Document
> Last updated: 2026-07-22 (session covering tiered memory, UI responsiveness, and lock-free background workers)

---

## 1. Project Overview

**AI Harness** is a desktop app built with **Tauri 2 + React/Vite (TypeScript)** frontend and a **Rust** backend (`src-tauri`). It downloads GGUF models, runs a local `llama.cpp` sidecar (`llama-server`), and provides streaming chat with:

- Persistent session history in SQLite (`harness.db`)
- Tiered memory system (short / mid / long-term)
- Background model workers for silent memory extraction
- Real-time web search grounding
- Context usage donut chart indicator
- Temporal context (local + network time calibration)

---

## 2. Current Architecture

### Frontend — `src/App.tsx`
- React state machine managing sessions, streaming, UI tabs
- `streaming` state: `true` while AI is generating → gates `sendMessage`
- `setStreaming(false)` must always be called in the `finally` block of `sendMessage`
- `<Textarea>` is only gated by `engineStarted`, never by `streaming` — user can always type
- Context Donut Chart: SVG next to Send button, shows token usage (~3.8 chars/token estimate), hover tooltip

### Backend — `src-tauri/src/`

```
commands/
  engine.rs        — generate_chat, start/stop engine, trigger_session_end_memory
  sessions.rs      — create/list/get/delete sessions, generate_session_title (lock-free)
engine/
  runtime.rs       — Engine struct, endpoint() getter, llama-server process management
  context_manager.rs — ConversationMemory, generate_with_recovery, web budget
  memory/
    mod.rs         — assemble_tiered_memory_prompts()
    worker.rs      — run_after_turn_extraction, run_session_end_extraction (NO Mutex inside)
    short_term.rs  — session-scoped constraints, expire_turn_constraints
    mid_term.rs    — goals, decisions, plan steps per session
    long_term.rs   — durable user facts (filtered for sensitivity)
  time_manager.rs  — TimeAuthority, network IANA timezone calibration
  faithfulness.rs  — claim grounding checker
  repetition_guard.rs
state.rs           — EngineState { engine: Mutex<Option<Engine>>, ... }
```

---

## 3. Critical Architecture Rule — Mutex / Lock Protocol

**The most important invariant in the codebase:**

```
state.engine: Mutex<Option<Engine>>
```

| Thread | Allowed to hold lock? | Duration |
|--------|----------------------|----------|
| `generate_chat` (user message) | ✅ Yes | Full generation duration |
| `generate_session_title` | ❌ Never during HTTP | Read endpoint → drop immediately |
| `run_after_turn_extraction` | ❌ Never | Receives endpoint as parameter |
| `run_session_end_extraction` | ❌ Never | Receives endpoint as parameter |

**Pattern — how to add any new background worker:**

```rust
// 1. Read endpoint while you already hold the lock (e.g. inside generate_chat)
let bg_endpoint = engine.endpoint().to_string();

// 2. Drop ALL guards before spawning
drop(current);   // MutexGuard for engine
drop(memory);    // MutexGuard for conversation_memory

// 3. Spawn thread — pass endpoint as owned String, no Mutex inside
std::thread::spawn(move || {
    my_worker::run(&app, &bg_endpoint, &session_id, ...);
});
```

**Never call `try_lock()` in a background thread after `generate_chat` is running** — it will always fail because `generate_chat` holds the lock for the full generation duration.

---

## 4. Tiered Memory System

### Memory Layers

| Layer | File | Scope | Trigger |
|-------|------|-------|---------|
| Short-term | `short_term.rs` | Constraints active this session | After each turn |
| Mid-term | `mid_term.rs` | Goals, decisions, plan steps | After each turn |
| Long-term | `long_term.rs` | Durable user facts (filtered) | Session end |

### Background Worker Flow

1. `generate_chat` completes → saves assistant response to DB
2. Reads `engine.endpoint()` (still holding lock)
3. Drops lock, spawns `std::thread::spawn`
4. `run_after_turn_extraction(app, endpoint, session_id, user_msg, assistant_msg)`:
   - Sends prompt to `llama-server` via `reqwest::blocking` (direct HTTP, no Mutex)
   - Parses JSON: `constraints`, `goals`, `decisions`, `plan_steps`
   - Saves to SQLite via `short_term::save_extracted_constraints` / `mid_term::merge_extracted_memory`
5. On session switch / new chat → frontend calls `trigger_session_end_memory` IPC
6. `run_session_end_extraction(app, endpoint, session_id)`:
   - Reads last 12 messages from DB
   - Extracts `facts` + `session_summary`
   - Saves via `long_term::process_extracted_facts` + `store::save_session_summary`

### Prompt Engineering Notes for Local LLMs

**DO NOT use bracketed meta-headers** like `[Active Session Constraints - Non-negotiable]` in system prompts — 3B–8B local models hallucinate meta-comments in response (e.g. `(Wait, I must remove the emoji!)`).

**Use natural language headers instead:**
```
Important Instructions & Constraints:
- Directive: Do not use emojis.

User Profile & Background Context:
- Preference: User communicates in Thai.

Session Context & Progress:
- Goal: Build AI Harness desktop app.
```

---

## 5. Bugs Fixed This Session

### Bug 1 — `streaming` state never reset (critical)
**Symptom:** After any AI response, UI showed `+ Generating` with Stop button forever. Sending next message was impossible.

**Root cause:** `setStreaming(true)` called in `sendMessage` but `setStreaming(false)` was **never called anywhere**.

**Fix:** Added `setStreaming(false)` as the first line of the `finally` block in `sendMessage`.

```diff
  } finally {
+   setStreaming(false);
    if (pendingAnimationFrame.current !== null) cancelAnimationFrame(pendingAnimationFrame.current);
    flushPendingDelta();
    if (streamAbort.current === controller) streamAbort.current = null;
  }
```

---

### Bug 2 — Memory worker always getting empty response (`expected value at line 1 column 1`)
**Symptom:** `[memory-worker] After-turn JSON parse skipped: expected value at line 1 column 1` on every turn.

**Root cause:** Background thread called `try_lock()` on `state.engine` **while `generate_chat` still held the lock** → always returned `Err(_)` → function returned empty string → JSON parse failed.

**Fix:** Changed architecture so `worker.rs` functions **never touch the Mutex**:
1. Read `engine.endpoint()` inside `generate_chat` while lock is still held
2. Clone to owned `String`
3. Drop all guards
4. Spawn thread with endpoint as parameter

```rust
// In generate_chat (holds lock):
let bg_endpoint = engine.endpoint().to_string();
drop(current);
drop(memory);
std::thread::spawn(move || {
    worker::run_after_turn_extraction(&bg_app, &bg_endpoint, &bg_session_id, ...);
});
```

```rust
// In worker.rs (no Mutex at all):
pub fn run_after_turn_extraction(app: &AppHandle, endpoint: &str, ...) {
    run_silent_generation(endpoint, &prompt, 512)
}

fn run_silent_generation(endpoint: &str, prompt: &str, max_tokens: u32) -> Result<String, String> {
    reqwest::blocking::Client::new()
        .post(format!("{}/v1/chat/completions", endpoint))
        ...
}
```

---

### Bug 3 — Second message blocked by `generate_session_title` Mutex
**Symptom:** Sending a follow-up message right after first response blocked indefinitely.

**Root cause:** `generate_session_title` held `state.engine.lock()` for entire HTTP title generation request. Second `generate_chat` call blocked waiting for the same lock.

**Fix:** `sessions.rs` now reads endpoint using `try_lock()` for <1μs, drops guard immediately, then makes HTTP call directly.

---

## 6. Build Status

```
cargo test    — 23/23 tests passing, 0 failures
cargo check   — 0 errors, 0 warnings
npx tsc --noEmit — 0 TypeScript errors
```

---

## 7. Key Files Quick Reference

| File | What to know |
|------|-------------|
| [`src/App.tsx`](../src/App.tsx) | Main UI — `sendMessage`, `streaming` state, `ContextDonutChart` |
| [`src-tauri/src/commands/engine.rs`](../src-tauri/src/commands/engine.rs) | `generate_chat`, mutex/lock pattern, background thread spawn |
| [`src-tauri/src/commands/sessions.rs`](../src-tauri/src/commands/sessions.rs) | `generate_session_title` (lock-free) |
| [`src-tauri/src/engine/memory/worker.rs`](../src-tauri/src/engine/memory/worker.rs) | Background memory extraction — takes `endpoint: &str`, NO Mutex |
| [`src-tauri/src/engine/memory/short_term.rs`](../src-tauri/src/engine/memory/short_term.rs) | Session constraints + turn expiry |
| [`src-tauri/src/engine/memory/mid_term.rs`](../src-tauri/src/engine/memory/mid_term.rs) | Goals, decisions, plan steps |
| [`src-tauri/src/engine/memory/long_term.rs`](../src-tauri/src/engine/memory/long_term.rs) | Durable facts, sensitivity filter |
| [`src-tauri/src/state.rs`](../src-tauri/src/state.rs) | `EngineState` — Mutex definitions |

---

## 8. Next Steps / Known Gaps

- [ ] **Verify memory extraction is working end-to-end** — run app, check terminal for `[memory-worker] after-turn: extracted N constraints` with N > 0
- [ ] **Add debug UI panel** to view what memory was extracted per session (short/mid/long-term stored in DB)
- [ ] **Tune extraction prompts** — current prompts may over-extract or miss constraints depending on model
- [ ] **Session summary quality** — `session_summary` stored in DB but not yet surfaced in UI or used in context assembly
- [ ] **Memory retrieval relevance** — `assemble_tiered_memory_prompts` could be smarter (semantic similarity vs. recency)
- [ ] **Long-term fact deduplication** — `process_extracted_facts` appends rows; add merging/dedup logic
- [ ] **UI for memory viewer** — let user inspect/edit/delete stored memory (privacy control)
- [ ] **Context budget** — currently memory prompts are prepended without checking total token budget; could overflow on small models

---

## 9. Temporal Context (from previous session)

- `TimeAuthority` in `time_manager.rs` combines OS clock + network IANA timezone
- Network sources: `ipwho.is` → `timeapi.io` (timeout 3s, cache 15min, fallback 24h)
- Only stores timezone ID, calibrated time, monotonic sync — no IP/location persisted
- Every generation prepends a structured temporal system message

---

## 10. Docs Index

| Document | Description |
|----------|-------------|
| [`handoff.md`](handoff.md) | This document |
| [`master-plan.md`](master-plan.md) | Feature roadmap and design goals |
| [`tiered-memory-system.md`](tiered-memory-system.md) | Memory system design spec |
| [`adaptive-retrieval-orchestrator.md`](adaptive-retrieval-orchestrator.md) | Web search pipeline design |
| [`handoff-2026-07-22-web-search.md`](handoff-2026-07-22-web-search.md) | Previous session — web search implementation |
