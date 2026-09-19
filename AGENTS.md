# LexWisp — Agent Instructions

## Scope and source of truth

Read `LexWisp_SPEC.md` and `LexWisp_SPEC_EXTEND.md` before changing the relevant stage. The extension refers to `LexWisp_IMPLEMENTATION_SPEC.md`; in this repository that means `LexWisp_SPEC.md`. Precedence: explicit user decisions, base SPEC, extension, local implementation choices. Keep stage evidence in `docs/implementation-log.md`.

Build only the requested stage and real dependencies. Do not generate future crates, empty services, placeholder actions, or clickable no-op UI. Stage 0 is a native dependency/input probe, not an AI product. Preserve existing working behavior and user changes.

## Product boundaries

- Windows x64 portable native app: Rust + GPUI + GPUI-Kit + windows-rs; one normal `LexWisp.exe` process, Host + plugins.
- No Electron, Tauri, WebView UI, GUI sidecar, per-plugin process, online marketplace, or extra Provider protocols without a new requirement.
- Chat is a trusted, compiled Native plugin. Translate, Polish, and custom prompts are Declarative plugins; third-party multi-step logic uses Script plugins. All share registration, authorization, Invocation supervision, and results.
- Support all three configurable shortcut modes. No selection means manual input, never silent clipboard upload.
- Chat continues when dismissed; Translate/Polish cancel by default, with user overrides. Popup-to-panel handoff attaches the destination before detaching the source and preserves the same conversation/request.
- Save submitted inputs, results, and partial results locally by default; support opt-out and deletion. Replacing source text always requires a user click and fresh target validation.
- Daily settings need UI; plugin authors edit manifest/prompt/JS files. Distribution must eventually run without Rust, Node, or developer tools.

## Ownership and dependencies

| Crate | Responsibility | Allowed project dependencies |
| --- | --- | --- |
| core | IDs, DTOs, rules, state machines, necessary ports | none |
| platform-windows | Win32/COM, tray, hotkeys, UIA, clipboard, credentials | core |
| storage | Config, SQLite, migrations, consistent backup | core |
| host | Registration, grants, supervision, shared services, AI, declarative execution | core, storage, platform-windows |
| ui | GPUI shell, UI contracts, product components, projections | core |
| plugins-builtin | Chat state, actions, views, lifecycle | core, ui |
| plugins-script | JS loading, VM, controlled Host bridge | core |
| app | Composition root, startup, resources, packaging | assembly dependencies |

Names may differ when responsibilities remain clear. Add crates only for real boundaries. Core must not import GPUI, Win32, HTTP, SQLite, or concrete plugins. Host must not depend on concrete plugins or branch on plugin IDs for business behavior. UI must not depend on Host implementations.

Host owns shared resources: one business Tokio Runtime, reused HTTP clients, storage, credentials, grants, tasks, and window routing. Plugins own business semantics and surface content; Host owns windows. Never duplicate these resources per plugin/request or use a global `Arc<Mutex<AppState>>`.

`HostHandles` is a typed, cheaply cloned set of initialized internal services, not a service locator. Plugins receive identity-bound scoped ports, never the full handles. No fallback Runtime creation, global EventBus, reflection bus, or universal `invoke(name, json)` escape hatch.

ExecutionStore owns live execution state; ChatController owns conversation semantics; storage owns persistence; Views own transient cursor/selection/scroll state. Establish ChatController and stable conversation/message IDs in Stage 2. Both Chat surfaces observe it. Chat bodies have one persistent source, referenced by history/results.

## UI discipline

- Before UI work, read Cargo.lock, workspace dependencies, locked Kit source and matching examples. Import the GPUI family through `gpui_kit`; do not mix sources/versions. Freeze working dependencies; pin Git patches to full commits and document removal conditions.
- Reuse Kit inputs, buttons, selection, dialogs, themes, scrolling and lists. Do not build a second general component library or copy upstream implementations.
- Initialize Kit once. Each real window gets one top-level Kit Root and shared overlay setup. Stage 0 uses `build_window_options`, `LexWispWindowRoot`, and `open_surface_window`; Stage 1 adds registry/lifecycle/bridge. Window creation contains no business logic.
- Use explicit quit behavior. Stage 0 must exit cleanly on closing its last window; Stage 1 must remain usable through tray/hotkeys with no GUI windows. Never keep a hidden GUI window solely to hold the process alive.
- Create InputState, focus/scroll handles, subscriptions, and stateful Entities once in their owner. Render only describes UI: no I/O, scripts, subscriptions, task creation, random IDs, or unconditional self-notify.
- Use stable object IDs, one state owner, and intent callbacks. Avoid feedback loops and cloning whole histories. Preserve subscriptions and release them with their owner.
- Keep GPUI/COM values on their owning threads. Never retain UI borrows across await or add unsafe Send/Sync. Background UI updates use weak references and validate window/plugin/request generations.
- Coalesce streaming UI updates around 33 ms; flush terminal states immediately. Cache Markdown by version, provide real selectable text, paginate and virtualize long lists, and stop hidden-window refresh work. User scrolling up stops auto-follow.
- IME candidate confirmation must not send. Shift+Enter inserts a newline; Escape respects overlay hierarchy. Menus/dialogs/IME focus changes are not dismissal.
- Only designated import surfaces accept drops. Drag inspection is metadata-only; import preview never executes scripts.

