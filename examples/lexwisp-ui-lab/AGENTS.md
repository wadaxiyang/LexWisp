# LexWisp UI Lab — Agent Instructions

## Scope

This directory is an independent GPUI-Kit UI design workspace. Work only inside `examples/lexwisp-ui-lab/` unless the user explicitly asks for a separate parent-project change.

- Do not add this crate to the parent LexWisp workspace.
- Do not depend on `lexwisp-core`, `lexwisp-ui`, another parent crate, or parent source files.
- Do not modify, format, migrate, or regenerate files outside this directory to make a lab example work.
- Parent-project files may be read as references only. Adapt ideas into lab-local files instead of creating cross-directory imports or symlinks.
- Keep generated artifacts under this directory's ignored `target/` folder. Do not put lab output into the parent `dist/` or packaging flow.

The lab exists to explore concrete UI examples. It is not a second product implementation: do not add networking, persistence, Provider calls, Windows integration, plugin execution, or speculative backend services unless the user explicitly asks for them. Use deterministic local mock state when an example needs data.

## Sources of truth

Before editing a visible UI, read in this order:

1. this `AGENTS.md`;
2. `design-system/DESIGN.md` in full;
3. `theme.rs` and `ui_metrics.rs`;
4. the nearest example and its tests;
5. the installed `gpui-kit` and `gpui-kit-design-guides` skills, including every reference they require for the task;
6. the locked GPUI-Kit 0.6.1 source and a matching upstream example for every API being introduced.

The user's current request is authoritative. Otherwise, `design-system/DESIGN.md` defines LexWisp's visual and interaction language. Do not invent a parallel design system.

## Dependency and workspace discipline

- This is a standalone Cargo workspace with its own `Cargo.lock`.
- Keep the lab-local Rust 1.95.0 toolchain and Windows MSVC target unless the user explicitly requests an upgrade.
- Keep `gpui-kit = "=0.6.1"` pinned unless the user explicitly requests and reviews an upgrade.
- Import the GPUI family only through `gpui_kit`; do not mix direct `gpui`, `gpui-component`, Git, path, or differently versioned sources.
- Prefer one source file per independently runnable example. Register each example as an explicit `[[bin]]` target.
- Shared lab code is limited to genuine common policy such as `theme.rs` and `ui_metrics.rs`. Do not grow a second general-purpose component library.

## LexWisp visual contract

LexWisp is a fast native AI workbench. Its UI is compact, precise, calm, Windows-first, and technically capable. Dark mode is a restrained graphite workstation; light mode is a crisp cool-neutral utility. Both appearances share geometry and hierarchy.

Design structure before decoration:

1. current task or Composer;
2. current result or conversation;
3. navigation;
4. secondary controls;
5. settings and administration.

Use one dominant action per region. Accent is scarce and reserved for the real primary commit, keyboard focus, links, and meaningful progress. Persistent surfaces are flat and quiet; use spacing, alignment, type hierarchy, and one structural divider before adding a card, border, color, or shadow.

The current Chat surface is one main window:

```text
LexWispMainWindow
└── Conversation
    ├── Header and in-window navigation
    ├── Transcript or history
    └── Retained Composer
```

- The main window starts directly in the 680 × 640 DIP Conversation layout. Do not reintroduce a Compact/Prompt presentation or an expansion transition.
- The transcript gets the vertical space and the retained Composer stays anchored at the bottom. Navigation opens inside the same window; there is no permanent sidebar.
- Switching conversations or opening history preserves the retained Composer entity and does not open another Chat window.
- `chart_main_window` remains an independent visual reference example, not another presentation of this Chat window.

## Theme, spacing, and geometry

- Raw product colors belong only in `theme.rs`. Views must not contain hex, `rgb`, `rgba`, or `hsla` literals.
- Use semantic `cx.theme()` roles. The Composer uses `group_box` with the semantic `input` border; normal assistant content uses a Ghost conversation surface; user content uses a restrained Tinted or neutral surface; failures use Destructive.
- Use GPUI-Kit size variants and rem-based scale helpers for ordinary layout. Use `ui_metrics` fixed values only for the documented Shell/window/chrome geometry that must remain exact.
- Follow the 4 DIP rhythm and the relationships defined by the design contract. Do not sprinkle one-off spacing values through a view.
- Use moderate radii. Ordinary controls are not pills; `radius_full` is reserved for true chips, badges, and circles.
- Borders define structure. Persistent panels do not receive shadows. Shadows are for floating popovers, menus, dialogs, and overlays.
- Use the platform-resolved UI font. Typography establishes hierarchy before color does.
- Verify both Light and Dark. No stock GPUI-Kit color should leak through an unmapped semantic role.

## GPUI-Kit first

Use standard GPUI-Kit behavior and components before custom primitives, including:

- `Root`, one per native window;
- `Button`, `Input`, `Textarea`, selection controls, and forms;
- `Message`, `MessageScroller`, `Bubble`, `Attachment`, and `AttachmentGroup`;
- `Popover`, menus, dialogs, sheets, notifications, and tooltips;
- lists and virtual lists for long collections.

Do not recreate a button, input, select, menu, dialog, tooltip, scroll area, message scroller, or virtual list from generic `div`s merely to match a screenshot. Confirm every API against the locked source; never infer a Rust method from React, CSS, Shadcn, or an older GPUI example.

## State and render discipline

- Create `InputState`, `TextareaState`, focus handles, scroll handles, subscriptions, and stateful `Entity` values once in their owner.
- `render` only describes the current frame. It performs no file or network I/O, subscriptions, task creation, random ID generation, or unconditional mutation/notification.
- Use `RenderOnce` for value-like presentation and `Entity<T>` when behavior or identity survives frames.
- Use stable semantic IDs such as `send-message`, `chat-model-trigger`, `workspace-conversation-list`, and `conversation-{id}`. Never use a transient row index or random value where domain identity exists.
- Store each state in its narrowest correct owner. Controlled callbacks request a change; the owner updates once and calls `cx.notify()` once.
- Keep async tasks owned, cancellable, and retained. Do not detach work merely to make an example compile.
- Virtualize long lists and avoid cloning whole transcripts in render callbacks.

## Interaction and accessibility

- Every control needs coherent rest, hover, pressed, focus-visible, selected, disabled, loading, and error states where applicable.
- Every icon-only control requires a tooltip and accessible name.
- All essential actions are keyboard reachable. Focus remains visible and returns to the trigger after an overlay closes.
- Escape dismisses the topmost overlay. Enter commits only the real default action. In the Composer, Shift+Enter inserts a newline and IME candidate confirmation must not send.
- Do not communicate status by color alone. Do not show routine `Ready`; show generating, stopping, saving/unsaved, errors, attachment state, or context warnings only when meaningful.
- Buttons in examples must either perform a real local state transition or be visibly disabled with an explanation. Do not ship clickable no-op controls.
- Use concise sentence-case labels. Internal commands are Buttons or native navigation components; Link styling is only for external URLs or email addresses.
- Destructive confirmation names the object and consequence, with `Cancel` and a precise destructive verb.
