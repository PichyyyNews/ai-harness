# 06. Frontend & Interactive UI Components

## Overview

The Aphelion frontend is built with **React**, **TypeScript**, and **CSS Modules**, hosted inside Tauri v2 desktop webview. It interfaces with the Rust backend via Tauri IPC commands (`invoke`) and real-time event streams (`listen`).

---

## Frontend Architecture Diagram

```
[ `App.tsx` (Main Shell) ]
     │
     ├── Session Manager (Sidebar, Create Chat, List Sessions)
     ├── Message Feed (User messages, Assistant Markdown, Citations)
     ├── Status Bar (Local engine status, Memory count, GPU stats)
     │
     ├── Event Listeners:
     │     ├── `ai-interaction-request` ──► Triggers [ `InteractiveChoiceBox.tsx` ]
     │     ├── `engine-status`          ──► Updates Status Bar
     │     └── `retrieval-trace`       ──► Populates Telemetry Drawer
     │
     └── IPC Commands (`invoke`):
           ├── `generate_chat`
           ├── `start_engine` / `stop_engine`
           ├── `get_session_details`
           └── `resolve_pending_interaction`
```

---

## Component Deep Dive

### 1. Main Shell (`src/App.tsx`)
- **Session Lifecycle:** Manages `activeSessionId`, fetches message history on session selection, and updates session titles.
- **Message Rendering:** Renders user bubbles and assistant Markdown responses. Includes custom rendering for:
  - **Reasoning Drawer:** `< /> Ran reasoning steps (N steps)` accordion displaying harness tool execution logs.
  - **Citations Drawer:** `🔍 Live Retrieval & Citations (N sources)` showing clickable web links and snippets.

### 2. Interactive Choice Box (`src/components/InteractiveChoiceBox.tsx`)
- **Trigger:** Rendered automatically when the backend emits `ai-interaction-request` (from `AgentLoopOutcome::SuspendedForUserChoice`).
- **UI Structure:**
  - **Question Header:** Displays clean, unpacked question text asking the user to choose or specify scope.
  - **Radio Option List:** Renders up to 4 radio button option cards (`options: string[]`).
  - **Custom Response Input:** Provides a text area for write-in user responses (`"อื่นๆ (พิมพ์ระบุเอง...)"`).
  - **Action Buttons:** `Submit` and `Skip` controls.
- **IPC Resolution:** On submit, calls `invoke("generate_chat", { request: { session_id, interaction_id, interaction_option_id } })` to resume the agentic loop seamlessly.

### 3. Styling System (`src/components/InteractiveChoiceBox.module.css`)
- Custom Vanilla CSS Modules enforcing modern dark-mode aesthetic:
  - Gradient background surfaces (`#0f172a`, `#1e293b`).
  - Glowing active border accents (`#38bdf8`, `#10b981`).
  - Smooth micro-animations for hover states and selection transitions.
  - Full responsive layout scaling across desktop window dimensions.
