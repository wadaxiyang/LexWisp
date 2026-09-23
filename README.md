# LexWisp

LexWisp is a portable native Windows x64 floating AI chat client built with Rust, GPUI, and Longbridge GPUI Kit. Chat, settings, and conversation history live in one popup. The app remains available from its global hotkey and notification-area icon when the popup is hidden.

## Run

Extract `LexWisp-v0.1.1-windows-x64.zip` and run `LexWisp.exe`. Keep `vcruntime140.dll`, `portable.flag`, and `THIRD-PARTY-NOTICES.txt` beside it. With `portable.flag`, settings and `lexwisp.db` are saved in an adjacent `data` directory. Remove the flag before first launch to use `%LOCALAPPDATA%\LexWisp`. API keys stay in Windows Credential Manager in either mode.

Launch starts LexWisp in the notification area without opening a window. Press the global hotkey or left-click the notification-area icon to open Chat. Open Settings from the icon's right-click menu, then enter an OpenAI-compatible Base URL and model ID. If the endpoint requires Bearer authentication, enter an API key, select **Test**, then **Save provider**. Configure the hotkey, theme, startup, local recording, and hidden-window retention in the same Settings page.

- The default global hotkey is `Ctrl + Alt + Space`; it toggles the popup directly. No selected text is required or captured.
- The notification-area icon opens Chat on left click. Its right-click menu offers Chat, Settings, About LexWisp, and Exit. A second launch leaves the existing process in its current state.
- The popup follows the layout in `examples/lexwisp-ui-lab/src/lexwisp_main_window.rs`: a quiet header, conversation switcher, transcript, and contained composer. History and Settings are pages within the same popup. The popup has no workspace or second native panel.
- Use **New chat** or `Ctrl+N` to start a conversation. `Ctrl+K` opens the switcher; `Ctrl+H` opens searchable conversation history. History can filter favorites. Conversations can be renamed or deleted with confirmation.
- Enter sends; Shift+Enter inserts a newline. The composer supports Send, Stop, Retry, and Fast/Smart model preferences. Assistant Markdown is selectable; answers and fenced code blocks have copy controls.
- The composer accepts up to eight PNG/JPEG/WebP/GIF images (8 MiB each) or UTF-8 Markdown/text/code files (512 KiB each) through the file picker or composer drop zone. Files are sent as content, never as local paths.
- Chat keeps streaming while the popup is hidden. Reopening shows the same conversation and request. Closing the popup hides it for the configured retention period; Exit in the tray menu shuts down the process.
- Submitted inputs, completed answers, and partial results are saved locally by default. Settings offers recording opt-out; individual conversations can be deleted. SQLite backup is available in Settings.

If a hotkey conflicts with another application, LexWisp keeps the previous working shortcut and reports the conflict. Settings are versioned TOML and saved atomically. Corrupt or newer settings are preserved and reported. Provider requests preserve Base URL prefixes and keep partial streaming text visible when interrupted.

## Build

Development requires Rustup, Visual Studio C++ Build Tools, and the Windows SDK. `rust-toolchain.toml` pins Rust 1.95.0. End users do not need Rust, Node.js, or developer tools.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1 -SkipBuild
```

The portable ZIP contains only `LexWisp.exe`, `vcruntime140.dll`, `portable.flag`, `README.md`, and `THIRD-PARTY-NOTICES.txt`. See [the design contract](docs/design-system/DESIGN.md) and [implementation log](docs/implementation-log.md).
