# LexWisp

LexWisp is a portable, native Windows text tool built with Rust, GPUI, and GPUI Kit. The current deliverable is **Stage 4: multi-conversation Chat with an independent native panel**.

Stage 4 adds persistent conversation create/rename/switch/delete, per-conversation model preference, multi-turn context budgeting, retained regeneration attempts, and a large Chat Panel. Quick Shell and Chat Panel observe the same `ChatController`; moving to the panel attaches it before hiding the popup and never resends an active request. Stage 3's selection, Translate, Polish, copy/favorite, and safe replacement paths remain available.

## Run

Extract `LexWisp-stage-04-windows-x64.zip` and run `LexWisp.exe`. Keep `vcruntime140.dll`, `portable.flag`, and the notice file beside it. The packaged `portable.flag` stores settings and `lexwisp.db` under the extracted `data` directory; removing the flag before first launch uses `%LOCALAPPDATA%\LexWisp` instead.

- First launch opens Control Center. Enter an OpenAI-compatible Base URL and model ID. If the endpoint uses Bearer authentication, enter an API key, choose **Test**, then **Save provider**. The key is stored only in Windows Credential Manager.
- Configure the shortcut, launch mode, default action, dismiss overrides, theme, startup, and retention settings, then choose **Save**.
- The default shortcut is `Ctrl + Alt + Space`. The notification-area icon opens Quick Shell on left click and provides Quick Shell, Chat Panel, Settings, and Exit commands on right click.
- In Quick Shell Chat, choose **Open chat…** to hand the current conversation—including an in-flight answer—to the independent panel. Closing either window does not cancel Chat; reopening it reads the current controller snapshot.
- The Chat Panel provides virtualized conversation and message lists, New chat, rename/delete confirmation, per-conversation Fast/Smart/configured-model preference, Send/Stop/Regenerate, selectable Markdown, and copy controls for complete answers and individual fenced code blocks.
- Context is assembled only from the current conversation and the latest completed assistant attempt for each retained round. It drops whole oldest rounds to fit the selected Provider/model's estimated client budget, reports that pruning, and rejects a current message that cannot fit rather than truncating it.
- Select text in another application and press the shortcut. Verified UI Automation text can be routed through the action palette, directly to Translate, or to the configured default action. A copy-fallback result is shown as candidate text and is never sent until you explicitly confirm it.
- Translate exposes its target-language parameter. Polish exposes Fluent, Concise, and Academic styles. Both stream into the shared result surface and support Stop, Copy, Favorite, and—only for a still-verifiable original selection—**Replace original**.
- **Use clipboard text** is an explicit command available only to actions that declare clipboard input. LexWisp never silently uploads the previous clipboard after selection capture fails.
- Chat continues when Quick Shell is hidden. Translate and Polish cancel by default; Control Center can override each action to continue. Opening a menu, dialog, or IME candidate window is not treated as hiding the surface.
- Manual replacement revalidates the original process, window, UIA element, selected text, and 60-second token. If validation fails, the result is copied and the original application is not modified. Successful paste replacement intentionally leaves the result on the clipboard.
- Closing or hiding Quick Shell keeps its native window warm for the configured interval (30 seconds by default), then destroys it. Closing every GUI window does not exit the resident process.
- A second launch wakes the existing process instead of starting another copy.
- Exit explicitly from Quick Shell or the notification-area menu.

If a shortcut is already owned by another application, LexWisp keeps the previous working shortcut and reports the conflict. Settings are versioned TOML and are replaced atomically; content is checkpointed to SQLite. Corrupt or newer settings are preserved and reported rather than reset.

Stage 4 does not yet provide Stage 5's complete history/search/favorites management UI or privacy controls. Selection and replacement support depends on the target application's UI Automation and input-injection behavior and deliberately degrades to manual input/copy when it cannot be proven safe.

## Build

The development machine needs Rustup, Visual Studio C++ Build Tools, and the Windows SDK. `rust-toolchain.toml` pins Rust 1.95.0 and `Cargo.lock` freezes dependencies. End users do not need Rust, Node.js, or developer tools.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1
```

The QuickJS dependency probes remain test-only in `crates/lexwisp-app/tests/quickjs_probe.rs`; no VM or probe tool ships in Stage 4.

See [LexWisp_SPEC.md](LexWisp_SPEC.md), [LexWisp_SPEC_EXTEND.md](LexWisp_SPEC_EXTEND.md), and [docs/implementation-log.md](docs/implementation-log.md) for scope and verified evidence.
