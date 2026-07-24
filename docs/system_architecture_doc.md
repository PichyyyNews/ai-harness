# Aphelion AI System Architecture & Module Reference

This document provides a comprehensive technical overview of the complete Aphelion AI Harness architecture, module contracts, data flows, and safety protections as of July 2026.

---

## 1. High-Level Architecture Overview

Aphelion is an autonomous, multi-turn AI harness built on top of Rust (Tauri v2 backend) and React (TypeScript frontend) communicating with a local `llama-server` GGUF inference engine.

```
[ User Input / React UI ] 
        │
        ▼
[ Tauri IPC Handler (`commands/engine.rs`) ]
        │
        ├─────────────────────────────────────────┐
        ▼                                         ▼
[ 3-Tier Memory Manager ]              [ SQLite Temporal Store ]
  ├── Tier 1: Turn Constraints           └── `sessions`, `messages`, `pending_interactions`
  ├── Tier 2: Session Goals
  └── Tier 3: Long-term Vector RAG
        │
        ▼
[ Multi-Hop Agent Loop (`tools/agent_loop.rs`) ] ◄─────► [ Local `llama-server` Endpoint ]
  ├── Max Token & Iteration Control (8 hops)
  ├── Tool Call Parser & Tag Unpacker (`<|tool_call|>`, `call:`)
  ├── Seamless Stitching Auto-Continuation Loop
  ├── Empty Response Guard & Forced Final Synthesis
  └── Choice Interception & UI Suspension (`ask_user_clarification`)
        │
        ├──► `search_web` ──► [ Adaptive Web Search Orchestrator (`web_search/`) ]
        │                       ├── Source Router (News, Finance, Tech, Weather)
        │                       ├── Multi-Engine Parallel Workers (DDG, Brave, SearXNG, Bing, Crawl4AI)
        │                       └── BM25 & Semantic Reranker
        │
        └──► `ask_user_clarification` ──► [ Native InteractiveChoiceBox UI ]
                                            ├── Question Header (Unpacked Clean Text)
                                            ├── Radio Button Selection Array (4 options + fallback)
                                            └── Custom Text Response Input
```

---

## 2. Component Deep Dive & File Reference

### 2.1 UI Layer & Native Interaction (`src/`)
- **`src/App.tsx`:** Manages main chat session state, listens for Tauri events (`ai-interaction-request`, `engine-status`, `retrieval-trace`), handles streaming text chunks, and renders the message feed.
- **`src/components/InteractiveChoiceBox.tsx`:** Renders interactive modal option cards when suspended for user choices. Displays clean Thai headers, up to 4 radio button options, custom write-in input field, and Submit/Skip controls.
- **`src/components/InteractiveChoiceBox.module.css`:** Custom modern dark-mode styling with subtle animations, hover states, glassmorphism, and responsive layout.

### 2.2 Core Command Engine & Memory (`src-tauri/src/commands/engine.rs`)
- **`generate_chat` Command Handler:**
  - Manages session lifecycle and message database rehydration.
  - Assembles and injects the 3-Tier Memory layers into the prompt before user messages.
  - Injects System Core Directives (`tools_system_prompt()`).
  - Dispatches execution to `tools::agent_loop::run_agentic_loop(...)`.
  - Guards SQLite message persistence to prevent saving empty assistant blocks (`!result.content.trim().is_empty()`).

### 2.3 3-Tier Memory Architecture (`src-tauri/src/engine/memory/`)
- **`short_term.rs` (Tier 1):** Manages turn-level boundary constraints and immediate user preferences. Expired automatically across turns.
- **`mid_term.rs` (Tier 2):** Tracks active session goals, topic progression, and multi-step conversation context across turns within a session.
- **`long_term.rs` (Tier 3):** Cross-session personalization facts and vector RAG retrieval. Stores user traits, preferences, and factual facts in vector embeddings for semantic recall.

### 2.4 Multi-Hop Agent Loop (`src-tauri/src/tools/agent_loop.rs`)
- **`run_agentic_loop` Orchestrator:**
  - Controls up to 8 iteration hops of reasoning, tool calls, and execution results.
  - Sets `"max_tokens": 4096` to eliminate premature generation cut-offs.
  - Captures and accumulates `sources`, `retrieval_trace`, and `thinking_steps`.

- **Tag Unpacker & Sanitizer:**
  - Parses OpenAI-spec tool calls as well as raw model text tags (`<|tool_call|>call:tool_name\n{...}\n<tool_call|>`, `call:tool_name`, `<function=tool_name>`).
  - Unpacks stringified JSON objects buried inside single argument string fields (e.g. `question: "{\"options\": [...], \"question\": \"...\"}"`).
  - Automatically repairs truncated JSON strings missing closing `}`.

- **Seamless Stitching Auto-Continuation Loop:**
  - Detects cut-off answers missing sentence-ending marks using `is_incomplete_text`.
  - Automatically executes up to 3 seamless continuation requests to `llama-server` using directive: `"Continue exactly where the previous answer ended. Do not repeat text; preserve the language and Markdown structure."`
  - Stitches continuation text directly to the answer before returning to the UI.

- **Fallback Option Population:**
  - If `ask_user_clarification` is called or extracted without an `options` array, automatically populates standard default category buttons so 0-option radio button cards never appear in the UI.

### 2.5 Adaptive Web Search & RAG (`src-tauri/src/web_search/`)
- **`orchestrator.rs`:** Multi-pass search planner. Runs initial queries, calculates confidence scores (Relevance × Agreement × Coverage), and triggers secondary expansion passes if confidence < 0.55.
- **`source_router.rs`:** Intent classifier determining target sources (Tech, News, Financial, Weather, Wikipedia, General).
- **`worker_runtime.rs`:** Parallel async workers executing queries concurrently across DuckDuckGo, Brave Search, SearXNG, Bing RSS, and Crawl4AI deep scraper.
- **`bm25.rs`:** Hybrid BM25 keyword matching + vector cosine similarity embeddings reranker.

### 2.6 Local Inference Runtime (`src-tauri/src/engine/runtime.rs`)
- **llama-server Endpoint Integration:** Communicates with `http://127.0.0.1:8080/v1/chat/completions`.
- **400 Bad Request Fallback:** Automatically retries without `tools` payload if a local model's Jinja template rejects tool calling syntax.
- **Error Transparency:** Extracts full HTTP error response bodies into error messages for easy diagnostics.

---

## 3. Key System Safeguards

| Feature / Issue | Safeguard Implementation | Location |
| :--- | :--- | :--- |
| **Response Cut-offs** | `max_tokens: 4096` + Seamless Stitching Auto-Continuation Loop | `agent_loop.rs` |
| **Empty Message Cards** | `append_message` guarded by `!result.content.trim().is_empty()` + `force_final_answer` | `commands/engine.rs` & `agent_loop.rs` |
| **0-Option Choice Cards** | Fallback option array population (`options.is_empty()`) | `agent_loop.rs` |
| **Truncated JSON in Tool Args** | JSON repair (`format!("{raw_question}}}")`) + Substring Extractor | `agent_loop.rs` |
| **Raw Tag Leakage in UI** | Extractor strips `<|tool_call|>` and converts to native UI choice event | `agent_loop.rs` |
| **[choice-pending] String** | Placeholder cleared (`content: ""`) on UI suspension | `agent_loop.rs` |
