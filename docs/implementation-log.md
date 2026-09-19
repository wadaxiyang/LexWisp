# Implementation log

## Stage 0 — 2026-09-19

Status: **implemented; Windows Release build, launch, and Stage 0 native interaction gates passed on the machine below**. This is a local input probe, not an AI release. Clean-machine portability and the broader performance/compatibility gates remain for Stages 8–9.

### Scope

- Read both repository specifications in full; the extension's `LexWisp_IMPLEMENTATION_SPEC.md` references resolve to the actual `LexWisp_SPEC.md`.
- Rewrote `AGENTS.md` in concise English, retaining product constraints, dependency direction, ownership, security, staged implementation, and evidence requirements. The specifications remain unchanged.
- Created only `lexwisp-app` and `lexwisp-ui`. No empty Host/core/plugin crates, registry, service container, event bus, Provider, or ChatController.
- Added the real `LexWisp` binary, one Kit initialization, explicit quit mode, retained application close subscription, and visible startup error reporting. Last-window close exits at Stage 0 because no tray/hotkey recovery exists yet. Startup open failure returns a nonzero exit code.
- `surface.rs` centralizes `build_window_options`, `open_surface_window`, and `LexWispWindowRoot`. Each window has exactly one Kit Root; dialog/sheet/notification layers share one composition. The helper contains no plugin business branches.
- The probe uses Kit Input, Textarea, Button, built-in Lucide icon, themes, and a genuinely selectable read-only result. Input entities and their subscription live on the View; render has no application I/O/tasks. Preview copies the submitted input locally and visibly counts submissions; it does not pretend to be a model response. Blank and over-128-KiB submissions show a local error.
- Rebuild-window creates the replacement before removing the original, leaving the original usable on failure. The probe intentionally resets its temporary input on replacement. Production conversation/draft ownership is deferred to its specified stages.
- QuickJS is a **dev-dependency only**, with two focused dependency regressions, not a distributed executable or an application-startup VM.
- One `scripts/package.ps1` builds Release, copies an explicit runtime allowlist, generates dependency/license notices, ZIP, and SHA-256. Data, logs, source, PDBs, and build caches are excluded.

### Frozen baseline and source checks

| Component | Baseline |
| --- | --- |
| Rust / Cargo | 1.95.0, target `x86_64-pc-windows-msvc`, edition 2024 |
| GPUI-Kit | crates.io 0.6.1, `component` + `assets` defaults |
| Component / Base / Assets / Component macros | crates.io 0.6.1 in Cargo.lock |
| GPUI implementation family | Kit-resolved `gpui-pre-*` 0.3.5 in Cargo.lock |
| rquickjs / core / sys | 0.14.0, only `std`; no `rust-alloc` or custom allocator feature |
| Startup Windows API | windows-sys 0.61.2; MessageBoxW only |
| Native build tools | VS Build Tools 18, MSVC 14.51.36231; linker 14.51.36256.0 |
| Packaged VC runtime | x64 developer redist, file version 14.51.36247.0, 178,616 bytes |
| Release profile | Cargo default optimized Release; panic unwind retained; no LTO tuning |

Read the locked Kit facade, matching published input/overlay/window/lifecycle tests, Base input `submit_on_enter` and `PressEnter` implementation, Component Textarea read-only behavior/theme APIs, GPUI close observers/window options, Windows keyboard/IME handling, and QuickJS allocator-limit implementation before relying on them. Kit supplies the matched GPUI types; the project has no separate direct GPUI dependency and does not enable UI test features in production.

