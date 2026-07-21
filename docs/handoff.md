# AI Harness handoff

## Current objective

Continue improving the local AI Harness desktop application: a Tauri 2 + React/Vite app that downloads GGUF models, runs a local llama.cpp sidecar, and provides streaming chat. The immediate implementation focus is reliable, privacy-conscious temporal context for local LLM responses.

## Current state

- The app has a monochrome desktop shell with `File`, `Edit`, `View`, and `Help`, using the supplied `logo.svg`.
- Chat has session persistence in SQLite, Recents/search/rename/delete controls, streaming Markdown rendering, copyable highlighted code blocks, and manual-scroll behavior while generation is active.
- Session storage lives in the Tauri app data directory as `harness.db`; user and assistant messages are persisted before/after generation.
- Frontend notifications now appear centered below the desktop menu bar.
- Model/GPU details and acceleration controls were intentionally removed from the visible sidebar; engine selection remains backend-driven.

## Temporal-context implementation

Relevant specification: `C:\Users\Newsk\Downloads\llm-time-perception-spec.md`.

The current implementation is in `src-tauri/src/engine/time_manager.rs` and is wired through `src-tauri/src/state.rs`, `src-tauri/src/commands/engine.rs`, `src-tauri/src/commands/sessions.rs`, and `src-tauri/src/engine/context_manager.rs`.

- Every generation receives a structured temporal system message.
- New persisted messages use UTC ISO-8601 timestamps; old Unix-millisecond rows are still readable for gap detection.
- Gaps of one hour or more add invisible system notes; negative deltas are ignored.
- Memory compaction receives timestamps and is instructed to normalize relative wording to absolute dates/timestamps.
- `TimeAuthority` combines two sources:
  1. the OS clock, which is always available offline;
  2. network-derived IANA timezone from `https://ipwho.is/`, followed by current time for that timezone from `https://timeapi.io/api/time/current/zone`.
- The network calibration is cached for 15 minutes, retries use a five-minute backoff, and an old calibration may be used for up to 24 hours. Requests time out after three seconds. If either external source fails, the app uses the OS clock without blocking chat further.
- The app retains only the timezone identifier, calibrated time, and monotonic sync point in memory. It does not persist the public IP address, city, coordinates, or the full API responses.

## Verification performed

- `cargo test` from `src-tauri` passed with 8 tests.
- The tests cover repetition guards, session cascade migration, temporal gap markers, negative clock changes, API timestamp parsing, timezone request validation, and temporal-header source labeling.
- `npm.cmd run build` passed before the last Rust-only temporal changes.

## Open verification / next steps

1. Run the Tauri desktop app and send a message while online. Confirm the generated prompt reports the network-calibrated source instead of the OS fallback. Do not expose the system prompt or any location details in the UI.
2. Test offline mode or temporarily block the providers. The composer must remain responsive and chat must use the OS-clock fallback.
3. Confirm `timeapi.io` remains reachable within the three-second timeout from the user's normal network. A manual probe succeeded once but another request timed out; the fallback behavior is deliberate, but production telemetry or a configurable provider would improve observability.
4. If an explicit privacy control is desired later, add a setting that disables network time and clears the in-memory calibration. Do not silently store IP-derived data.
5. For a production release, consider replacing public best-effort providers with a user-configured or owned time service, while preserving the same OS fallback.

## Suggested skills

- `pichyycode` for any UI/settings work and build verification.
- `handoff` again when passing this project to another session.
- `skill-installer` only when adding another global Codex skill.

