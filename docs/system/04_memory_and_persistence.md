# 04. 3-Tier Memory Architecture & Persistence

## Overview

Aphelion features a state-of-the-art **3-Tier Memory Architecture** (`src-tauri/src/engine/memory/`) coupled with a temporal SQLite database store (`src-tauri/src/sessions/store.rs`). This allows the assistant to maintain short-term turn constraints, track mid-term session goals, and perform long-term vector RAG retrieval across multiple user sessions.

---

## 3-Tier Memory Architecture

```
[ User Message / Turn ]
           │
           ▼
┌─────────────────────────────────────────────────────────────┐
│ 1. Tier 1: Short-Term Constraints (`short_term.rs`)         │
│    • Turn-level rules, user-selected UI choices              │
│    • Expired automatically after turn completion             │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│ 2. Tier 2: Mid-Term Session Goals (`mid_term.rs`)           │
│    • Active topics, ongoing user goals, session progress     │
│    • Persisted across turns within the active session        │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│ 3. Tier 3: Long-Term Facts & Vector RAG (`long_term.rs`)     │
│    • Personalization facts, user traits, cross-session facts │
│    • Stored in vector embeddings & semantically recalled     │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
[ Primary Memory Block Injected into LLM System Prompt Header ]
```

---

## Memory Assembly & Prompt Injection Logic

In `src-tauri/src/commands/engine.rs`:
1. `assemble_tiered_memory_prompts()` compiles the 3 memory layers.
2. The `primary` block is inserted into `request.messages` as a `system` message right before the user prompt:
   ```markdown
   [Active Tiered Memory Context]
   - Active Constraints: User selected option "เทคโนโลยี AI"
   - Active Session Goals: Analyze Transformer models and LLMs
   - Long-Term Personalization: User name is "นิวส์", prefers concise Thai summaries
   ```
3. The `reminder` block (active constraints only) is inserted immediately preceding the latest user message for maximum model salience.

---

## SQLite Database Schemas (`harness.db`)

Persistence is handled by SQLite via `rusqlite` in `src-tauri/src/sessions/store.rs`.

### 1. `sessions` Table
| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | TEXT | PRIMARY KEY | Unique UUID string |
| `title` | TEXT | NOT NULL | Session title (auto-generated or "New chat") |
| `created_at` | TEXT | NOT NULL | ISO-8601 UTC timestamp |
| `updated_at` | TEXT | NOT NULL | ISO-8601 UTC timestamp |
| `model_id` | TEXT | NULL | GGUF model identifier used |
| `conversation_memory`| TEXT| NOT NULL | Compressed conversation summary text |

### 2. `messages` Table
| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | TEXT | PRIMARY KEY | Unique UUID string |
| `session_id` | TEXT | FOREIGN KEY | References `sessions(id)` |
| `role` | TEXT | NOT NULL | "user", "assistant", "system", "tool" |
| `content` | TEXT | NOT NULL | Main message body text |
| `thinking_summary` | TEXT| NULL | Accumulated reasoning steps text |
| `finish_reason` | TEXT| NULL | "stop", "length", "tool_calls" |
| `web_sources` | TEXT | NULL | Serialized JSON array of web sources |
| `retrieval_trace` | TEXT | NULL | Serialized JSON array of telemetry events |
| `sequence` | INTEGER| NOT NULL | Monotonic sequence number |
| `created_at` | TEXT | NOT NULL | ISO-8601 UTC timestamp |

### 3. `pending_interactions` Table
| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | TEXT | PRIMARY KEY | Unique UUID string |
| `session_id` | TEXT | FOREIGN KEY | References `sessions(id)` |
| `request_content` | TEXT | NOT NULL | Original user request prompt |
| `question` | TEXT | NOT NULL | Clean question header text |
| `options_json` | TEXT | NOT NULL | Serialized JSON array of options |
| `status` | TEXT | NOT NULL | "pending", "resolved", "superseded" |
| `selected_option_id`| TEXT| NULL | Option ID selected by user |
| `created_at` | TEXT | NOT NULL | ISO-8601 UTC timestamp |
