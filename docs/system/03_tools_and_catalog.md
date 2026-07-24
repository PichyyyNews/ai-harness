# 03. Tool Catalog & Execution Reference

## Overview

Aphelion equips the local LLM with **15 registered tools** defined in `src-tauri/src/tools/catalog.rs` and dispatched via `src-tauri/src/tools/mod.rs`. These tools span web search grounding, interactive UI choice prompts, workspace access, session memory retrieval, and external model discovery.

---

## Registered Tool Matrix

| # | Tool Name | Description | Arguments Schema | Handler Location |
| :- | :--- | :--- | :--- | :--- |
| 1 | `search_web` | Searches the web using adaptive multi-engine RAG pipeline | `{ query: string }` | `web_search/orchestrator.rs` |
| 2 | `ask_user_clarification` | Asks the user a clarifying question with native UI radio buttons | `{ question: string, options: string[] }` | `agent_loop.rs` |
| 3 | `ask_user_choice` | Presents options for user selection in native UI | `{ question: string, options: string[] }` | `agent_loop.rs` |
| 4 | `search_chat_history` | Searches past messages in SQLite database | `{ query: string }` | `sessions/store.rs` |
| 5 | `get_session_details` | Retrieves full session conversation history | `{ session_id?: string }` | `sessions/store.rs` |
| 6 | `search_huggingface_models` | Queries HuggingFace Hub for open-source GGUF models | `{ query: string }` | `tools/mod.rs` |
| 7 | `read_workspace_file` | Reads contents of a file in the active workspace | `{ path: string }` | `tools/workspace.rs` |
| 8 | `write_workspace_file` | Writes text content to a file in the active workspace | `{ path: string, content: string }` | `tools/workspace.rs` |
| 9 | `list_workspace_files` | Lists files and directories in the active workspace | `{ path?: string }` | `tools/workspace.rs` |
| 10| `get_time_context` | Returns current system clock, ISO-8601, and timezone | `{}` | `engine/time_manager.rs` |
| 11| `summarize_text` | Summarizes a long text block | `{ text: string }` | `tools/mod.rs` |
| 12| `extract_keywords` | Extracts key concepts and entities from text | `{ text: string }` | `tools/mod.rs` |
| 13| `format_markdown_table`| Formats raw structured data into a Markdown table | `{ data: string }` | `tools/mod.rs` |
| 14| `calculate_math` | Evaluates mathematical expressions safely | `{ expression: string }` | `tools/mod.rs` |
| 15| `get_system_specs` | Returns CPU, RAM, and GPU hardware profile | `{}` | `hardware.rs` |

---

## Detailed Tool Contracts

### 1. `search_web`
- **Purpose:** Primary web grounding tool. Dispatches query to `web_search::orchestrator::run_adaptive_pipeline()`.
- **Input JSON:**
  ```json
  { "query": "10 แนวโน้มเทคโนโลยี AI ปี 2030" }
  ```
- **Returns:** Grounded Markdown context snippet, list of `sources` (url, title, snippet), and `retrieval_trace` telemetry events.

### 2. `ask_user_clarification`
- **Purpose:** Prompts the user to clarify intent via native React radio button UI widget.
- **Input JSON:**
  ```json
  {
    "question": "นิวส์สนใจที่จะให้ผมช่วยหาข้อมูลในหัวข้อหลักด้านใดครับ?",
    "options": [
      "เทคโนโลยีและปัญญาประดิษฐ์ (AI / LLMs)",
      "การวิเคราะห์ตลาดและการเงิน",
      "ข่าวสารและเหตุการณ์ปัจจุบัน",
      "หัวข้ออื่น ๆ (ระบุได้)"
    ]
  }
  ```
- **Behavior:** Suspends the agentic loop, saves a `pending` interaction to SQLite, and emits the `ai-interaction-request` event to the frontend.

### 3. `search_chat_history`
- **Purpose:** Searches through SQLite `messages` table for past conversations matching keywords.
- **Input JSON:**
  ```json
  { "query": "สถาปัตยกรรม AI" }
  ```
- **Returns:** Formatted list of past matching message snippets and session IDs.

### 4. `search_huggingface_models`
- **Purpose:** Queries HuggingFace Hub REST API for popular GGUF model repositories.
- **Input JSON:**
  ```json
  { "query": "Qwen2.5-Coder-7B-GGUF" }
  ```
- **Returns:** List of model IDs, download counts, and file formats.
