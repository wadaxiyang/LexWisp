# LexWisp

LexWisp is a portable, native Windows text-tool shell built with Rust, GPUI, and GPUI Kit. The current deliverable is **Stage 1: Host core, system residency, and window lifecycle**.

Stage 1 deliberately contains no AI actions. It establishes the single-instance Host, notification-area icon, global shortcut, retained task ownership, native window registry, and persistent shell settings required by later stages.

## Run

Extract `LexWisp-stage-01-windows-x64.zip` and run `LexWisp.exe`. Keep `vcruntime140.dll`, `portable.flag`, and the notice file beside it. The packaged `portable.flag` stores `settings.toml` under the extracted `data` directory; removing the flag before first launch uses `%LOCALAPPDATA%\LexWisp` instead.

- First launch opens Control Center. Choose a global shortcut, theme, launch-at-sign-in behavior, and Quick Shell retention, then choose **Save**.
- The default shortcut is `Ctrl + Alt + Space`. The notification-area icon opens Quick Shell on left click and provides Quick Shell, Settings, and Exit commands on right click.
- Closing or hiding Quick Shell keeps its native window warm for the configured interval (30 seconds by default), then destroys it. Closing every GUI window does not exit the resident process.
- A second launch wakes the existing process instead of starting another copy.
- Exit explicitly from Quick Shell or the notification-area menu.

If a shortcut is already owned by another application, LexWisp keeps the previous working shortcut and reports the conflict. Settings are versioned TOML and are replaced atomically; corrupt or newer settings are preserved and reported rather than reset.

## Build

The development machine needs Rustup, Visual Studio C++ Build Tools, and the Windows SDK. `rust-toolchain.toml` pins Rust 1.95.0 and `Cargo.lock` freezes dependencies. End users do not need Rust, Node.js, or developer tools.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1
```

The QuickJS dependency probes remain test-only in `crates/lexwisp-app/tests/quickjs_probe.rs`; no VM or probe tool ships in Stage 1.

See [LexWisp_SPEC.md](LexWisp_SPEC.md), [LexWisp_SPEC_EXTEND.md](LexWisp_SPEC_EXTEND.md), and [docs/implementation-log.md](docs/implementation-log.md) for scope and verified evidence.