## Execution, storage, and security

- Tasks have an owner, scope, cancellation and retained handle. No unowned spawn/detach. Blocking UIA, SQLite and JS belong in their dedicated execution domains; abort cannot stop already-running blocking work.
- Supervisor commits exactly one terminal state. Reject stale generations and post-terminal deltas. Use bounded queues and body limits; lagging subscribers recover from versioned snapshots. Cancellation preserves partial results.
- ExecutionStore feeds independent UI and persistence projections. Checkpoint around 500 ms/16 KiB and immediately on terminal state. Coalesce safely, enforce sequence/retention guards in storage, and establish deletion barriers before deleting. Storage failure preserves copyable results and visibly reports unsaved content.
- Register whole packages atomically after validation. Disable/reload: reject new work, advance generation, revoke grants, cancel tasks, detach UI/subscriptions, bounded cleanup, then activate the new version.
- Manifests request capabilities; Host grants authorize every sensitive call. Bind true plugin/Invocation identity through closures and Promises. Templates replace known variables only; user input is separate user content, never executable template syntax.
- QuickJS runs on one owned worker with a shared lazy VM and per-plugin Contexts. Load only package-local JS/modules. No npm, remote code, shell, arbitrary files, raw credentials, or direct GPUI/HTTP/SQL handles. Enforce CPU/job-pump/memory/output/network/KV budgets and verify allocator features. Do not claim OS sandbox isolation.
- Validate package paths, Windows names, ADS/UNC/drive paths, links, collisions, file counts and extraction sizes before installation. Validate redirects and never forward authentication across origins.
- OpenAI-compatible text requests use configurable providers/profiles. Preserve Base URL prefixes; attach credentials per request. Parse SSE incrementally across UTF-8/JSON/chunk boundaries; truncated streams preserve partial text and fail visibly. No automatic paid POST retries, disabled TLS checks, or silent Provider fallback.
- Versioned TOML is authoritative for settings; SQLite for content. Validate and atomically save; preserve corrupt/newer data rather than resetting it. Back up SQLite consistently with WAL. Keys belong only in Credential Manager; diagnostics contain no secrets or text bodies.
- Paths are independent of cwd and support Chinese/spaces. Default data is `%LOCALAPPDATA%/LexWisp`; `portable.flag` selects executable-adjacent data. Credentials do not migrate with that directory.
- Encapsulate unsafe Windows code with ownership/thread/release comments. Capture the original foreground target before activation; distinguish verified selection, candidate text, no selection, unsupported and failure. Preserve clipboard formats/competing writes. Revalidate source replacement and one-shot user authorization; never replace a whole field as a selection fallback or auto-elevate.
- Centralize DPI/work-area conversion, including negative coordinates. Register a replacement hotkey before releasing the old one; restore tray state after Explorer restarts.

## Quality and delivery

Read and reuse existing code first. Prefer concrete types, enums, guard clauses and small meaningful ports. Avoid speculative abstractions, forwarding layers, unrelated refactors, and error swallowing. Do not unwrap fallible user/network/file/database/system operations. Startup failures need visible diagnostics and controlled exit.

Test real risks and actual regressions; do not test every getter/label or replicate upstream's suite. Keep test-only features in matching dev-dependencies; avoid `--all-features`. Never use live keys in automatic tests.

For the packages that exist, run:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
```

Build and launch Release from Stage 0 onward, using the actual staged/extracted directory. Use one main `scripts/package.ps1`; package only explicit runtime files and required licenses, never developer data/logs/caches/keys. Keep panic-unwind semantics unless justified otherwise.

Record actual commands, platform/hardware/driver/DPI, artifact paths, Release size and measurements in the implementation log. Distinguish compile, launch, functional acceptance and performance acceptance. Measure working set, Private Bytes, handles/threads and GPU memory separately; never trim working sets to hide growth. Do not fabricate screenshots, test output or benchmarks. Mark unavailable native/IME/service/clean-machine checks as pending; independent work may proceed, but unverified gates are not complete.
