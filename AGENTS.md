# LexWisp — Agent Instructions

## Product

LexWisp is a **lightweight native Windows x64 floating AI chat tool** built with Rust, GPUI, and Longbridge GPUI Kit. Its job is simple: open quickly from a global hotkey or tray icon, chat with an OpenAI-compatible provider, preserve local conversations, then get out of the way.

The product has one popup window. Chat, history, and settings live inside it. The popup may be hidden while a request continues in the background.

Do not rebuild the old broad product. LexWisp has **no runtime plugin system, selection-first actions, action palette, source-text replacement workflow, secondary workspace, generic Agent/MCP framework, or speculative extensibility layer**. Add a future built-in tool or Agent capability only when a concrete requirement asks for it.

Prefer the smallest design that solves the current feature.

## Architecture boundaries

| Crate | Owns |
| --- | --- |
| `lexwisp-core` | IDs, DTOs, rules, state machines, typed ports |
| `lexwisp-platform-windows` | Win32, tray, hotkey, credentials, native window integration |
| `lexwisp-storage` | versioned TOML, SQLite, migrations, backups |
| `lexwisp-host` | ChatController, providers, run/task supervision, shared services |
| `lexwisp-ui` | GPUI popup, Chat/history/settings views, transient view state |
| `lexwisp-app` | composition root, startup, resources, packaging |

`lexwisp-core` must not depend on GPUI, Win32, HTTP, or SQLite. UI must not reach through typed ports into Host implementation details.

Host owns the reusable Tokio runtime, HTTP client, storage, credentials, provider services, and background run supervision. Do not add a service locator, global event bus, reflection registry, universal JSON invoke layer, or a global `Arc<Mutex<AppState>>`.

ChatController owns conversation/message identity and authoritative chat state. Storage owns persisted data. Views own only transient UI state such as focus, selection, scrolling, and overlays.

## Resource and memory discipline

Rust memory safety is not permission to waste memory. Code must remain bounded and have explicit ownership.

- Every long-lived resource needs a clear owner and end of life: task, subscription, timer, channel, cache, request, file buffer, window/view state, and native handle.
- **No detached or forgotten tasks.** Do not fire-and-forget `tokio::spawn`, `cx.spawn`, or equivalent work. Retain a handle or cancellation token and cancel/finish it when its owner is replaced, dropped, or the app shuts down.
- Background work that must survive popup destruction is Host-owned, never secretly kept alive by a hidden view.
- **No unbounded queues.** Use bounded channels with an explicit overload policy. A slow UI must recover from the latest/versioned state instead of accumulating infinite deltas.
- **No unbounded in-memory collections.** Any `Vec`, `VecDeque`, `HashMap`, cache, log, history, or pending-work collection that can grow from user/network activity must define a limit, eviction/pagination rule, or persistence boundary.
- Conversation history belongs in SQLite and is loaded incrementally. Do not keep the whole database or every rendered conversation resident in memory.
- Caches must be bounded and invalidatable. Markdown/layout/image caches must use stable keys and a deliberate retention policy.
- Do not keep attachment bytes in long-lived UI state. Validate metadata/limits before reading, keep bytes only as long as required for the request, and drop them afterward.
- **Do not clone to silence the borrow checker.** Avoid repeated `.clone()` of message bodies, conversations, Markdown text, vectors, or attachment buffers, especially in render and streaming paths. Borrow where possible. Use `Arc<str>`, `Arc<[u8]>`, IDs, or another shared immutable representation only when ownership is genuinely shared.
- Do not wrap broad state in `Arc<Mutex<_>>` merely to make code compile. Keep state ownership narrow and typed.
- Avoid `Arc`/`Rc` reference cycles. Prefer parent ownership plus IDs or `Weak` back-references where a back-reference is necessary.
- Streaming must not clone/rebuild the full assistant body for every token. Append efficiently, coalesce UI updates around 33 ms, and flush terminal states immediately.
- Render code describes UI only. It must not perform I/O, database/network calls, spawn tasks, create subscriptions, generate persistent IDs, or repeatedly allocate large buffers.
- Create InputState, focus/scroll handles, subscriptions, and other stateful GPUI entities once in their owner, not on each render.
- Hiding the popup is not a reason to retain expensive UI forever. Respect the configured hidden-window retention period; after it expires, release popup-scoped views/resources while Host-owned chat work continues.
- Do not add a hidden keeper window or invisible heavyweight UI tree to preserve state.

