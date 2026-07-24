# Aphelion System Documentation Index

Welcome to the comprehensive system documentation for **Aphelion AI Harness**. This documentation suite details every module, data pipeline, safety guard, and architectural connection across both backend and frontend layers.

---

## Documentation Modules

1. 🏛️ **[01. System Architecture & Component Mapping](01_system_architecture.md)**
   - High-level end-to-end execution flow
   - File and directory module mapping
   - Core data models (`ChatMessage`, `GenerationResult`, `AgentLoopOutcome`)

2. 🔄 **[02. Agentic Loop & Execution Mechanics](02_agentic_loop_and_execution.md)**
   - 8-hop iteration loop orchestrator (`agent_loop.rs`)
   - Tag unpacker & stringified JSON repair (`<|tool_call|>`, `call:`)
   - Seamless Stitching auto-continuation loop (`is_incomplete_text`)
   - Option safety fallbacks & empty response guards

3. 🛠️ **[03. Tool Catalog & Execution Reference](03_tools_and_catalog.md)**
   - Registered tool matrix (15 cataloged tools)
   - Detailed JSON schemas for inputs and outputs
   - Search, memory, UI interaction, workspace, and model discovery handlers

4. 🧠 **[04. 3-Tier Memory Architecture & Persistence](04_memory_and_persistence.md)**
   - Short-term turn constraints (`short_term.rs`)
   - Mid-term session goals (`mid_term.rs`)
   - Long-term personalization facts & Vector RAG (`long_term.rs`)
   - SQLite `harness.db` database schemas (`sessions`, `messages`, `pending_interactions`)

5. 🔍 **[05. Adaptive Web Search & RAG Pipeline](05_web_search_rag_pipeline.md)**
   - Source router & intent classification (`source_router.rs`)
   - Parallel multi-worker runtime (DuckDuckGo, Brave, SearXNG, Bing RSS, Crawl4AI)
   - BM25 & vector semantic embeddings reranker (`bm25.rs`)
   - Multi-pass confidence scorer & query expansion

6. 🎨 **[06. Frontend & Interactive UI Components](06_frontend_and_ui_components.md)**
   - React application shell (`App.tsx`)
   - Native interactive choice box (`InteractiveChoiceBox.tsx`)
   - Reasoning & Citations telemetry drawers
   - Tauri IPC event bridge