The [upstream getting-started guide](https://gpui-kit.com/docs/getting-started/) was orientation only; shipped source determined the actual APIs. In this version, multiline state is `TextareaState`, not an invented `InputState::multi_line` method. Native interaction tests below exercise the actual packaged controls, rather than claiming the upstream test suite ran here.

Two diagnosed baseline issues were resolved without upstream source patches:

1. The initially installed Rust 1.94.1 failed with E0658 at GPUI's `std::hint::cold_path` calls. That API is stable [since Rust 1.95.0](https://doc.rust-lang.org/stable/std/hint/fn.cold_path.html). Installed and pinned 1.95.0; no nightly flags or dependency roulette.
2. An initial application manifest duplicated GPUI's `windows-manifest` resource (CVT1100/LNK1123). Removed the redundant build script/resource dependency. Verified the existing GPUI manifest supplies `asInvoker`, PerMonitorV2, Common Controls and SegmentHeap. No additional manifest is linked.

### Commands and results

All final commands ran successfully on native Windows:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc -- --nocapture
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1
```

- fmt: passed; check: passed; Clippy: passed with warnings denied.
- Tests: **2 passed, 0 failed**. Unicode expression returned `中文 42`; a synchronous `for (;;) {}` raised an interruption after **50.0613 ms**, then the same context evaluated successfully after removing its interrupt handler. An 8-MiB-limited VM rejected a 32-MiB ArrayBuffer with an out-of-memory exception. This verifies the selected allocator path; it does not claim per-plugin isolation or complete Script support.
- Final Release compiled/linked successfully. `dumpbin /DEPENDENTS` found OS DLLs plus `VCRUNTIME140.dll`; the latter is bundled from the MSVC x64 Redist directory, not copied from System32. Its own imports are OS/UCRT DLLs. The running extracted process's module path confirmed it loaded **the bundled** DLL.
- Local raw build/check/Clippy/test logs are retained under ignored `target/stage0-verification/`; this document is the durable stage record.

### Native Release acceptance

Used the Computer Use skill against the real Release windows, first at `dist/stage-00/LexWisp.exe`, then at `dist/验证 解压/stage-00/LexWisp.exe` after ZIP extraction.

| Action | Observed result |
| --- | --- |
| Initial launch | One visible native window; Chinese labels, icon, input caret and light theme rendered |
| Unicode entry | `你好，LexWisp! English 123` displayed correctly |
| Shift+Enter | Inserted a newline; did not submit |
| Enter | Two-line input appeared unchanged in result; submission counter advanced once |
| Mouse submission | Preview button updated result/counter once |
| Read-only selection | Drag selected just the first result line; Ctrl+C then Ctrl+V in the label input pasted only that line |
| Read-only edit rejection | Typing `SHOULD NOT EDIT` over the selection did not alter result |
| Native IME | Physical `n`, `i` produced a visible Chinese candidate window; Enter ended composition without changing the submission counter/result; a subsequent `n` + Space committed `你` without submitting |
| Theme | Entire content switched light → dark while preserving text; system titlebar remained native |
| Root overlay | About dialog rendered above content; Escape dismissed only the dialog and restored the previous input focus |
| Resize | Native maximize and restore preserved text and usable controls |
| Helper called again | Rebuild created window ID 1378240 and removed old ID 198576; one window/one process remained; probe content reset and dark theme persisted |
| Titlebar close | Last window disappeared and process exited; no invisible Stage 0 process remained |
| ZIP extraction | App started from a Chinese path containing a space and loaded its app-local VC runtime |
| Explicit Quit | Quit button in the extracted build ended the process; final process count was zero |

The accessibility bridge exposed input/button labels but sometimes reported focus/value updates later than the rendered frame. Native screenshots and copy/paste behavior were used to confirm the interactions; accessibility focus output alone was not treated as proof. This observation should be revisited if accessibility becomes an acceptance target.

### Machine and initial measurements

| Item | Observed value |
| --- | --- |
| OS | Windows 11 Pro for Workstations, 10.0.26200, build 26200 |
| CPU | Intel Core i5-13500 |
| Installed physical RAM reported by CIM | 34,132,275,200 bytes |
| GPU inventory | NVIDIA RTX 5070 Ti, driver 32.0.16.1047; Intel UHD 770, driver 31.0.101.3616; GameViewer virtual adapter, driver 15.6.5.199 |
| DPI baseline | Registry AppliedDPI 96, per-monitor DpiValue offsets 0; no scaling settings changed |
| Executable | 22,809,600 bytes (~21.75 MiB) |
| ZIP with runtime/readme/notices | 8,477,426 bytes (~8.08 MiB) |
| Visible staged window snapshot | Working set 75,153,408 bytes; Private Bytes 83,021,824; 593 handles; 50 threads |
| Fresh extracted window snapshot | Working set 68,784,128 bytes; Private Bytes 81,440,768; 596 handles; 51 threads |

Memory was sampled with `Get-Process` while the window was visible, with no network/task workload. These are initial snapshots, not leak, CPU, latency, GPU-memory or p95 benchmarks. No working-set trimming was performed. GPU inventory does not establish which adapter the renderer selected. Destroying one window is not evidence that driver caches returned to zero.

### Artifacts, limits, and next stage

- Runnable directory: `dist/stage-00/` (`LexWisp.exe`, `vcruntime140.dll`, `README.md`, `THIRD-PARTY-NOTICES.txt`).
- ZIP: `dist/LexWisp-stage-00-windows-x64.zip`; checksum: adjacent `.zip.sha256`.
- EXE SHA-256: `1cf749fdee2e8f2eaac12f19dcda1414b8674a7cdfdb7a86c71e3c4980dc0efd`.
- Final ZIP SHA-256: `7baab70550a66d81eea827f5f0974bccb5cca4a864c92ea92b46bcfa6d1ba141`.
- No AI/API, tray, global hotkey, persistence, settings UI or plugin execution is delivered at this stage. The application labels that boundary explicitly.
- Windows 10, clean machines without development tools, alternative IMEs, multiple monitors/scales, tiny displays, 100-window-cycle resource trends, GPU memory and startup latency are **not verified**. App-local runtime loading on this developer machine is not a substitute for Stage 9's clean-machine gate.
- Stage 1 should introduce the real Host services/scoped handles, task ownership, single instance, tray/hotkey, WindowRegistry and HiddenWarm lifecycle, UI command bridge and persisted basic settings. Move the startup-only Win32 diagnostic into the platform crate when that real layer is introduced. Do not retain Stage 0's last-window-exits policy once tray/hotkey residency is implemented.

### Stage 0 re-audit — 2026-09-19

- Confirmed the installed `gpui-kit` and `gpui-kit-design-guides` skills are available. Read their complete required design/coding guides, component conventions, application recipe, and the locked 0.6.1 source/tests relevant to Input, Textarea, Root overlays, and window creation.
- Added an explicit `AGENTS.md` rule that all LexWisp UI work must use those two Longbridge GPUI Kit skills and the locked workspace APIs; agents must stop UI changes if the skills are unavailable rather than substitute another framework.
- Re-read `LexWisp_SPEC.md` and `LexWisp_SPEC_EXTEND.md` in full and audited every Stage 0 task against source, dependencies, artifacts, and the evidence above. Stage 0 remains deliberately limited to `lexwisp-app` and `lexwisp-ui`; no Stage 1 scaffolding was added.
- Replaced the probe's fixed-pixel content heights with GPUI rem-scale helpers (`h_32` and `h_40`) and corrected the dialog-opening command label to `关于验证…`. The remaining direct `px` values are confined to `WindowOptions`, where GPUI requires resolved platform window geometry, and that exception is documented at the owner.
- Re-ran successfully: `cargo fmt --all -- --check`, workspace check, workspace/all-target Clippy with warnings denied, workspace tests, Release build, and `scripts/package.ps1 -SkipBuild`. QuickJS again passed 2/2 tests; the loop interrupted after 50.0685 ms.
- The rebuilt staged executable is 22,809,600 bytes and the ZIP is 8,477,326 bytes. The artifact hashes above now describe this rebuilt output.
- Launched the rebuilt executable from `C:\123\CODE\LexWisp\dist\stage-00\LexWisp.exe`. The process exposed a nonzero main window handle titled `LexWisp · Stage 0`, reported responsive, loaded from the staged path, and exited within 10 seconds after its main window was closed; final process count was zero. Snapshot at launch: working set 66,568,192 bytes, Private Bytes 81,059,840, 561 handles, 47 threads.
- The Windows computer-use connector returned no native application inventory in this session and its native app methods were disabled, so the earlier full input/IME/selection/overlay native acceptance was not represented as newly rerun. The current changes do not alter those interaction paths; current compile, test, package, launch, responsive-window, and clean-exit gates passed. A fresh screenshot/keyboard smoke run remains desirable when native UI automation is available, but it does not invalidate the previously recorded Stage 0 acceptance.

## Stage 1 — 2026-09-19

Status: **implemented; Release build, packaging, and the required start → no-window residency → global-hotkey recovery → explicit exit sequence passed on Windows**. Stage 1 is a system shell and contains no AI product action.

### Delivered boundaries

- Added real `lexwisp-core`, `lexwisp-platform-windows`, `lexwisp-storage`, and `lexwisp-host` crates with the dependency direction defined by the specifications. `core` remains free of GPUI, Win32, SQLite, HTTP, and concrete plugins.
- `core` provides stable plugin/action identifiers, object-safe action execution, typed settings/errors, capability names, task owners, surface commands, and the atomic-file port.
- `host` owns one two-worker Tokio business runtime, `HostTaskPort`/owned `TaskScope`s with cancellation and retained handles, typed `HostHandles`, the settings service, plugin/action registries, bounded lifecycle notifications, and `CapabilityAuthority`. Package registration validates the complete package before commit; removal detaches only that package and its actions.
- `platform-windows` owns the single-instance mutex/wake message, notification-area icon and real menu commands, replacement-before-release global hotkeys, `TaskbarCreated` restoration, current-user startup registration, message-only window/thread, two-second bounded shell shutdown, Win32 show/hide, startup diagnostics, and atomic settings replacement. Unsafe blocks include local ownership/thread/lifetime comments.
- `storage` owns versioned TOML discovery, validation, load, and save. The packaged `portable.flag` selects executable-adjacent `data`; without it the path is `%LOCALAPPDATA%/LexWisp`. Missing configuration is first-run; corrupt/newer configuration is reported and left untouched.
- `ui` uses the locked GPUI Kit 0.6.1 family and its Root, Radio, Switch, Button, theme, and overlay composition. `SurfaceFactory`/`SurfaceController` owns a real `WindowRegistry`; Quick Shell implements Visible/HiddenWarm/Destroyed with generation checks and configurable 0/30/60-second retention. Window destruction is queued after controller borrowing ends, preventing a reentrant GPUI window-table update. Control Center persists only real hotkey/theme/startup/retention settings.
- `app` is the composition root and retains the bounded async `HostUiCommand` bridge, window-close observer, quit observer, SurfaceFactory entity, Host, Windows shell, and instance guard. Closing all GUI windows does not create or retain a hidden GPUI keeper window and does not exit; explicit quit shuts down Host before the Windows shell.
- `scripts/package.ps1` remains the single packaging script. Stage 1 archives only `LexWisp.exe`, app-local `vcruntime140.dll`, `portable.flag`, README, and generated notices.

### Defects found by native acceptance

1. The first implementation destroyed a hidden Quick Shell by nesting `WindowHandle::update` inside an active `SurfaceController` update. The Release process terminated with `0xc0000409` when the retention timer expired. Destruction is now two-phase: the generation-checked registry entry is taken first, then GPUI window removal is deferred to the next effect cycle after the controller borrow ends. Both immediate destruction and timed destruction subsequently kept the same resident process alive.
2. The first repeated settings save called `ReplaceFileW` while the temporary file handle was still open and also requested a persistent backup name. The second save failed. The writer now syncs and closes the temporary handle before a backup-free atomic replacement; a Windows regression writes and replaces the same file twice.

### Commands and automated results

The following final commands passed on native Windows:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo test -p lexwisp-platform-windows --locked --target x86_64-pc-windows-msvc replacement_conflict_preserves_the_previous_hotkey -- --ignored --nocapture
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1 -SkipBuild
```

- Normal suite: **13 passed, 0 failed, 1 native fixture ignored by default**. This includes plugin-ID/action-ID/settings validation, atomic package registration, wrong-owner rejection, package removal, missing/corrupt/round-trip TOML, repeated Win32 atomic replacement, and both QuickJS probes.
- The separately invoked native hotkey fixture passed: it occupied `Ctrl+Shift+Space`, confirmed replacement registration failed, then proved `Ctrl+Alt+Space` remained registered. It also starts and cleanly shuts down the actual notification-area/message-thread shell.
- Workspace format, locked check, and all-target Clippy with warnings denied passed. Release compiled and linked successfully with panic unwind unchanged.

### Native packaged Release acceptance

The final Release was launched from `C:\123\CODE\LexWisp\dist\stage-01\LexWisp.exe`; Computer Use controlled the actual GPUI windows and captured their rendered state/accessibility controls.

| Action | Observed result |
| --- | --- |
| First run | Control Center opened with the three hotkeys, system/light/dark themes, startup Switch, 0/30/60-second retention, and one real Save action |
| Default save | Saved successfully and displayed settings generation 1; `data/settings.toml` contained schema 1 and the selected values |
| Subsequent launch | Process ID 296 was responsive with `MainWindowHandle = 0`, 411 handles and 34 threads; no GUI keeper window existed |
| Global hotkey | From an unrelated foreground window, `Ctrl+Alt+Space` opened Quick Shell |
| Single instance | A second staged process exited 0 within five seconds; the original process ID remained and Quick Shell was shown |
| HiddenWarm | Quick Shell ID 395448 was hidden and reopened 19,416 ms later with the same ID |
| Immediate Destroyed | With retention 0, ID 3541168 disappeared; process 21712 remained responsive with no window; hotkey recreated ID 7210412 |
| Timed Destroyed | With retention restored to 30, a hidden Quick Shell later reopened with a new ID while process 21712 and the saved configuration remained intact |
| No-window residency | After Quick Shell hide and Control Center close, LexWisp remained alive with no targetable GUI window; hotkey recovery worked |
| Repeated save | Changing retention 30 → 0 and later 0 → 30 both saved; final TOML restored the default 30 seconds |
| Explicit exit | `Exit LexWisp` removed the final window and process; process count reached zero after bounded Host/shell shutdown |
| Final staged smoke | Final rebuilt/package-copied executable started tray-only, a second launch woke one Quick Shell, and its Exit action again left zero processes |

`Shell_NotifyIconW(NIM_ADD)` is a startup gate, so every successful resident launch above established a notification icon; the menu contains only Quick Shell, Settings, and Exit commands. The automation API could not target the Windows taskbar itself, so a physical pointer right-click on the icon was not separately replayed. Explorer-restart restoration also remains a later manual stress check.

### Artifacts and measurements

| Item | Observed value |
| --- | --- |
| OS / hardware / DPI | Same Windows 11 build 26200, Intel i5-13500, 34,132,275,200-byte RAM, NVIDIA RTX 5070 Ti + Intel UHD 770 inventory, 96-DPI baseline recorded for Stage 0 |
| Final executable | 19,420,160 bytes |
| Final ZIP | 7,379,991 bytes |
| Tray-only snapshot | Working set 48,476,160 bytes; Private Bytes 58,605,568; 411 handles; 34 threads |
| Visible snapshot after lifecycle cycles | Working set 71,081,984 bytes; Private Bytes 98,975,744; 594 handles; 48 threads |
| EXE SHA-256 | `6ecd2a717a6b3fedbb61bc12e2921e8c826ab8af3b7d0976bf2e66687e3e56ad` |
| ZIP SHA-256 | `f250ae325a78d6edda572b21daa55c91fc49824a3536c17815004baaa75ce96f` |

Runnable directory: `dist/stage-01/`. Archive: `dist/LexWisp-stage-01-windows-x64.zip`, with adjacent checksum file. Measurements are point samples, not leak/GPU/p95 claims; no working-set trimming was used. Windows 10, clean-machine portability, Explorer restart, alternate DPI/multi-monitor layouts, prolonged resource trends, and GPU memory remain pending for their later acceptance stages.

## Stage 2 — 2026-09-19

Status: **implementation complete; locked build, local-mock AI tests, Release packaging, fresh-directory launch, single-instance wake, and SQLite recovery passed. The external real-provider call/manual Stop flow remains an open acceptance gate because no user API credential or local compatible model was available.** Per the project rules, that unavailable gate is recorded as pending rather than fabricated.

### Delivered vertical slice

- Added stable `ConversationId`, `MessageId`, `AttemptId`, and `InvocationId` types plus typed provider, AI, execution, Chat, action, and UI ports in `core`. Action lookup is now qualified by plugin and action ID, so packages may safely reuse local action names.
- Added a single reused `reqwest` client with an OpenAI-compatible `/chat/completions` adapter. Base path prefixes are preserved; only HTTPS and loopback HTTP are accepted; credentials/fragments/full completion endpoints are rejected; cross-origin redirects are rejected; request, first/next-event, total-size, and total-duration limits are explicit. Both streaming and non-streaming responses are supported without automatic paid POST retry or provider fallback.
- Added Windows generic Credential Manager storage. TOML stores only a credential reference. Provider save writes the new credential before atomically committing settings, rolls it back on settings failure, and deletes the superseded reference only after success.
- Added the dedicated `lexwisp-sqlite` worker, one main connection, a bounded queue, WAL, schema version 1, and `conversations`, `messages`, and `executions` tables. Startup converts abandoned running states to interrupted. Checkpoints use sequence and retention-generation guards; Chat message content has one persistent source in `messages`.
- Added `ExecutionStore`, `ExecutionAccumulator`, checkpoint projection, and `InvocationSupervisor`. Network semantic deltas first enter the in-memory store; UI notifications are coalesced around 33 ms; persistence checkpoints occur around 500 ms or 16 KiB; terminal state flushes UI immediately and waits for a bounded reliable SQLite enqueue/receipt. Exactly one terminal transition wins, stale plugin generations and post-terminal deltas are rejected, and storage failure leaves the copyable in-memory result visible as unsaved.
- Added `lexwisp-plugins-builtin`. Its compiled Chat package is registered through the formal Plugin/Action Registry and receives only an identity-scoped `ChatRunPort`. `ChatController` owns the one Stage 2 conversation, stable message order, context assembly, send/stop/retry semantics, execution binding, and surface snapshots; it owns no HTTP, SQL, credentials, or windows.
- Replaced the Stage 1 placeholder with the GPUI Kit Chat view: retained `TextareaState`, IME-safe Enter submit/Shift+Enter newline behavior, virtual `MessageScroller`, stable selectable Markdown, Kit messages/bubbles, Copy, Send, Stop, Retry, status/unsaved feedback, and jump-to-latest behavior. Hidden windows receive no Chat snapshot churn; showing a new/warm surface reads the latest controller snapshot, while Host execution continues independently.
- Extended Control Center with provider name, Base URL, manual model ID, masked API key, authentication/stream switches, and real Test/Save operations. System settings and provider settings preserve each other. The page uses the locked GPUI Kit input, switch, button, theme, and scrolling components.
- `HostHandles` now exposes only initialized typed services and builds scoped plugin ports; plugins never receive the handles. Host still owns one Tokio runtime, reused HTTP/storage/credential/provider resources, and bounded shutdown. No hidden GUI keeper window, per-request runtime, service locator, global event bus, raw JSON invoke escape hatch, or UI-side reqwest/SQLite path was added.

### Automated and native results

The following final commands passed on Windows x64:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo test -p lexwisp-platform-windows --locked --target x86_64-pc-windows-msvc replacement_conflict_preserves_the_previous_hotkey -- --ignored --nocapture
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1 -SkipBuild
```

- Normal suite: **24 passed, 0 failed, 1 native fixture ignored by default**. The separately invoked native hotkey replacement fixture also passed.
- AI mock coverage includes distinct 401 and 429 errors, empty choices, fragmented UTF-8/JSON/SSE boundaries, interrupted streams with preserved partial text, and cancellation of a slow stream. No live key is stored in tests.
- Storage/execution coverage proves an older checkpoint cannot overwrite a terminal message and that 1,000 small deltas produce fewer than 20 queued checkpoints rather than one SQL write per delta.
- ChatController coverage proves the conversation ID remains stable while a send creates stable message IDs and projects the terminal assistant answer.

The final packaged archive was extracted to fresh `acceptance-stage02-settings-20260919-193756` and `acceptance-stage02-final-20260919-193738` directories under `dist`. First launch exposed a responsive native `LexWisp · Settings` window and created executable-adjacent `data\lexwisp.db`, WAL, and SHM files. A second launch exited with code 0 and woke `LexWisp · Quick Shell` in the original process. The original process remained responsive. A preceding fresh-directory run was forcibly stopped and relaunched to verify WAL/database recovery; schema version 1 and all four expected tables were queried successfully.

The Computer Use connector returned no native application surfaces in this session, so button-level screenshots, IME interaction, hiding during an active native request, and tray-driven graceful exit were not newly replayed. Process cleanup used `Stop-Process` and is not claimed as an explicit-quit acceptance result; Stage 1 already holds the separate explicit-exit evidence for the unchanged shell path.

### Artifacts and measurements

| Item | Observed value |
| --- | --- |
| OS / hardware | Windows 11 build 26200; Intel i5-13500; 33,332,300 KiB visible RAM; NVIDIA RTX 5070 Ti + Intel UHD 770 inventory |
| Fresh Settings window | Working set 69,562,368 bytes; Private Bytes 82,841,600; 579 handles; 52 threads |
| Quick Shell after second-launch wake | Working set 72,323,072 bytes; Private Bytes 100,397,056; 591 handles; 52 threads; process responsive |
| Release executable | 31,055,360 bytes; SHA-256 `c3c7783a6044d091abdfb3f4c6bfbddb24cfc182dd335c1eda3a9f6a584e78f0` |
| Release ZIP | 12,114,860 bytes; SHA-256 `4f040ab714024361015bf39956015fed6e63bfedea8e9ed105e194afd39af8de` |

Runnable directory: `dist/stage-02/`. Archive: `dist/LexWisp-stage-02-windows-x64.zip`, with adjacent checksum. Package contents are explicitly allowlisted and contain no settings, database, logs, caches, or credentials. Measurements are point samples, not leak/GPU/p95 claims; no working-set trimming was used.

Pending Stage 2 acceptance: configure a user-authorized real compatible endpoint through Control Center, complete one streamed answer, manually stop a long answer, hide/reopen Quick Shell during the request, inspect the persisted rows, verify the key is absent from TOML/log output, and capture the native UI evidence. Windows 10, clean-machine portability, alternate IMEs, DPI/multi-monitor layouts, prolonged resource trends, GPU memory, and provider-specific redirect/proxy behavior remain later/manual gates.

## Stage 3 — 2026-09-19

Status: **implemented and packaged; locked build/test/static quality gates, the native hotkey fixture, staged Release launch, single-instance wake, and schema-2 creation passed. Native selection/replace interaction against Notepad and a browser/editor, IME/menu dismissal interaction, and a live-provider Translate/Polish result remain pending because the Windows Computer Use surface returned no applications and no user-authorized compatible provider was available.** Those missing manual gates are not represented as complete.

### Delivered daily-use path

- Added `LaunchMode` with the exact `action_palette`, `translate_selection`, and `default_action` settings. The pure route test covers every mode with and without a verified selection. Invalid or absent automatic actions fall back to the action entry with a visible reason; no-input routes to manual Quick Ask and never starts a model request.
- Added a dedicated `lexwisp-uia-mta` COM MTA worker. The Windows shell captures the foreground HWND/process/title before the GPUI window is shown, asks UI Automation for the focused text selection, rejects password controls, and returns only an authorized DTO plus an opaque 60-second replacement token. COM elements never leave their owning thread.
- Added a bounded 800-ms capture wait. A blocked cross-process UIA item pauses later automatic capture rather than spawning replacement workers. Ordinary clipboard read/write stays available independently if UIA is blocked.
- Added restricted Ctrl+C fallback. It first snapshots every clipboard format that can be safely copied as movable global memory, refuses known handle-based formats, waits for shortcut modifiers to release, rechecks the foreground target, detects a new clipboard sequence, reads only new Unicode text, and restores the prior formats only if no competing writer changed the clipboard. Because the sequence cannot prove source selection, fallback text is always a candidate requiring a user click.
- Added manual replacement behind an explicit result button. The platform worker rechecks token age, HWND existence, process identity, the retained UIA element, and the exact still-selected text before activating the original window and issuing paste. It never uses `ValuePattern.SetValue`, never substitutes the whole field, never elevates, invalidates a successful token, and copies the result instead when validation fails. Successful replacement intentionally leaves the result on the clipboard.
- Added generic manifest parsing and Prompt execution for the compiled Translate and Polish packages. Their IDs, names, allowed sources, parameters, Fast profile, cancel default, and output actions come from `manifest.toml`; prompt files use only declared `{{params.*}}` replacement. The actual input remains a separate user message. Unknown variables, parameters, kinds, sources, or required values fail before the request.
- Translate and Polish use the same scoped `TextRunPort`, `InvocationSupervisor`, `AiService`, `ExecutionStore`, checkpoint projector, Provider resolution, cancellation, and bounded streaming path as Chat. Host contains no Translate/Polish ID branches and no second HTTP/SSE/persistence implementation.
- Replaced the Chat-only popup composition with one GPUI Kit Quick Shell containing the action palette, verified/candidate/explicit-clipboard source preview, manifest-driven text/enum/boolean/number parameter controls, manual composer, selectable streaming result, Stop, Copy, Favorite, and safe Replace. Stateful Kit inputs and subscriptions are created once; side effects start from intent callbacks rather than render.
- Added explicit clipboard input only for actions whose manifest allows it. Failed selection never reads or uploads the previous clipboard implicitly.
- Added generic per-action `cancel`/`continue` overrides to versioned TOML and Control Center. Chat still continues when hidden. Declarative controllers cancel on the actual Surface detach by default; menu/dialog/IME focus changes do not call the hide path.
- Extended SQLite to schema 2 with `action_executions` and `favorites`. Non-Chat actions persist input/output/partial/terminal state directly without fake Chat rows. Favorite toggling is real and survives in SQLite. A newer schema is now rejected before any schema or status write.

### Commands and automated evidence

The following final commands passed on native Windows x64:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo test -p lexwisp-platform-windows --locked --target x86_64-pc-windows-msvc replacement_conflict_preserves_the_previous_hotkey -- --ignored --nocapture
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1 -SkipBuild
```

- Normal suite: **29 passed, 0 failed, 1 native fixture ignored by default**. The separately invoked real-hotkey fixture passed.
- New coverage proves all six launch-mode/input-presence route combinations, strict manifest decoding, known-variable-only prompt expansion, direct non-Chat execution persistence, favorite persistence, settings round-trip for launch/default/dismiss values, sequence protection, and rejection of a schema-99 database without creating Stage 3 tables.
- Existing fragmented SSE, truncated stream/partial result, HTTP error, cancellation, Chat stable-ID, checkpoint coalescing, credential/settings, and QuickJS probes remain green.

### Staged Release smoke and artifacts

Launched `C:\123\CODE\LexWisp\dist\stage-03\LexWisp.exe` from the actual packaged staging directory. First launch exposed a responsive native `LexWisp · Settings` window. A second process exited with code 0 and changed the original process's visible window to `LexWisp · Quick Shell`, proving the existing single-instance wake path still works with the Stage 3 composition. The staged portable data directory created `lexwisp.db`, WAL, and SHM; `sqlite3` reported `schema_version = 2` and the six expected tables: `conversations`, `messages`, `executions`, `action_executions`, `favorites`, and `schema_version`. The smoke process was then forcibly stopped because native UI control was unavailable; this is not claimed as a fresh explicit-quit test.

| Item | Observed value |
| --- | --- |
| OS / hardware / DPI | Windows 11 Pro for Workstations 10.0.26200 build 26200; Intel Core i5-13500; 34,132,275,200 bytes RAM; 96 DPI |
| First Settings window | Working set 70,836,224 bytes; Private Bytes 82,165,760; 576 handles; 53 threads; responsive |
| Release executable | 31,917,568 bytes; SHA-256 `ec1917b65134fc13223c90f416f115a5dcd0ce313a9e8671f51a5a41bb5c4e6a` |
| Release ZIP | 12,402,145 bytes; SHA-256 `8b5510e8c02e3b915fb97f7861ea62f91ab15b46c3688a6086c1bb8c02177902` |

Runnable directory: `dist/stage-03/`. Archive: `dist/LexWisp-stage-03-windows-x64.zip`, with adjacent checksum. The ZIP allowlist contains only the executable, app-local VC runtime, `portable.flag`, README, and generated notices; it contains no settings, database, logs, caches, or credentials. The staged directory's `data` files were generated only by the smoke run and are not in the ZIP.

### Pending native acceptance

The `computer-use` skill was initialized twice as prescribed, including one reset/retry, but both inventories returned no native applications and a `nodeRepl.fetch request failed` error. Therefore the following specification gates remain pending and must be replayed on an interactive desktop with working native control and a user-authorized Provider:

- select text in Notepad and one common browser/editor, exercise all three launch modes with and without selection, confirm candidate fallback, and record unsupported targets;
- run real Translate and Polish streams, Stop with a partial result, Copy, Favorite, hide-to-cancel and the configured continue override;
- open Kit menus/dialogs and a Chinese IME candidate window during generation and prove none is treated as Surface dismissal;
- replace a still-valid Notepad selection, then switch/change/expire the original target and prove LexWisp copies without writing to the wrong application;
- verify target applications at different integrity levels, complex multi-format clipboard contents, Windows 10, alternative IMEs, multi-monitor/DPI coordinates, resource trends, GPU memory, and clean-machine portability at their later acceptance gates.

## Stage 4 — 2026-09-19

Status: **implemented and packaged; all locked build/test/static gates, the native hotkey fixture, staged Release startup, single-instance wake, schema-3 creation, and persisted-conversation restart recovery passed. Native button-level Chat Panel/handoff/IME interaction and live-provider parallel-stream checks remain pending because Windows Computer Use again exposed no applications and no user-authorized compatible Provider was available.** These unavailable gates are not represented as complete.

### Delivered multi-conversation Chat

- Expanded the compiled Chat plugin's single `ChatController` into a stable-ID conversation index with create, deterministic initial naming, rename, switch, delete, startup restore, per-conversation model preference, and one independently supervised active invocation per conversation. One conversation rejects a competing generation while different conversations can prepare independently; execution observations route by conversation and assistant-message IDs even when another conversation is active.
- Regeneration reuses the user message but creates a new `AttemptId`, assistant `MessageId`, ordinal, and Invocation. Old attempts remain in the authoritative message history and SQLite. Context assembly admits only the latest completed assistant attempt for each user round, never a cancelled/failed partial attempt, removes complete oldest rounds against the selected Provider/model's client-side estimated budget, reports pruning, and rejects an oversized current input without truncation.
- Extended the OpenAI-compatible Provider configuration with a backward-compatible `context_budget` value (default 8,192 estimated tokens). Host resolves the selected model and exposes only its numeric budget through the scoped Chat port; Chat retains ownership of conversation semantics and context selection.
- Added schema 3 columns for conversation model preference and assistant attempt/reply identity plus a `conversation_deletions` tombstone table. Conversation deletion first establishes a durable barrier so late checkpoints cannot recreate deleted content; restoring reconstructs titles, models, message order, attempts, replies, and terminal/partial status. Version 1/2 databases migrate in place; a future schema is still rejected before writes.
- Added a native independent Chat Panel using the locked GPUI Kit 0.6.1 `Root`, `Input`, `Textarea`, `Radio`, dialog, virtual list, `MessageScroller`, message/bubble, selectable Markdown, and clipboard components. Its conversation sidebar and transcript are virtualized; it provides New chat, rename, confirmed delete, model preference, Send, Stop, Regenerate, full-answer copy, per-fenced-code-block copy, context-budget feedback, and jump-to-latest behavior.
- Quick Shell and Chat Panel observe the same controller and shared message renderer. `SurfaceController::handoff_to_chat_panel` shows/attaches the Panel before hiding/detaching Quick Shell, so it changes only projections and never invokes Chat again. Closing the Panel detaches only that surface and does not cancel Host work. The tray now has an explicit Chat Panel command.
- Snapshot delivery remains bounded but evicts a stale queued projection before retrying the newest one; this prevents a terminal state from being stranded behind streaming updates. Hidden windows still receive no refresh churn and reconstruct from the latest versioned snapshot when shown.
- Updated the main README and the single allowlisted packaging script for Stage 04. The ZIP contains only `LexWisp.exe`, the app-local VC runtime, `portable.flag`, README, and generated third-party notices; it contains no settings, database, logs, caches, or credentials.

### Commands and automated evidence

The following final commands passed on native Windows x64:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo test -p lexwisp-platform-windows --locked --target x86_64-pc-windows-msvc replacement_conflict_preserves_the_previous_hotkey -- --ignored --nocapture
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1 -SkipBuild
```

- Normal suite: **37 passed, 0 failed, 1 native fixture ignored by default**. The separately invoked real-hotkey replacement fixture passed.
- New controller coverage proves stable conversation IDs, retained regeneration attempts, same-conversation serialization, different-conversation independence, partial-attempt exclusion, complete-round budget pruning/current-input rejection, and recovery of the newest snapshot after bounded-channel pressure.
- New storage coverage proves conversation/model/attempt restoration and deletion barriers against late checkpoints. The existing SSE boundary, truncation/partial, cancellation, checkpoint coalescing, schema-forward rejection, settings, manifest, credential, and QuickJS probes remain green.

### Staged Release smoke and artifacts

Launched `C:\123\CODE\LexWisp\dist\stage-04\LexWisp.exe` from the actual staging directory. First launch exposed a responsive `LexWisp · Settings` window. A second process exited 0 within five seconds and woke `LexWisp · Quick Shell` in the original process. The portable database reported schema version 3 and the seven expected tables: `action_executions`, `conversation_deletions`, `conversations`, `executions`, `favorites`, `messages`, and `schema_version`. After stopping and restarting the staged build, the conversation count remained 1 rather than creating a replacement conversation, providing a storage-level recovery smoke. Final staged process count was zero; because cleanup used `Stop-Process`, it is not claimed as a fresh explicit-quit UI test.

| Item | Observed value |
| --- | --- |
| OS / hardware / DPI | Windows 11 Pro for Workstations 10.0.26200 build 26200; Intel Core i5-13500; 34,132,275,200 bytes visible RAM; WindowMetrics AppliedDPI 96 |
| GPU inventory | NVIDIA RTX 5070 Ti driver 32.0.16.1047; Intel UHD 770 driver 31.0.101.3616; GameViewer virtual adapter driver 15.6.5.199 |
| Final Settings window | Working set 71,249,920 bytes; Private Bytes 83,570,688; 576 handles; 53 threads; responsive |
| Release executable | 32,517,632 bytes; SHA-256 `fa202c66539dff32b813b23be341ff728d88a96aba0dfb982afaeaa4b45d49d5` |
| Release ZIP | 12,584,793 bytes; SHA-256 `b32085cca21af2a77358673683f4fed4b7182e732c194cbabe084e9c9b32ccf2` |

Runnable directory: `dist/stage-04/`. Archive: `dist/LexWisp-stage-04-windows-x64.zip`, with adjacent checksum. Measurements are point samples, not leak/GPU/p95 claims; no working-set trimming was used.

### Pending native acceptance

The `computer-use` skill was read and initialized as required. Two lightweight inventory attempts plus one reset/retry all returned an empty Windows application list and `nodeRepl.fetch request failed`, so the following Stage 4 gates remain pending on an interactive desktop with working native control and a user-authorized Provider:

- open the Panel from Quick Shell and the tray, verify destination-first handoff during generation, and prove the provider receives only one request;
- create/rename/switch/delete conversations through the native UI, run two conversations concurrently, and prove an inactive answer updates only its original conversation;
- close/reopen both surfaces during a stream, Stop and Regenerate, restart, and inspect restored completed/partial attempts and per-conversation model choice;
- exercise a long conversation/sidebar, scroll upward during streaming to stop auto-follow, use jump-to-latest, select long Markdown, copy each fenced code block, and test Chinese IME candidate confirmation versus Enter/Shift+Enter;
- repeat Windows 10, alternate DPI/multi-monitor layouts, clean-machine portability, long-run resource/GPU measurements, and a real compatible endpoint at their later/manual gates.

## Stage 5 — 2026-09-19

Status: **implemented and packaged; locked formatting/check/Clippy/test gates, the native hotkey fixture, Stage 05 Release launch, single-instance wake, schema-4 creation, 10,005-row cursor paging, retention barriers, config backup, and explicit preservation of an unrecorded favorite passed. Button-level Computer Use remains pending because the direct `@oai/sky` runtime reports that its trusted RPC service is not configured.**

### Delivered manageable personal-tool slice

- Added a typed history domain and `HistoryUiPort`; Host owns the service, while UI receives only typed snapshots and commands. History reads the authoritative Chat bodies from `messages` and non-Chat bodies from `action_executions`; it does not create a third body copy.
- Migrated SQLite to schema 4 with action identity for Chat executions, annotated favorite references, per-execution deletion tombstones, and a singleton retention generation. The one existing SQLite worker/connection now performs query, favorite, delete, clear, backup, and retention operations through its bounded command queue.
- Added stable `(updated_at_ms DESC, invocation_id ASC)` cursor paging capped at 100 rows, title/input/output basic search, plugin/status/favorite filters, detail loading, retry, and a 10,005-row regression proving complete duplicate-free traversal without loading all bodies into the UI.
- Single-history deletion revokes its live `ExecutionStore` persistence eligibility before a transaction establishes the tombstone. Clear advances the global retention generation before deleting; the default path preserves favorite-referenced executions/content, while the explicit alternate path removes favorites too. Old-generation and tombstoned late checkpoints cannot recreate deleted data.
- Added the automatic-recording setting. Turning it off advances the durable retention barrier and synchronously marks every running accumulator `NotRecorded`; later deltas remain copyable in memory but do not enqueue body checkpoints. A user-initiated favorite can explicitly persist a completed in-memory result under the current generation.
- Favorites are restored into the Host's in-memory index at startup. The history UI supports annotation updates and unfavorite; clear/delete keep their reference rules transactionally consistent.
- Added SQLite Online Backup API support, executed on the storage worker so the backup includes WAL state consistently. Valid config saves keep one bounded `backups/settings.previous.toml`; corrupt/newer TOML remains untouched, and Control Center can explicitly reload a valid external edit.
- Expanded Provider settings with context budget, explicit proxy, connect/total/stream-event timeouts, temperature, and max output tokens. Values are validated before save; request timeout and protocol fields are applied to actual calls, and HTTP clients are cached/reused by proxy/connect-timeout profile. Provider registry refresh now follows every settings apply/reload.
- Added generic per-action parameter defaults sourced from declarative manifests. The Control Center exposes Translate's text target-language value and Polish's declared enum choices without editing or hard-coding Prompt content; caller-supplied invocation parameters still take precedence.
- Reworked Control Center with locked Longbridge GPUI Kit components into real Settings, History & favorites, and Privacy & diagnostics views. History uses Kit `v_virtual_list`, `Input`, `Radio`, `Switch`, `Button`, selectable text, and clipboard support. Destructive operations require a second explicit click. Privacy exposes recording, config reload, consistent backup, paths, Credential Manager migration caveat, and a redacted diagnostic report that omits credentials and content bodies.
- The plugin-management page remains Stage 6 by specification and was not added as a clickable no-op.

### Commands and automated evidence

The final native Windows x64 commands passed:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo test -p lexwisp-platform-windows --locked --target x86_64-pc-windows-msvc replacement_conflict_preserves_the_previous_hotkey -- --ignored --nocapture
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
./scripts/package.ps1 -SkipBuild
```

- Normal suite: **42 passed, 0 failed, 1 native fixture ignored by default**; the separately invoked real-hotkey replacement fixture passed.
- New storage tests cover 10,005-row stable cursor pagination, per-execution delete barriers, clear-generation rejection, favorite-preserving clear, previous-valid-config backup, and forward-schema refusal.
- New execution tests prove recording-off prevents further running body checkpoint enqueue while keeping the in-memory result, and that explicit favorite preservation can save a completed unrecorded result.

### Staged Release smoke and artifacts

Launched `C:\123\CODE\LexWisp\dist\stage-05\LexWisp.exe` from the actual staged directory. First launch exposed a responsive `LexWisp · Settings` window. A second process exited 0 within five seconds and woke `LexWisp · Quick Shell` in the original process. The staged portable database reported schema version 4, retention generation 0, and the expected tables: `action_executions`, `conversation_deletions`, `conversations`, `execution_deletions`, `executions`, `favorites`, `messages`, `retention_state`, and `schema_version`. Cleanup used `Stop-Process`, so this run is not claimed as a fresh explicit-quit UI test.

| Item | Observed value |
| --- | --- |
| OS / hardware / DPI | Windows 11 Pro for Workstations 10.0.26200 build 26200; Intel Core i5-13500; 34,132,275,200 bytes RAM; prior 96-DPI baseline |
| Quick Shell point sample | Working set 71,000,064 bytes; Private Bytes 103,542,784; 583 handles; 52 threads; responsive |
| Release executable | 33,352,192 bytes; SHA-256 `096dd4ac70c7580284463bf3668898ed1b6e75b9a24fd0226bd112ec134e96f0` |
| Release ZIP | 12,852,444 bytes; SHA-256 `9a4fb29a7247365de8724a547a78808b393e9b4fc7fbf4d066fc4eef7de97a0b` |

Runnable directory: `dist/stage-05/`. Archive: `dist/LexWisp-stage-05-windows-x64.zip`, with adjacent checksum. Its explicit allowlist contains only `LexWisp.exe`, `vcruntime140.dll`, `portable.flag`, `README.md`, and `THIRD-PARTY-NOTICES.txt`; generated settings/database/logs/caches/credentials are not archived.

### Pending native acceptance and exact Computer Use blocker

The `computer-use` skill, guidance, API reference, and confirmations were read before automation. Following the user-provided workaround, the session imported `@oai/sky` directly and called `sky.list_apps()` without calling `cua.getState()`. The import succeeded, but the initial call, one delayed lightweight retry, and one reset/reinitialize retry all returned exactly `Trusted RPC service is not configured: sky`. Per the skill's bounded recovery guidance, no further repeated inventory calls were made.

Consequently, button-level Control Center navigation, search/copy/confirmation UI, Chinese IME interaction, and a real Provider's concurrent streaming/retry remain pending. The direct error indicates missing trusted-RPC host configuration rather than a LexWisp failure. Windows 10, alternate DPI/multi-monitor layouts, clean-machine portability, prolonged resource/GPU measurements, disk-full UI observation, and Credential Manager migration to a second machine remain later/manual gates; the README and diagnostics page explicitly state that keys do not migrate with portable data.
