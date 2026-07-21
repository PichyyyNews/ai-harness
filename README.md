<div align="center">

<img src="./logo.svg" width="96" alt="AI Harness logo" />

# AI Harness

### A private, local-first desktop workspace for open GGUF models.

Search, install, and chat with local models through a focused desktop experience built with Tauri, Rust, and React.

[Features](#features) · [Quick start](#quick-start) · [Architecture](#architecture) · [Privacy](#privacy)

</div>

---

## Why AI Harness?

AI Harness keeps the model and conversation on your computer while making local inference feel like a polished desktop tool. Choose a GGUF model from Hugging Face, download it with checksum verification, then chat through a local `llama-server` engine with streaming output, Markdown rendering, and durable conversation history.

## Features

- **Model discovery and install** — search the Hugging Face GGUF catalog, choose a file, download in chunks, and verify SHA-256 before it becomes available.
- **Local inference** — launches a managed llama.cpp sidecar and waits for its health endpoint before enabling chat.
- **Adaptive acceleration** — chooses the safest available backend and GPU offload configuration, with reliable CPU fallback.
- **Streaming-first chat** — token streaming, stop generation, repetition protection, context management, and automatic bounded continuation for long answers.
- **Readable Markdown** — real-time Markdown, GFM tables, syntax highlighting, and one-click code copying.
- **Persistent sessions** — SQLite-backed conversations with recents, search, rename, delete, automatic titles, and restored memory.
- **Respectful scrolling** — new tokens never pull a reader away from older text; jump to the latest response only when requested.
- **Temporal awareness** — combines OS time with cached network timezone calibration, handles offline fallback, and adds meaningful conversation-gap context.
- **Desktop-native shell** — a compact `File` / `Edit` / `View` / `Help` menu, custom window controls, and a restrained monochrome interface.

## Quick start

### Prerequisites

- Node.js 22+
- Rust stable with the Windows MSVC toolchain
- A supported local GPU is optional; the CPU backend remains available

### Develop

```powershell
npm install
npm run tauri dev
```

The app opens a model picker on first launch. Install a GGUF model, wait for checksum verification, start the local engine, and begin chatting.

### Frontend only

```powershell
npm run dev
```

### Validate and package

```powershell
npm run build

Set-Location src-tauri
cargo test
Set-Location ..

npm run tauri:bundle
```

## Architecture

```text
React + Vite desktop UI
        │ Tauri commands/events
        ▼
Rust application core
  ├─ model catalog, download, checksum, installed-model registry
  ├─ session store (SQLite) and context/memory management
  ├─ temporal authority (system clock + network timezone calibration)
  └─ engine/runtime manager
        │ local HTTP + SSE
        ▼
Managed llama.cpp / llama-server sidecar
        │
        ▼
Local GGUF model
```

## Project structure

```text
src/                     React UI, components, styles, chat client
src-tauri/src/
  commands/              Tauri command boundary
  engine/                runtime, context, repetition, time, hardware logic
  models/                Hugging Face and local-model management
  sessions/              SQLite-backed conversation persistence
docs/                    engineering notes and handoffs
scripts/                 local sidecar preparation
```

## Privacy

- Model files, prompts, responses, session history, and conversation memory stay on the local machine.
- The managed inference engine listens only on localhost.
- The temporal authority uses the OS clock offline. When available, it makes short-lived requests to resolve an IANA timezone from the network and obtain the current time for that timezone. It does **not** store IP addresses, city names, coordinates, or full provider responses.
- Downloading models and refreshing managed runtimes naturally requires network access.

## Development notes

- Downloaded GGUF files, build output, local environment files, and runtime sidecars are intentionally ignored by Git.
- The backend is modular by design; keep command handlers thin and place behavior in `engine`, `models`, or `sessions` modules.
- `docs/handoff.md` records the current engineering handoff and outstanding runtime verification.

## Status

AI Harness is actively evolving. Core model installation, local streaming chat, session persistence, adaptive runtime selection, and temporal context are implemented. Release packaging and wider platform validation are the next areas to harden.
