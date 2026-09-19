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
