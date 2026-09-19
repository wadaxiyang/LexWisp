# LexWisp

LexWisp is a portable, native Windows text tool built with Rust, GPUI, and GPUI Kit. The current deliverable is **Stage 2: the first usable AI chat loop**.

Stage 2 adds an OpenAI-compatible provider, Windows Credential Manager integration, streaming Chat, stop/retry/copy, and local SQLite persistence while retaining the Stage 1 native shell and lifecycle behavior.

## Run

Extract `LexWisp-stage-02-windows-x64.zip` and run `LexWisp.exe`. Keep `vcruntime140.dll`, `portable.flag`, and the notice file beside it. The packaged `portable.flag` stores settings and `lexwisp.db` under the extracted `data` directory; removing the flag before first launch uses `%LOCALAPPDATA%\LexWisp` instead.

- First launch opens Control Center. Enter an OpenAI-compatible Base URL and model ID. If the endpoint uses Bearer authentication, enter an API key, choose **Test**, then **Save provider**. The key is stored only in Windows Credential Manager.
- Configure the shortcut/theme/startup/retention settings separately, then choose the System **Save** button.
- The default shortcut is `Ctrl + Alt + Space`. The notification-area icon opens Quick Shell on left click and provides Quick Shell, Settings, and Exit commands on right click.
- In Quick Shell, Enter sends and Shift+Enter inserts a newline. Streaming answers can be stopped, retried, selected, and copied. Hiding or closing Quick Shell does not stop an active Chat request; reopening shows the same in-memory conversation and result.
- Closing or hiding Quick Shell keeps its native window warm for the configured interval (30 seconds by default), then destroys it. Closing every GUI window does not exit the resident process.
- A second launch wakes the existing process instead of starting another copy.
- Exit explicitly from Quick Shell or the notification-area menu.

If a shortcut is already owned by another application, LexWisp keeps the previous working shortcut and reports the conflict. Settings are versioned TOML and are replaced atomically; content is checkpointed to SQLite. Corrupt or newer settings are preserved and reported rather than reset.

Stage 2 intentionally has one active conversation and no selection capture, translation/polish actions, multi-conversation history UI, or independent Chat Panel yet.

## Build

The development machine needs Rustup, Visual Studio C++ Build Tools, and the Windows SDK. `rust-toolchain.toml` pins Rust 1.95.0 and `Cargo.lock` freezes dependencies. End users do not need Rust, Node.js, or developer tools.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1
```

The QuickJS dependency probes remain test-only in `crates/lexwisp-app/tests/quickjs_probe.rs`; no VM or probe tool ships in Stage 2.

See [LexWisp_SPEC.md](LexWisp_SPEC.md), [LexWisp_SPEC_EXTEND.md](LexWisp_SPEC_EXTEND.md), and [docs/implementation-log.md](docs/implementation-log.md) for scope and verified evidence.
