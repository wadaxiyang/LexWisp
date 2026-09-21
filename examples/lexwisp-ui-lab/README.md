# LexWisp UI Lab

This directory is a standalone GPUI-Kit workspace for designing and reviewing LexWisp UI examples. It is intentionally independent from the parent LexWisp product workspace: it has its own Cargo workspace boundary, dependency lock, theme, metrics, design contract, and agent instructions.

## Run the UI examples

From this directory:

```powershell
cargo run --locked --bin chart_main_window
cargo run --locked --bin lexwisp_main_window
```

Or from anywhere:

```powershell
cargo run --locked --manifest-path C:\123\CODE\LexWisp\examples\lexwisp-ui-lab\Cargo.toml --bin chart_main_window
cargo run --locked --manifest-path C:\123\CODE\LexWisp\examples\lexwisp-ui-lab\Cargo.toml --bin lexwisp_main_window
```

`chart_main_window` applies the local LexWisp semantic theme and follows the Windows system appearance. Its client-drawn global title bar and collapsible sidebar share a translucent navigation material over GPUI's Windows 11 Mica backdrop; the conversation workspace stays opaque for readability and owns a compact secondary title bar with the optional workspace-context panel control. The local conversation flow and retained Composer remain deterministic and functional.

`lexwisp_main_window` is the single Conversation-first Chat window, developed toward the PopChat-inspired interaction in the earlier `LexWisp_POPUP_FIRST_CHAT_UI_SPEC.md`. The former Compact/Prompt presentation and expansion animation have been removed. This is now the one page around which future Chat features in the lab should be built. It retains one Composer and conversation state, a local preview stream, in-window switcher and history, search, pin state, file selection, and model/web UI states. It has no sidebar or native caption buttons. This standalone lab has no provider, persistence, or OS-global hotkey integration. Hide minimizes the window so it remains recoverable from the taskbar; the product host will later own true hide/show, global summoning, and pinned window policy.

For appearance review, run `cargo run --locked --bin lexwisp_main_window -- --light` or `cargo run --locked --bin lexwisp_main_window -- --dark`; without a flag it follows Windows appearance.

## Local sources of truth

- `AGENTS.md` — standalone instructions for a Codex session working only in this lab.
- `design-system/DESIGN.md` — normative visual and interaction contract copied from `LexWisp_Design_System.zip`.
- `src/theme.rs` — lab-local GPUI-Kit 0.6.1 light/dark theme implementation.
- `src/ui_metrics.rs` — lab-local Shell geometry, spacing, typography, and radius metrics.
- `src/chart_main_window.rs` — runnable ChatGPT-inspired main-window example.
- `src/lexwisp_main_window.rs` — runnable, single-surface Chat main window.
- `rust-toolchain.toml` and `Cargo.lock` — lab-local Rust/toolchain and dependency freeze.

## Add another example

Keep examples independently runnable. Add a new `[[bin]]` entry to `Cargo.toml` with a distinct source file, reuse the local `theme` and `ui_metrics` modules where appropriate, and avoid depending on any parent-workspace crate.

Before sharing an example, run:

```powershell
cargo fmt --all -- --check
cargo check --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```
