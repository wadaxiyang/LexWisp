# Integration Notes

This pack is written against the current LexWisp Main Shell architecture and **GPUI-Kit 0.6.1**.

It is intentionally not a monolithic patch. Apply it in small, reviewable changes so visual changes do not accidentally disturb Shell lifecycle, Chat state, IME, or request semantics.

---

## 1. Add files to `lexwisp-ui`

Copy:

```text
src/theme.rs
src/ui_metrics.rs
```

to:

```text
crates/lexwisp-ui/src/theme.rs
crates/lexwisp-ui/src/ui_metrics.rs
```

Export them from `crates/lexwisp-ui/src/lib.rs`:

```rust
pub mod theme;
pub mod ui_metrics;
```

Keep raw application colors in `theme.rs`.

---

## 2. Replace the local `surface.rs` theme helper

The current `crates/lexwisp-ui/src/surface.rs` owns a local `apply_theme`.

Replace that helper with:

```rust
use crate::theme::apply_theme;
```

Call:

```rust
apply_theme(preference, window, cx);
```

at the same window creation/show points.

### Important system-theme observer

The current appearance observer calls `Theme::change(...)` directly.

That would reset the GPUI-Kit theme and lose LexWisp's overrides.

Change the observer to re-run the LexWisp projection:

```rust
window.observe_window_appearance(move |window, cx| {
    if settings.snapshot().settings().theme() == ThemePreference::System {
        crate::theme::apply_theme(ThemePreference::System, window, cx);
    }
})
```

Do not call only `Theme::change(...)` after this design system is installed.

---

## 3. Use shared Shell dimensions

`surface.rs` currently hardcodes:

```text
680 × 190
680 × 640
1180 × 780
```

Those values already match this design system.

Move their source of truth to `ui_metrics.rs`:

```rust
match presentation {
    ShellPresentation::Compact => ui_metrics::compact_shell_size(),
    ShellPresentation::Expanded => ui_metrics::expanded_shell_size(),
    ShellPresentation::Workspace => ui_metrics::workspace_shell_size(),
}
```

The native bounds logic, monitor clamping, negative-coordinate support, and retained transition behavior should remain unchanged.

---

## 4. Current `ChatExperience` visual corrections

File:

```text
crates/lexwisp-plugins-builtin/src/view.rs
```

### 4.1 Composer

Current composer uses:

```rust
.bg(cx.theme().background)
```

Change the contained composer surface to:

```rust
.bg(cx.theme().group_box)
.border_color(cx.theme().input)
```

The outer canvas remains `background`.

Do not add another inner border around the Textarea unless GPUI-Kit requires it for accessibility.

### 4.2 Message variants

Current normal assistant message:

```rust
BubbleVariant::Muted
```

Change to:

```rust
BubbleVariant::Ghost
```

Current user message:

```rust
BubbleVariant::Filled
```

Prefer:

```rust
BubbleVariant::Tinted
```

Failed assistant output remains:

```rust
BubbleVariant::Destructive
```

This one change removes a large amount of “AI-generated dashboard” visual weight.

### 4.3 Routine status

Do not permanently render `Ready`.

Show status text only for states with actual information, such as:

- Generating
- Stopping
- Saving / Unsaved
- error
- attachment loading/error
- context budget warning

### 4.4 Header

For Compact:

- avoid a heavy permanent bottom rule if the layout reads clearly without it
- keep identity weak
- keep close action quiet

Expanded/Workspace may retain one structural separator.

### 4.5 Workspace transcript width

The current Workspace composer is already centered and width-limited with a rem-based max.

For alignment-critical Shell geometry, prefer a shared maximum around:

```text
840 DIP
```

Transcript and composer should visually align to the same content column.

---

## 5. Sidebar

The current `w_64()` is approximately the correct visual scale.

Do not add more wrappers. Refine it toward:

```text
248–264 DIP
```

Conversation rows:

- transparent default
- semantic hover
- semantic selected state
- no card border
- truncate title
- local context actions only when needed

---

## 6. GPUI-Kit components remain authoritative

Continue using:

```text
Button
Input
Textarea
Message
MessageScroller
Attachment / AttachmentGroup
Popover
Dialog
Tooltip
v_virtual_list
Root
```

Do not replace them with custom primitives solely to achieve the visual style.

The theme file exists specifically so standard GPUI-Kit controls inherit LexWisp's visual language.

---

## 7. Build and review sequence

After adding the theme:

```bash
cargo fmt --all
cargo check --workspace
cargo clippy --workspace --all-targets
```

Then visually inspect:

```text
Compact Light
Compact Dark
Expanded Light
Expanded Dark
Workspace Light
Workspace Dark
```

At minimum also inspect Windows scaling at:

```text
100%
125%
150%
```

Visual work is incomplete until both appearances are reviewed.

---

## 8. Do not combine styling refactor with business refactor

While applying this pack, avoid changing:

- ChatController semantics
- conversation persistence
- attachment encoding
- provider request flow
- HiddenWarm lifecycle
- Main Shell window identity
- bounds-transition ownership

The design pass should remain presentation-focused.

