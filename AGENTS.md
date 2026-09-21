# LexWisp — Agent Instructions

## Product boundary

LexWisp is a portable native Windows x64 floating AI chat client. It has one popup window, a global hotkey, a tray icon, persistent conversations, and configurable OpenAI-compatible providers. Chat is a compiled first-party capability. There are no installable runtime plugins, selection-first actions, action palettes, source-text replacement, or secondary workspace window.

Build only requested behavior and real dependencies. Do not introduce speculative plugin, tool, Agent, MCP, workflow, registry, or generic command frameworks. A future built-in Agent tool needs its own concrete requirement. Preserve working Chat, settings, Windows integration, and user data.

## Independent UI lab

`examples/lexwisp-ui-lab/` is an independent reference project. Main product work may read and adapt it outside that directory. Do not modify, reformat, regenerate, or add a product dependency on anything under the lab unless the user explicitly requests changes to the lab itself.

## Ownership and dependencies

| Crate | Responsibility | Allowed project dependencies |
| --- | --- | --- |
| core | IDs, DTOs, rules, state machines, typed ports | none |
| platform-windows | Win32, tray, hotkey, credentials, native window functions | core |
| storage | versioned TOML, SQLite, migrations, consistent backup | core |
| host | ChatController, run supervision, providers, tasks, shared services | core, storage, platform-windows |
| ui | GPUI popup, Chat and settings views, transient view state | core |
| app | composition root, startup, resources, packaging | assembly dependencies |

Core must not import GPUI, Win32, HTTP, or SQLite. UI must not depend on Host implementations. Host owns one business Tokio runtime, reused HTTP clients, credentials, storage, task scopes, and run supervision. `HostHandles` is a typed set of initialized services, not a service locator. Keep typed ports; never add a global `Arc<Mutex<AppState>>`, reflection bus, or universal JSON invoke function.

ChatController owns conversations and stable conversation/message IDs. ExecutionStore owns live run state. Storage owns persistence. Views own cursors, selection, scrolling, and other transient UI state. Preserve one authoritative message body and observe the same ChatController when the popup is hidden or reopened.

## UI discipline

- `docs/design-system/DESIGN.md` is the normative visual and interaction contract. Before UI work, read it, `crates/lexwisp-ui/src/theme.rs`, `crates/lexwisp-ui/src/ui_metrics.rs`, Cargo.lock, workspace dependencies, and the relevant UI lab reference.
- Longbridge GPUI Kit is mandatory. Before designing, reviewing, or changing UI, use the installed `gpui-kit` and `gpui-kit-design-guides` skills, read their required guides, and verify APIs against locked Kit source and matching examples. If either skill is unavailable, stop UI changes and report the blocker.
- Import the GPUI family through `gpui_kit`. Reuse Kit inputs, buttons, selection, dialogs, themes, scrolling, lists, and `Root`. Do not copy upstream components or invent a general component library.
- Raw product colors belong only in `theme.rs`. Views use semantic theme roles, `ui_metrics`, a 4 px spacing rhythm, restrained radii, and structural borders. The popup is the only native product window; settings and history appear inside it.
- Initialize Kit once. The popup has one top-level Kit Root and shared overlay setup. Window creation contains no business logic. Use explicit quit behavior: closing or hiding the popup leaves tray/hotkey access; Quit performs bounded shutdown. Never use a hidden GUI keeper window.
- Create InputState, focus/scroll handles, subscriptions, and stateful entities once in their owner. Render describes UI; it performs no I/O, subscriptions, tasks, or random ID creation. Use stable IDs and intent callbacks. Keep GPUI values on their owning thread, and validate window/request generations in background UI updates.
- Coalesce streaming updates around 33 ms and flush terminal states immediately. Keep text selectable, cache Markdown by version where practical, and virtualize or page large lists. Scrolling up stops auto-follow.
- IME candidate confirmation must not send. Shift+Enter inserts a newline. Escape respects overlays before hiding the popup. Menus, dialogs, and IME focus changes are not dismissal.
- Only the designated composer accepts file drops. Inspect metadata and enforce attachment size/type limits before reading. Never send local file paths to a provider.

## Execution, storage, and Windows

- Tasks have an owner, scope, cancellation, and retained handle. No unowned spawn/detach. Blocking SQLite work belongs on its worker. Aborting a UI task cannot be assumed to stop already-running work.
- RunSupervisor commits exactly one terminal state. Reject stale and post-terminal deltas. Use bounded queues and body limits; lagging subscribers recover from versioned snapshots. Cancellation preserves partial output.
- ExecutionStore feeds UI and persistence independently. Checkpoint around 500 ms/16 KiB and immediately at terminal state. Enforce sequence/retention guards and deletion barriers. Storage failure leaves copyable output visible and reports it as unsaved.
- OpenAI-compatible requests use configured providers and profiles. Preserve Base URL prefixes, attach credentials per request, parse SSE across UTF-8/JSON/chunk boundaries, and report truncated streams while keeping partial text. Do not add paid POST retries, disable TLS checks, or forward credentials across origins.
- Versioned TOML is authoritative for settings; SQLite stores content. Validate and save atomically. Preserve corrupt or newer data rather than resetting it. Back up SQLite consistently with WAL. API keys belong only in Windows Credential Manager; diagnostics contain no keys or message bodies.
- Paths are independent of cwd and support Chinese characters and spaces. Default data is `%LOCALAPPDATA%/LexWisp`; `portable.flag` selects executable-adjacent data. Credentials do not migrate with the data directory.
- Encapsulate unsafe Windows code with ownership/thread/release comments. Centralize DPI/work-area conversion, including negative coordinates. Register a replacement hotkey before releasing the previous one and restore tray state after Explorer restarts. The hotkey only toggles the popup; it never captures selected text.

## Quality and delivery

Read and reuse existing code first. Prefer concrete types, enums, guard clauses, and small meaningful ports. Avoid forwarding layers, unrelated refactors, and error swallowing. Do not unwrap fallible user, network, file, database, or system operations. Startup failures need visible diagnostics and controlled exit.

Test real risks and regressions, not every getter/label or upstream library behavior. Keep test-only features in matching dev-dependencies; avoid `--all-features`. Never use live keys in automatic tests. Run:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
```

Launch Release from the actual staged/extracted directory. `scripts/package.ps1` must package only explicit runtime files and required licenses, never developer data, logs, caches, or keys. Keep panic-unwind semantics unless justified otherwise.

Record actual commands, platform/hardware/driver/DPI, artifact paths, Release size, and available measurements in `docs/implementation-log.md`. Distinguish compile, launch, functional acceptance, and performance acceptance. Measure working set, Private Bytes, handles/threads, and GPU memory separately; never trim working sets to hide growth. Do not fabricate screenshots or results. Mark unavailable native, IME, service, and clean-machine checks pending.