When changing long-lived state, streaming, history, caching, attachments, or window lifecycle, check Release memory behavior. Repeated show/hide, conversation switching, and completed/cancelled requests should settle rather than show unexplained monotonic growth. Measure Working Set, Private Bytes, handles/threads, and GPU memory separately; never trim a working set to disguise growth.

## GPUI and UI

`docs/design-system/DESIGN.md` is the visual and interaction source of truth. Before visible UI changes, also inspect `crates/lexwisp-ui/src/theme.rs`, `ui_metrics.rs`, locked GPUI Kit APIs, and the relevant reference under `examples/lexwisp-ui-lab/`.

The UI lab is reference material. Do not modify or depend on it unless the task explicitly targets the lab.

- Longbridge GPUI Kit is the default component source. Reuse its inputs, buttons, dialogs, menus, themes, scrolling, lists, selection, and Root before creating product-local equivalents.
- Do not copy upstream components into LexWisp or create a generic component framework.
- Product colors live in `theme.rs`; views use semantic roles and shared metrics.
- The popup is the only native product window. History and settings are pages in that popup.
- Window creation contains no business logic.
- Keep GPUI values on their owning thread and reject stale background UI updates using request/window generations where needed.
- Virtualize or page large lists. Scrolling away from the bottom disables chat auto-follow.
- IME confirmation must not send. Enter sends, Shift+Enter inserts a newline, and Escape resolves overlays before hiding the popup.
- Only the composer accepts file drops. Never send local file paths to a provider.

## Async, network, storage, and Windows

- Reuse the Host runtime and HTTP client. Do not create a runtime or client per request.
- Blocking SQLite work runs off the UI thread.
- A run has one supervisor and exactly one terminal state. Reject stale/post-terminal deltas. Cancellation preserves partial output.
- OpenAI-compatible requests preserve configured Base URL prefixes and parse SSE across chunk/UTF-8/JSON boundaries. Keep partial text on interruption. Do not retry paid POSTs automatically, disable TLS checks, or forward credentials across origins.
- Settings use versioned TOML and atomic saves. SQLite stores content. Preserve corrupt/newer data instead of silently resetting it.
- API keys live only in Windows Credential Manager. Logs and diagnostics contain no keys or message bodies.
- Paths must not depend on cwd and must support Unicode/spaces. `portable.flag` selects executable-adjacent data; otherwise use `%LOCALAPPDATA%/LexWisp`.
- Keep native Windows code narrowly encapsulated with explicit ownership/release rules. Hotkey replacement must register the new shortcut before releasing the old one. Restore the tray icon after Explorer restarts.
- The global hotkey only toggles LexWisp. It never captures selected text.

## Code quality

Read and reuse existing code before adding abstractions. Prefer concrete types, enums, guard clauses, and small typed interfaces. Avoid forwarding layers, speculative architecture, unrelated refactors, and duplicated state.

Do not `unwrap` fallible user input, files, network, database, credentials, or Windows operations. Surface actionable failures without corrupting or discarding user data.

A change is not acceptable merely because it compiles. For resource-sensitive code, explicitly check ownership, cancellation, growth bounds, and repeated-use behavior.

## Validation

Test real risks and regressions rather than getters, labels, or upstream-library behavior. Never use live API keys in automated tests.

Run:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked --target x86_64-pc-windows-msvc
cargo clippy --workspace --all-targets --locked --target x86_64-pc-windows-msvc -- -D warnings
cargo test --workspace --locked --target x86_64-pc-windows-msvc
cargo build -p lexwisp-app --bin LexWisp --release --locked --target x86_64-pc-windows-msvc
```

Validate the Release build from the staged/extracted directory. Packaging must contain only explicit runtime files and required notices, never developer data, logs, caches, or credentials.

Record meaningful commands, acceptance results, artifacts, and performance measurements in `docs/implementation-log.md`. Distinguish verified results from checks that could not be run; never fabricate screenshots, measurements, or platform behavior.
