# llama.cpp sidecar binaries

This directory is deliberately source-controlled without executables. Release automation will place the compiled `llama-server` binary for each supported Tauri target here:

- `llama-server-x86_64-pc-windows-msvc.exe`
- `llama-server-x86_64-apple-darwin`
- `llama-server-aarch64-apple-darwin`
- `llama-server-x86_64-unknown-linux-gnu`

`tauri.release.conf.json` adds `bin/llama-server` as `bundle.externalBin`; this keeps `cargo check` usable before release binaries are available, while Tauri resolves the target-triple suffix during `npm run tauri:bundle`. Do not commit downloaded GGUF models or locally compiled binaries.
