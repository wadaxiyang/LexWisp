# LexWisp

LexWisp is a portable, native Windows text tool built with Rust, GPUI, and Longbridge GPUI Kit. The current release is **v0.1.0**.

The current source tree provides the native Host, Chat plugin, declarative and Script plugin runtimes, storage, Windows integration, and management UI. Additional text behaviors are intentionally not bundled; they are installed later through the plugin framework. Long-running operation reclaims completed task scopes, shares one Host-owned HTTP pool, rejects stale window callbacks, detaches hidden projections after unexpected window closure, and opens new windows in the visible work area of the display under the pointer (including negative-coordinate layouts).

## Run

Extract `LexWisp-v0.1.0-windows-x64.zip` and run `LexWisp.exe`. Keep `vcruntime140.dll`, `portable.flag`, and the notice file beside it. The packaged `portable.flag` stores settings, `lexwisp.db`, and managed plugins under the extracted `data` directory; removing the flag before first launch uses `%LOCALAPPDATA%\LexWisp` instead.

- First launch opens Control Center. Enter an OpenAI-compatible Base URL and model ID. If the endpoint uses Bearer authentication, enter an API key, choose **Test**, then **Save provider**. The key is stored only in Windows Credential Manager.
- Configure the shortcut, theme, startup, and Main Shell retention settings, then choose **Save**. Existing text-action compatibility settings remain in Control Center for installed packages but are not promoted into the Main Shell.
- Open **Plugins** to choose or drop one local declarative or Script plugin directory/ZIP. Review its type, source, actions, SHA-256, version, and requested permissions, then explicitly confirm installation. Plugin-author schema, API/type documentation, and examples remain in the source repository rather than the runtime ZIP.
- Plugin Reload validates changed managed files and opens a fresh confirmation preview before switching generations. Disable and Uninstall revoke grants and cancel active work; Uninstall removes only LexWisp's managed copy and preserves the original source and saved history.
- The default shortcut is `Ctrl + Alt + Space`. The notification-area icon opens the Main Shell on left click and provides LexWisp, Settings, and Exit commands on right click.
- The Main Shell is one native window with three presentations: Compact is an input-first composer, the first Send expands that same window into a continuous transcript, and **Open workspace** reveals the conversation sidebar without creating a second Chat window. The same composer, draft, attachments, conversation, and in-flight request survive every presentation change.
- Workspace provides a virtualized conversation list, New chat, contextual rename/delete, Settings, and collapse back to Expanded. Chat retains per-conversation Fast/Smart/configured-model preference, Send/Stop/Regenerate, selectable Markdown, answer copy, and individual fenced-code copy controls.
- The composer accepts up to eight PNG/JPEG/WebP/GIF images (8 MiB each) or UTF-8 Markdown/text/code files (512 KiB each) through the file picker or its designated drop zone. Attachments have real previews/removal controls and are sent as in-memory image or text content; local paths are never sent to the Provider.
- Context is assembled only from the current conversation and the latest completed assistant attempt for each retained round. It drops whole oldest rounds to fit the selected Provider/model's estimated client budget, reports that pruning, and rejects a current message that cannot fit rather than truncating it.
- Select text in another application and press the shortcut. Only verified UI Automation selection appears as a removable attachment chip; no selection still opens an empty composer, and candidate clipboard text is never silently attached or sent.
- Chat continues when the Main Shell is hidden. Reopening it reads the same controller snapshot and returns to Compact without cancelling generation.
- Manual replacement revalidates the original process, window, UIA element, selected text, and 60-second token. If validation fails, the result is copied and the original application is not modified. Successful paste replacement intentionally leaves the result on the clipboard.
- Closing or hiding the Main Shell keeps its native window warm for the configured interval (30 seconds by default), then destroys it. Closing every GUI window does not exit the resident process.
- A second launch wakes the existing process instead of starting another copy.
- Exit explicitly from the notification-area menu.

If a shortcut is already owned by another application, LexWisp keeps the previous working shortcut and reports the conflict. Settings are versioned TOML and are replaced atomically; content is checkpointed to SQLite. Corrupt or newer settings are preserved and reported rather than reset.

The Control Center's History & favorites page supports text search, plugin/status/favorite filters, stable cursor paging, selectable authoritative bodies, copy, retry, annotation, explicit two-step delete, and clear-with-or-without-favorites. Privacy & diagnostics controls automatic recording, active config reload, consistent backup, and redacted diagnostics. Selection and replacement support still depends on the target application's UI Automation and input-injection behavior and deliberately degrades to manual input/copy when it cannot be proven safe.

## Build

The development machine needs Rustup, Visual Studio C++ Build Tools, and the Windows SDK. `rust-toolchain.toml` pins Rust 1.95.0 and `Cargo.lock` freezes dependencies. End users do not need Rust, Node.js, or developer tools.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1
```

QuickJS is embedded in `LexWisp.exe`; no Node.js, npm, browser runtime, developer tool, or probe binary is required on an end-user machine. Scripts import only relative package-local `.js` modules and reach Host resources only through explicitly granted finite APIs.

Pushing a `v*` tag whose version matches the workspace package version runs the Windows formatting, check, Clippy, test, Release build, and packaging gates, then publishes the ZIP and checksum as a GitHub Release. The portable ZIP contains only `LexWisp.exe`, `vcruntime140.dll`, `portable.flag`, `README.md`, and `THIRD-PARTY-NOTICES.txt`.

See [docs/design-system/DESIGN.md](docs/design-system/DESIGN.md) for the normative UI contract, [docs/implementation-log.md](docs/implementation-log.md) for verified implementation evidence, and [docs/plugin-schema.md](docs/plugin-schema.md) plus [docs/script-plugin-api.md](docs/script-plugin-api.md) for plugin contracts.
