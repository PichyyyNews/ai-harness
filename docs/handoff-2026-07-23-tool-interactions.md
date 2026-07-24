# Tool Interaction Pipeline Handoff — 2026-07-23

## Outcome

The chat pipeline no longer parses assistant prose or `<<TOOL: ...>>` markers to drive tools or Choices. Native-tool routing happens before synthesis, and a Choice is now a persisted interaction transaction rather than a regular user message.

This work extends the architecture in [plan-03-dynamic-ai-pipeline.md](plan-03-dynamic-ai-pipeline.md) and [plan-04-harness-tools-expansion.md](plan-04-harness-tools-expansion.md); do not duplicate those broader plans.

## Implemented flow

```text
user request
  -> typed local-model router (answer | call_tool | ask_user_choice)
  -> host executes native tool, or persists native interaction
  -> retrieval only when no native tool was selected
  -> answer synthesis with tool/evidence context
  -> deterministic frontend renderer
```

- `src-tauri/src/tools/planner.rs` asks the local model for a schema-validated route.
- `src-tauri/src/commands/engine.rs` executes a selected native tool before answer generation.
- A native-tool result suppresses generic web retrieval for that turn. This prevents a local request such as GPU/VRAM status from being diluted by unrelated web results.
- `ask_user_choice` is controller-owned. It cannot be emitted by generated answer text.
- The post-generation `review_draft` interception was removed. It was responsible for replacing streamed content and causing visible jumps.

## Choice transaction contract

Persistence is in `pending_interactions` in the existing SQLite database.

1. Router returns `ask_user_choice` only when an actual user decision is required.
2. Backend stores a pending interaction with `id`, session ID, original request, question, option IDs/labels, and status.
3. Backend emits `ai-interaction-request` with `{ id, question, options: [{ id, label }] }`.
4. The UI renders `InteractiveChoiceBox` from that typed payload.
5. Selecting an option sends `interactionId + interactionOptionId`, not a tool marker and not a model-created command.
6. Backend validates and resolves the pending record, injects the original request plus selected option as private continuation context, and skips Choice routing for that continuation.

Consequences:

- A selection may appear as a user bubble because it is a genuine user action.
- AI text must never appear in a user bubble.
- An old or mismatched option ID is rejected instead of being interpreted as a new prompt.
- Pending interactions survive renderer restart because they are stored in SQLite. Restoring an already-pending card after an app restart is not implemented yet.

## Files changed

| File | Purpose |
| --- | --- |
| `src-tauri/src/commands/engine.rs` | Typed routing, tool/retrieval precedence, Choice continuation injection. |
| `src-tauri/src/tools/planner.rs` | Local-model JSON router and capability registry. |
| `src-tauri/src/tools/mod.rs` | Removes tool-owned Choice event emission; answer-synthesis policy. |
| `src-tauri/src/sessions/store.rs` | SQLite `pending_interactions`, create/resolve methods. |
| `src-tauri/src/sessions/types.rs` | Typed interaction/option payloads. |
| `src-tauri/src/engine/runtime.rs` | Request fields for interaction and option IDs. |
| `src-tauri/src/engine/context_manager.rs` | Updates internal `ChatRequest` constructors. |
| `src/App.tsx` | Listens for typed interaction events and submits option IDs. |
| `src/lib/local-chat.ts` | Sends interaction IDs to Tauri. |
| `src/components/InteractiveChoiceBox.tsx` | Renders `{ id, label }` options; custom free text is hidden for required choices. |

## Live verification performed

The rebuilt debug app was started from `src-tauri/target/debug/ai-harness.exe` using the installed local model `gemma-4-E4B-it-Q4_K_M.gguf`.

1. Native tool test — Thai prompt asking for current GPU and VRAM status.
   - UI displayed `Ran a tool command`.
   - Answer contained live local values including NVIDIA GeForce RTX 3070 and current VRAM values.
   - First run also launched web retrieval. The precedence fix was added, rebuilt, and rerun.
   - Second run displayed only the native-tool stage (3 steps), with no Live Retrieval panel. Retrieval debug log showed only pre-route Tier 0 entries for that turn and no later `routing`/`retrieval` event.
2. Choice test — Thai request to plan deployment but require the user to choose an environment.
   - Router rendered a native Choice card with `development`, `staging`, and `production`.
   - An option was selected; the Choice card disappeared and the model resumed with a deployment plan. It did not request another Choice.
   - The Windows automation selected `development` during this run, despite the intended `staging`; this does not affect the verified ID-based resume behavior. Retest option-to-label precision manually if changing Choice UI automation or DOM structure.

## Verification commands

```powershell
npm.cmd run build
cargo test --manifest-path src-tauri\Cargo.toml --lib
cargo build --manifest-path src-tauri\Cargo.toml
git diff --check
```

Latest results: frontend production build passed; Rust library tests passed (56 tests); debug build passed before the final live sessions. Existing warnings remain for unused provider-health code and the future-incompatible `nom v1.2.4` dependency.

## Current Git state

The implementation is committed in `51c24e1` (`Persist native interactions and resolve user selections`), with the preceding route-first work in `461af37` (`Route native tools and choices before generation`). This handoff document is currently the only uncommitted workspace file.

The implementation spans:

```text
src-tauri/src/commands/engine.rs
src-tauri/src/engine/context_manager.rs
src-tauri/src/engine/runtime.rs
src-tauri/src/sessions/mod.rs
src-tauri/src/sessions/store.rs
src-tauri/src/sessions/types.rs
src-tauri/src/tools/mod.rs
src-tauri/src/tools/planner.rs
src/App.tsx
src/components/InteractiveChoiceBox.tsx
src/lib/local-chat.ts
```

## Follow-up priorities

1. Add explicit router telemetry: route selected, model/parse failure, tool execution result, and interaction ID. Current `eprintln!` fallback can make router failure look like an ordinary Answer.
2. Restore a pending native Choice card when reopening the corresponding session after an app restart.
3. Add an end-to-end regression test that verifies selecting each option preserves its exact label/ID.
4. Keep broad/current questions on retrieval + direct answer. Do not introduce topic-based hardcoded Choice triggers.
5. If extending controller actions, keep the same typed contract (`final`, `run_tools`, `request_input`, `revise`) and cap tool iterations; never reintroduce prose parsing.

## Suggested skills

- `computer-use:computer-use` for live Tauri acceptance testing.
- `aphelion-dev` only if the work moves to the separate `D:\Aphelion` Docker project; it does not apply to this checkout.
- `$handoff` when handing further work to a new agent.
