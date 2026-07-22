# AI Harness handoff — web-first grounding and dynamic context

## Purpose of this handoff

Continue the AI Harness work on making a small local model rely on fresh web evidence for ordinary factual questions, retrieve corroborating sources, and use the available context window efficiently. This document supplements the earlier temporal-context handoff in [`docs/handoff.md`](./handoff.md); it does not repeat that implementation.

## Workspace and scope

- Project: Tauri 2 + React/Vite desktop app in `C:\Users\Newsk\Downloads\Aphelion`.
- Local inference uses the bundled `llama-server` sidecar and streams `/v1/chat/completions`.
- The repository is private on GitHub as configured by the existing `origin` remote. Do not add credentials or API keys to the repository.
- At the time of this handoff, the feature work is intentionally uncommitted and should be reviewed before further publishing.

## Implemented in this work

### Search-first routing

`src-tauri/src/web_search/query.rs` now routes ordinary non-trivial prompts to web retrieval by default. It skips only short acknowledgements, clearly self-contained transformations, and explicit offline/no-search requests. This avoids making a second local-model classification pass.

### Multi-source retrieval

`src-tauri/src/web_search/manager.rs` now:

- deduplicates URLs and prefers distinct hosts;
- reads up to six result pages concurrently;
- asks the ranking layer for a context budget supplied by the context manager;
- keeps grounding instructions before source text so emergency truncation preserves them;
- exposes source URLs/titles for citations and persisted session metadata.

`searxng.rs` and `duckduckgo.rs` accept up to eight search results. `bm25.rs` lowers the relevance floor slightly, uses a no-whitespace-language fallback, and makes a first pass that favors distinct source documents before adding depth.

### Dynamic context allocation

`src-tauri/src/engine/context_manager.rs` now calculates a per-request budget from the active engine context size. It reserves safety space and a response floor, gives web evidence a proportional slice, then uses remaining capacity for recent history. Memory and web blocks are shrunk progressively instead of deleting the entire evidence block at the first overflow.

`src-tauri/src/engine/hardware.rs` probes available Windows physical memory best-effort. `src-tauri/src/engine/runtime.rs` selects a conservative 4K/8K/16K context tier based on RAM, free VRAM, and GGUF size, while retaining llama-server `--fit` as the final allocation guard. The engine reports the selected context size to the frontend.

## Verification already completed

- `cargo test` from `src-tauri`: 11 tests passed.
- `cargo check` from `src-tauri`: passed.
- `npm.cmd run build`: passed (`tsc -b` and Vite production build).
- Formatting and `git diff --check` were run on the touched Rust paths.

## Important runtime validation still needed

1. Restart the local engine/app so the adaptive `--ctx-size` and `--fit-ctx` arguments are applied.
2. Send a normal factual question and verify the status sequence reports web searching, multiple-source reading, ranking, then generation.
3. Confirm the assistant response contains source markers and that the UI displays the returned source list without exposing hidden prompt text.
4. Test a short acknowledgement, a self-contained rewrite, and an explicit offline request to confirm they do not trigger network retrieval.
5. Test a long conversation with web grounding. Confirm the newest user turn remains present and that scrolling/streaming stays responsive.
6. Test a GPU-constrained machine or occupied VRAM. Confirm `--fit` or the existing CPU fallback handles the selected context tier without falsely claiming CUDA acceleration.

## Suggested skills

- `pichyycode` for any UI changes, integration review, or end-to-end build checks.
- `handoff` when creating the next session summary.
- `github:yeet` for a future intentional publish/PR workflow; review this feature and run the runtime checks before using it.
- `github:github` for repository/issue context if remote tracking is needed.

## Safe next change candidates

- Add a small source-diversity test fixture with three hosts and irrelevant results.
- Add configurable search-provider timeouts and an opt-out privacy setting.
- Add telemetry-free debug status counters for cache hit, provider fallback, source count, and final context tier.
- Query the running llama-server for its effective context if the sidecar exposes that metadata, so the UI can distinguish requested from accepted context size.
