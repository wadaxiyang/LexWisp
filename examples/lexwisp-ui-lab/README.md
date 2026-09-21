# LexWisp UI Lab

This directory is a standalone GPUI-Kit workspace for designing and reviewing LexWisp UI examples. It is intentionally independent from the parent LexWisp product workspace: it has its own Cargo workspace boundary, dependency lock, theme, metrics, design contract, and agent instructions.

## Run the quick start

From this directory:

```powershell
cargo run --locked --bin quick_start
```

Or from anywhere:

```powershell
cargo run --locked --manifest-path C:\123\CODE\LexWisp\examples\lexwisp-ui-lab\Cargo.toml --bin quick_start
```

`quick_start` applies the local LexWisp semantic theme, follows the Windows system appearance, and includes one functional state transition so it can be used as a minimal verified starting point.

## Local sources of truth

- `AGENTS.md` — standalone instructions for a Codex session working only in this lab.
- `design-system/DESIGN.md` — normative visual and interaction contract copied from `LexWisp_Design_System.zip`.
- `theme.rs` — lab-local GPUI-Kit 0.6.1 light/dark theme implementation.
- `ui_metrics.rs` — lab-local Shell geometry, spacing, typography, and radius metrics.
- `quick_start.rs` — smallest runnable example.
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
