# 01. System Architecture & Component Mapping

## Overview

**Aphelion AI Harness** is an enterprise-grade, autonomous local AI desktop application built with a high-performance **Rust (Tauri v2)** backend and a dynamic **React + TypeScript** frontend. It interfaces directly with local `llama-server` GGUF inference engines to execute multi-step reasoning, multi-engine web search, 3-tier memory management, and native interactive UI choices.

---

## Complete End-to-End Execution Flow

```
[ User Interaction / React UI ]
             │
             │ (Tauri IPC `generate_chat`)
             ▼
[ Tauri Command Handler (`src-tauri/src/commands/engine.rs`) ]
             │
             ├── 1. SQLite Database Rehydration (`sessions/store.rs`)
             ├── 2. User Intent & Prompt Enhancement (`tools/prompt_enhancer.rs`)
             ├── 3. 3-Tier Memory Injection (`engine/memory/mod.rs`)
             │      ├── Short-Term Constraints (Turn boundaries)
             │      ├── Mid-Term Session Goals
             │      └── Long-Term Personalization Vector RAG
             └── 4. System Core Directives Injection (`tools/mod.rs`)
             │
             ▼
[ Multi-Hop Agent Loop (`src-tauri/src/tools/agent_loop.rs`) ] ◄─────► [ Local `llama-server` ]
             │                                                           (http://127.0.0.1:8080)
             ├── Max Tokens (4096) & Max Hops (8 iterations)
             ├── Tool Call Parser & Tag Unpacker (`<|tool_call|>`, `call:`)
             ├── Seamless Stitching Auto-Continuation Loop
             └── Empty Response Guard & Forced Synthesis
             │
             ├─────────────────────────────────────────┐
             ▼                                         ▼
[ Tool Execution Handlers ]             [ Native Choice UI Interception ]
  ├── `search_web`                        ├── `ask_user_clarification`
  ├── `search_chat_history`               └── `ask_user_choice`
  ├── `get_session_details`                            │
  ├── `search_huggingface_models`                      │ (Tauri Event `ai-interaction-request`)
  └── (15 cataloged tools)                             ▼
             │                          [ React `InteractiveChoiceBox.tsx` ]
             ▼                            ├── Clean Unpacked Question Header
[ Adaptive Web Search (`web_search/`) ]   ├── 4 Radio Button Options
  ├── Source Router                       └── Custom Text Response Input
  ├── Parallel Fetch (DDG, Brave, etc.)
  ├── Crawl4AI Deep Scraper
  └── BM25 + Vector Semantic Reranker
```

---

## Directory & Module Structure Mapping

| Component Layer | Path | Core Responsibility |
| :--- | :--- | :--- |
| **Frontend UI** | `src/App.tsx` | Main application shell, chat feed, streaming listener |
| **Choice Modal** | `src/components/InteractiveChoiceBox.tsx` | Interactive radio buttons and custom write-in input |
| **IPC Bridge** | `src-tauri/src/commands/engine.rs` | Main Tauri command handlers (`generate_chat`, `get_session_details`) |
| **Agentic Loop** | `src-tauri/src/tools/agent_loop.rs` | 8-hop iteration loop, tool call parsing, Seamless Stitching |
| **Tool Handlers** | `src-tauri/src/tools/mod.rs` | System directives, tool dispatch, grounding prompts |
| **Tool Catalog** | `src-tauri/src/tools/catalog.rs` | 15 tool definitions and OpenAPI parameter schemas |
| **Memory Manager**| `src-tauri/src/engine/memory/` | 3-tier memory (short, mid, long-term Vector RAG) |
| **Web Search** | `src-tauri/src/web_search/` | Adaptive RAG, parallel workers, BM25 reranker, Crawl4AI |
| **Database Store**| `src-tauri/src/sessions/store.rs` | SQLite persistence for sessions, messages, and interactions |
| **Local Runtime** | `src-tauri/src/engine/runtime.rs` | Process manager for `llama-server` GGUF execution |

---

## Data Model & Message Contracts

### 1. ChatMessage (`src-tauri/src/engine/mod.rs`)
```rust
pub struct ChatMessage {
    pub role: String,               // "user", "assistant", "system", "tool"
    pub content: String,            // Main text payload
    pub tool_calls: Option<Vec<Value>>, // Structured tool requests
    pub tool_call_id: Option<String>,// ID of tool call being responded to
    pub name: Option<String>,       // Name of tool
    pub created_at: Option<String>, // ISO-8601 UTC timestamp
}
```

### 2. GenerationResult (`src-tauri/src/engine/mod.rs`)
```rust
pub struct GenerationResult {
    pub content: String,
    pub finish_reason: FinishReason,
    pub sources: Vec<WebSource>,
    pub retrieval_trace: Vec<RetrievalTraceEvent>,
    pub thinking_summary: Option<String>,
}
```

### 3. AgentLoopOutcome (`src-tauri/src/tools/agent_loop.rs`)
```rust
pub enum AgentLoopOutcome {
    Completed(GenerationResult),
    SuspendedForUserChoice(GenerationResult),
}
```
