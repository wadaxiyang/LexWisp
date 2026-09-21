# LexWisp UI Rules for Coding Agents

These rules may be appended to the project `AGENTS.md`.

## Source of truth

Before editing LexWisp UI:

1. Read `DESIGN.md`.
2. Read `src/theme.rs` and `src/ui_metrics.rs`.
3. Inspect the existing nearby GPUI-Kit component usage.
4. Reuse the current Main Shell / ShellPresentation architecture.
5. If reference screenshots exist, inspect them before changing composition.

Do not invent a second visual system.

## GPUI-Kit first

LexWisp uses GPUI-Kit.

Prefer existing:

- `Root`
- `Button`
- `Input`
- `Textarea`
- `Message`
- `MessageScroller`
- `Attachment`
- `Popover`
- `Dialog`
- `Tooltip`
- virtual list primitives

Do not hand-build controls already available in GPUI-Kit.

## No raw colors in views

No raw:

```text
hex
rgb
rgba
hsla
```

outside `lexwisp-ui/src/theme.rs`, except dedicated syntax highlighting data if added later.

Use semantic `cx.theme()` roles.

## Do not solve layout with cards

Do not add a bordered/rounded container just because several elements are adjacent.

First use:

- spacing
- alignment
- typographic hierarchy
- one shared surface
- one structural divider

Persistent panels should not have shadows.

## Preserve the Shell model

The Main Shell is neutral and host-owned.

```text
Compact
Expanded
Workspace
```

are presentation states, not separate Chat windows.

Do not create a second native Chat window.

Do not rename Shell primitives around the current Experience.

## Chat presentation

Normal assistant content should be visually light, generally using a Ghost conversation surface.

User content may use a restrained tinted/secondary bubble.

Primary accent must not become the background of every message or row.

## Composer

Composer is the primary local surface.

- retained Textarea state
- semantic raised surface
- one border
- Attach / Model / status / Send live in one control region
- no permanent `Ready`
- no duplicate inner cards

## Render discipline

`render` describes UI.

Never perform in render:

- I/O
- database operations
- provider calls
- file reads
- plugin loading
- subscription creation
- task creation
- random ID generation

Stateful GPUI entities are created once by their owner.

## Stable IDs

Use semantic IDs such as:

```text
send-message
chat-model-trigger
workspace-conversation-list
conversation-{id}
```

Do not derive identity from transient row positions where a stable domain ID exists.

## Spacing and geometry

Use the 4 px rhythm and shared `ui_metrics`.

Do not sprinkle arbitrary one-off pixel values through a view.

Use pixel metrics for window/chrome alignment. Use rem-based width only where text-flow behavior benefits from it.

## Accessibility

Every icon-only control needs a tooltip/accessibility label.

Keyboard focus must stay visible.

Do not use color alone to communicate state.

Test Light and Dark appearances.

## Change policy

A UI PR should not simultaneously rewrite business architecture unless the UI cannot function otherwise.

When a requested design conflicts with GPUI-Kit behavior:

1. confirm the upstream component API
2. prefer composition
3. prefer theme refinement
4. add a small custom component only as the last option

Do not guess an API from React/CSS conventions.

