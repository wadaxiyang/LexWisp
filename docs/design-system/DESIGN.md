# LexWisp Design System

**Version 1.0 · Windows-first · GPUI-Kit 0.6.1 · September 2026**

This document is the visual contract for LexWisp. It is not a mood board and it is not a collection of optional suggestions. New UI work should satisfy these rules before local styling is invented.

LexWisp is a **fast native AI workbench**. Its interface should feel compact, precise, calm, and technically capable. The product is allowed to be dense, but never cluttered. Dark mode should read as a restrained graphite workstation. Light mode should read as a crisp, neutral Windows utility with clear white working surfaces and quiet gray navigation.

The same geometry and hierarchy must work in both appearances. Theme changes alter tone, not information architecture.

---

## 1. Design principles

### 1.1 Structure before decoration

A screen must remain understandable when accent color, shadows, and decorative icons are mentally removed.

Priority order:

1. current task or composer
2. current result or conversation
3. navigation
4. secondary controls
5. settings and administration

Do not solve weak hierarchy by adding more color.

### 1.2 One dominant action per region

Each region should make its next action obvious.

Examples:

- Compact Shell → compose or send
- Expanded Shell → continue the conversation
- Workspace Sidebar → switch or create a conversation
- Dialog → confirm or cancel

Secondary actions should use ghost, outline, overflow, context menu, or message-local controls.

### 1.3 Quiet surfaces

Persistent UI should be built from a small number of surfaces:

- canvas
- navigation surface
- working surface
- elevated/floating surface
- interactive state wash

Do not turn every paragraph, message, toolbar group, or setting into a card.

### 1.4 Borders define structure, not decoration

Use a 1 px border only where two structural regions need separation.

Good:

- sidebar / workspace boundary
- composer boundary
- popover boundary
- dialog boundary
- input boundary

Bad:

- a border around every message
- nested bordered cards
- a border plus a shadow plus a different background for the same hierarchy level

### 1.5 Accent is scarce

The primary accent is reserved for:

- primary action
- keyboard focus
- selected micro-indicator when a neutral selection wash is insufficient
- links
- progress that requires emphasis

The accent is not a general-purpose decoration color.

### 1.6 Windows-first interaction

LexWisp targets Windows first.

- Preserve predictable Windows window controls and Snap behavior.
- Do not imitate macOS traffic-light controls or macOS sidebar chrome.
- Use system UI font resolution rather than a platform-specific font name.
- Keyboard focus must remain visible.
- Hover states should be clear with a mouse, but every essential action must also be keyboard reachable.
- Layout must remain correct at 100%, 125%, 150%, and 200% scale.

---

# 2. Product surface model

LexWisp has one host-owned **Main Shell** and one current **Experience**.

```text
Main Shell
    │
    ├── Presentation
    │      ├── Compact
    │      ├── Expanded
    │      └── Workspace
    │
    └── Experience
           └── Chat, currently
```

The Shell owns:

- native window
- outer background
- global chrome
- presentation transition
- workspace navigation area
- placement and resize behavior
- global overlays
- theme

The Experience owns:

- conversation or task content
- composer semantics
- message/task actions
- experience-specific state

Do not encode Chat-specific names into Shell styling primitives. Future experiences must be able to reuse the same Compact → Expanded → Workspace grammar.

---

# 3. Main Shell presentations

## 3.1 Compact

**Reference size:** 680 × 190 DIP  
**Allowed normal range:** 640–720 × 160–220 DIP

Compact is a launcher-like AI entry surface. The composer is the visual anchor.

Required content:

- minimal LexWisp identity or context
- composer
- optional selection/context chip
- attachments
- attach action
- model/tool selector if needed
- Send
- Close or overflow

Do not show:

- conversation history
- permanent action-chip rows
- a large page title
- a permanent `Ready` status
- independent Settings button at primary hierarchy
- source preview card
- separate toolbar and footer

### Compact composition

```text
┌─────────────────────────────────────────────────────────┐
│ LexWisp                                             ×   │
│ ┌─────────────────────────────────────────────────────┐ │
│ │ Ask LexWisp…                                       │ │
│ │                                                     │ │
│ │ +   Context / Attachments   Model              ↑   │ │
│ └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

The outer Shell and composer may both be rounded, but they must not look like two unrelated stacked cards. The composer carries the stronger local boundary.

---

## 3.2 Expanded

**Reference size:** 680 × 640 DIP

Expanded is a narrow, vertically oriented continuous interaction surface.

```text
┌─────────────────────────────────────────────────────────┐
│ LexWisp     Current conversation                  ⫶  ↗ ×│
│                                                         │
│  conversation transcript                                │
│                                                         │
│  conversation transcript                                │
│                                                         │
│ ┌─────────────────────────────────────────────────────┐ │
│ │ Composer                                            │ │
│ └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

Rules:

- Transcript receives most of the vertical space.
- Header is quiet and no taller than necessary.
- Composer remains visually anchored to the bottom.
- There is no sidebar.
- Model choice stays inside the composer control zone.
- Rename/Delete/Settings are not permanent top-level buttons.
- Empty-state copy must be short and centered.
- Status is shown only when meaningful.

---

## 3.3 Workspace

**Reference size:** 1180 × 780 DIP  
**Recommended minimum:** 920 × 620 DIP  
**Sidebar target:** 248–264 DIP  
**Main conversation content max width:** ~840 DIP

Workspace reveals navigation while preserving the same active experience.

```text
┌───────────────────┬─────────────────────────────────────┐
│ LexWisp           │ Current conversation                │
│                   │                                     │
│ + New chat        │ transcript                          │
│                   │                                     │
│ Conversations     │ transcript                          │
│ Conversation A    │                                     │
│ Conversation B    │                                     │
│ Conversation C    │                                     │
│                   │       ┌───────────────────────┐     │
│                   │       │ Composer              │     │
│ Settings          │       └───────────────────────┘     │
└───────────────────┴─────────────────────────────────────┘
```

Rules:

- Sidebar is one continuous navigation surface, not a stack of cards.
- Main content has breathing room and should not touch both edges on wide windows.
- Conversation content is centered within the main region.
- The composer should not become arbitrarily wide.
- Workspace expansion should feel like revealing navigation, not launching another app.

---

# 4. Color system

All raw color literals belong in `theme.rs`. Product views must consume semantic GPUI-Kit theme roles.

## 4.1 Core palette

| Semantic role | Light | Dark | Purpose |
|---|---:|---:|---|
| Canvas / `background` | `#F7F8FA` | `#0F1115` | Main working canvas |
| Foreground | `#171A1F` | `#E7EAF0` | Primary text |
| Sidebar | `#F1F3F6` | `#11141A` | Navigation / title chrome |
| Elevated working surface | `#FFFFFF` | `#1A1F27` | Composer, contained work |
| Popover | `#FFFFFF` | `#1D232C` | Menus, popovers |
| Secondary | `#EDF0F4` | `#1B2028` | Neutral control fill |
| Muted | `#EDF0F3` | `#1A1F27` | Quiet chips and low-emphasis surfaces |
| Muted foreground | `#68717D` | `#939EAC` | Secondary text |
| Border | `#E1E5EA` | `#2A303A` | Structural hairline |
| Input border | `#D6DCE4` | `#343C49` | Input/composer boundary |
| Primary | `#3567CE` | `#6689DF` | Primary action and focus |
| Primary hover | `#4577D9` | `#7698EA` | Hover |
| Primary active | `#2D5AB5` | `#5678C9` | Pressed |
| Neutral hover | `#ECEFF3` | `#181D24` | Row/control hover |
| Neutral selection | `#E6ECF8` | `#202B3E` | Selected row |
| Danger | `#D84955` | `#E26D78` | Destructive/error |
| Success | `#2F9B68` | `#4FC28B` | Success |
| Warning | `#A36C13` | `#D9A441` | Warning |
| Info | `#3278C8` | `#65A8E8` | Informational state |

The palette is neutral by default. Status colors are semantic and must never be used as decorative accents.

## 4.2 Dark appearance

Dark mode is a **graphite workstation**, not pure black.

Surface progression:

```text
#0F1115  canvas
#11141A  navigation
#1A1F27  contained work / composer
#1D232C  floating popover
#2A303A  structure line
```

Use slight luminance changes to create depth. Do not create depth by saturating every panel.

Dark hover/active treatment should feel like a soft light wash over graphite. Selected navigation may use a restrained blue-gray surface, but text remains neutral.

Avoid:

- pure `#000000` main canvas
- neon blue edges
- glowing cards
- high-saturation purple/blue gradients
- heavy glass blur everywhere

## 4.3 Light appearance

Light mode is **crisp neutral utility UI**.

Surface progression:

```text
#F7F8FA  canvas
#F1F3F6  navigation
#FFFFFF  working / composer surface
#FFFFFF  popover
#E1E5EA  structure line
```

White surfaces should be intentional. Do not make the entire app white with no zoning, and do not warm the palette toward beige.

Hover is a subtle cool gray. Selection is a quiet blue-gray, not a bright blue block.

---

# 5. Typography

Use the resolved Windows/system UI family through GPUI-Kit.

`theme.font_family = ".SystemUIFont"`

Recommended visual scale:

| Role | Size | Weight | Usage |
|---|---:|---:|---|
| Display | 22 px | 600 | Rare empty-state headline |
| Page title | 18 px | 600 | Settings or major page title |
| Section heading | 15 px | 600 | Group heading |
| Body | 14 px | 400 | Standard UI and prose |
| Compact body | 13 px | 400 | Message metadata, dense rows |
| Caption | 12 px | 400–500 | Secondary status |
| Micro label | 11 px | 500 | Category label only |
| Code | 13 px | 400 | Code/monospace content |

Rules:

- Default interface size is 14 px.
- Do not use bold for ordinary navigation.
- Use weight 600 for hierarchy; weight 700 is rare.
- Muted text is communicated by semantic color, not by shrinking important content below legibility.
- Long-form assistant text should keep comfortable line height, approximately 1.55–1.65.
- Do not center multi-paragraph conversation content.

---

# 6. Spacing

Use a **4 px base grid**.

Primary spacing tokens:

```text
4   micro gap
8   control-internal gap
12  compact padding
16  standard region padding
20  loose section spacing
24  major region padding
32  large separation
```

Rules:

- Alignment-critical shell geometry uses `px(...)`.
- Text-flow width may use `rems(...)` where appropriate.
- Prefer one consistent region padding over many local one-off values.
- Adjacent controls usually use 4–8 px gaps.
- Major groups need more space between groups than within a group.

---

# 7. Radius system

LexWisp uses **moderate radii**, not pill-shaped everything.

| Role | Radius |
|---|---:|
| Tiny chip / code tag | 4 px |
| Standard control | 6 px |
| Panel / attachment | 8 px |
| Dialog / popover | 10 px |
| Composer | 12 px |
| Compact outer shell | 14–16 px |
| Pill | full radius, only for true chips/badges |

GPUI-Kit theme defaults should use:

```text
theme.radius    = 6 px
theme.radius_lg = 10 px
```

Do not use `radius_full` for ordinary rectangular buttons.

---

# 8. Borders and shadows

## 8.1 Borders

Default border: **1 px**

Use borders for:

- composer
- input
- sidebar split
- popover/dialog
- attachment tile

Do not outline the whole transcript.

## 8.2 Shadows

Persistent application panels should not use shadows.

Shadows are appropriate for:

- popover
- menu
- dialog
- detached floating overlay
- Compact shell if native window rendering needs separation from the desktop

Even then, shadows should remain soft and low-contrast.

GPUI-Kit global shadow support can remain enabled so floating components retain platform-appropriate depth. Do not manually add shadows to every container.

---

# 9. Header and chrome

## 9.1 Header height

Target: **40–44 px** in Expanded/Workspace.

Compact may use a smaller visual header if window-control constraints allow it.

## 9.2 Header content

Expanded:

- product identity, weak
- current conversation title, muted
- overflow if needed
- Workspace expand
- Close

Workspace main header:

- current conversation title
- experience-local controls as needed
- Close only if consistent with Shell behavior

Do not put Rename, Delete, Settings, model selection, and close all in the same permanent row.

## 9.3 Window controls

On Windows, retain correct native hit targets and Snap behavior. Visual customization must not break standard window affordances.

---

# 10. Sidebar

Workspace sidebar is a single quiet navigation surface.

### Dimensions

- target width: 256 px
- acceptable: 248–264 px
- row horizontal padding: 8–12 px
- conversation row height: 36–42 px
- section label height: 24–28 px

### Hierarchy

```text
Product identity
New chat
Section label
Conversation list
flex spacer
Settings / bottom utilities
```

### Row states

| State | Treatment |
|---|---|
| Normal | transparent on sidebar |
| Hover | sidebar accent / neutral wash |
| Selected | quiet blue-gray selection |
| Focused | visible ring/border |
| Generating | subtle status indicator or text, not animated decoration |

Conversation rows should not each be cards.

Truncation is mandatory for long titles. Tooltips may reveal full titles.

Destructive actions belong in a context menu or row-local secondary action, not as a permanent bright red icon.

---

# 11. Composer

The composer is the **visual anchor** of the product.

## 11.1 Surface

Composer uses the elevated working surface:

- Light: white
- Dark: raised graphite
- 1 px input/structure border
- 12 px radius
- 12 px internal padding

Do not use the same flat canvas color for the composer in both modes. It needs enough contrast to remain discoverable.

## 11.2 Internal layout

```text
optional context / attachments
textarea
bottom control row
```

Bottom row:

```text
Attach | Model/Tool | flexible spacer | transient status | Stop/Retry | Send
```

Status text must disappear when it carries no information. Do not display `Ready` permanently.

## 11.3 Textarea

- No redundant inner card around the Textarea.
- No heavy border if the composer itself already supplies the boundary.
- Enter sends; Shift+Enter inserts newline.
- Use retained `TextareaState`.
- Keep IME stable across presentation transitions.

## 11.4 Send

Primary Send may be compact and icon-first.

Use the primary accent only here and in similarly high-value actions.

Disabled Send should be visibly disabled without becoming a large gray visual block.

---

# 12. Context and attachment chips

Context and attachments are subordinate to the draft.

Context chip:

- compact
- muted surface
- foreground or muted foreground
- full radius is acceptable
- removable
- one-line truncated preview

Attachment:

- use GPUI-Kit `Attachment` / `AttachmentGroup`
- 8 px local radius
- clear filename
- quiet metadata
- remove action appears as local secondary control
- image preview should not enlarge Compact Shell uncontrollably

Do not create a large permanent “source preview” card.

---

# 13. Conversation transcript

The transcript should feel like content, not a dashboard.

## 13.1 Assistant messages

Assistant content should normally **flow directly on the transcript background**.

Recommended GPUI-Kit treatment:

```rust
BubbleVariant::Ghost
```

Benefits:

- markdown gets full width
- code blocks breathe
- long answers do not become enormous gray cards
- interface remains lighter

Failed assistant output may use `BubbleVariant::Destructive`.

## 13.2 User messages

User messages may use a restrained differentiated surface.

Recommended:

```rust
BubbleVariant::Tinted
```

or a neutral Secondary treatment if accent becomes too visible.

Avoid a saturated primary-blue rectangle for every user message.

## 13.3 Message width

- assistant: may use the main content width
- user: max ~75–80% content width
- workspace transcript container: centered, max ~840 px
- expanded transcript: use available width with 16–24 px side padding

## 13.4 Metadata

Sender labels, timestamps, status, and copy controls are secondary.

Do not make `Complete`, `Generating`, or sender labels visually compete with the content.

Message actions should appear near the message and can be subdued until hover/focus.

---

# 14. Markdown and code

Assistant Markdown should favor reading.

- body text uses foreground
- headings use weight/spacing before extra color
- links use semantic link color
- inline code uses contained neutral surface
- code blocks use a distinct but subtle code surface
- code block actions are local
- quotes use a left structural rule rather than a large tinted card

Do not color ordinary prose.

---

# 15. Buttons

Use GPUI-Kit `Button` variants before custom button rendering.

### Primary

Use for:

- Send
- destructive-confirm replacement only when semantically appropriate
- major confirm action

### Secondary / outline

Use for meaningful secondary actions.

### Ghost

Default for:

- toolbar icons
- close
- sidebar utilities
- model selector
- message-local actions

### Button sizing

- standard utility control: 28–32 px visual height
- icon button: ~28–30 px
- Compact Shell should favor `small` / `xsmall`
- no oversized 40+ px desktop buttons unless they are the sole CTA in an empty state

---

# 16. Menus, popovers, and dialogs

Use GPUI-Kit overlay components through a single window `Root`.

Popover/menu:

- elevated surface
- 1 px border
- 10 px radius
- soft shadow
- stable width while searching/filtering when practical

Dialog:

- visually centered
- concise title
- short description
- one clear primary action
- destructive primary action only for destructive confirmation

Do not open a second native window for an interaction that belongs in a popover or dialog.

---

# 17. Inputs and settings

Inputs use GPUI-Kit `Input`, `Textarea`, `Select`, `Switch`, `Radio`, etc.

Settings page:

- one readable content column
- clear labelled groups
- 16–24 px between groups
- 8–12 px within a group
- labels are stronger than descriptions
- descriptions use muted foreground
- do not wrap each setting in an independent card unless the setting truly represents a standalone object

Focus must remain visible in both themes.

---

# 18. Empty states

Empty states are quiet.

Recommended structure:

```text
short heading
one sentence
optional single action
```

Do not use:

- giant illustration
- multiple action cards
- five prompt suggestions in permanent chips
- marketing copy inside a utility surface

---

# 19. Status and error feedback

Semantic status colors are reserved:

- danger → failure/destructive
- warning → degraded or caution
- success → confirmed success
- info → informational state

Prefer local feedback:

- provider error near composer
- attachment error near attachment workflow
- connection status near connection control

Do not add a global status bar solely to display `Ready`.

---

# 20. Interaction states

Every interactive control must specify:

1. normal
2. hover
3. active/pressed
4. focus-visible
5. selected, if applicable
6. disabled, if applicable

### Neutral state wash

Light mode:

- hover approximates a 4–6% dark wash
- active approximates an 8–10% dark wash

Dark mode:

- hover approximates a 7–10% light wash
- active approximates a 12–16% light wash

Prefer semantic theme roles rather than hardcoding alpha washes in every component.

---

# 21. Motion

Motion communicates **continuity**, not personality.

## 21.1 Shell morph

Current target:

- Compact ↔ Expanded: ~160 ms
- Expanded ↔ Workspace: ~190–220 ms
- easing: ease-out cubic

Workspace expansion should reveal space primarily toward the left so the active conversation feels anchored.

## 21.2 Content transition

When presentation changes:

- preserve the same Composer entity
- preserve current draft
- preserve focus where sensible
- preserve active conversation
- do not replay request animation
- do not destroy/recreate transcript state unnecessarily

## 21.3 Reduced motion

If a reduced-motion setting is introduced, bounds/content transitions should snap or shorten substantially.

Avoid:

- bouncing
- overshoot
- perpetual pulsing
- decorative shimmer outside loading skeletons

---

# 22. GPUI-Kit implementation contract

LexWisp uses **GPUI-Kit**, not a hand-built component library.

Prefer existing GPUI-Kit components:

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
- lists / virtual lists
- standard icons

Rules:

### 22.1 One Root per native window

Do not nest independent `Root`s to fix overlay problems.

### 22.2 Retain stateful entities

Create `InputState`, `TextareaState`, `MessageScrollerState`, focus handles, and subscriptions once in the owning view.

Do not recreate them in `render`.

### 22.3 Render is presentation

Do not perform:

- file I/O
- network I/O
- database work
- plugin loading
- subscription creation
- random ID generation

inside `render`.

### 22.4 Stable element IDs

IDs should describe stable UI identity, not frame order.

Good:

```text
send-message
chat-model-trigger
workspace-conversation-list
conversation-{conversation_id}
```

Bad:

```text
button-4
row-current-index
random UUID created in render
```

### 22.5 Theme-only color source

No `rgb(...)`, `rgba(...)`, raw hex, or arbitrary HSL literals in product views.

Exceptions:

- `theme.rs`
- syntax highlighting data if a dedicated syntax theme is later introduced
- image/media content itself

### 22.6 Do not fork GPUI-Kit controls for styling

First try:

- semantic theme
- component variants
- composition
- local style refinement

Only create a custom primitive when the interaction genuinely does not exist upstream.

---

# 23. Current LexWisp-specific visual corrections

The following corrections should be applied to the current Main Shell implementation.

## 23.1 Composer

Current behavior paints the composer with the general `background`.

Change it to the elevated working surface (`group_box` / the theme role defined for composer surfaces). Keep the border semantic.

Goal: composer reads as a deliberate control in both light and dark modes.

## 23.2 Assistant bubble

Current assistant messages use a muted filled bubble.

Change normal assistant messages to:

```rust
BubbleVariant::Ghost
```

Keep destructive treatment for failed output.

## 23.3 User bubble

Current user message uses `Filled`, which consumes the primary accent too aggressively.

Prefer:

```rust
BubbleVariant::Tinted
```

Evaluate `Secondary` if the tinted result remains too saturated.

## 23.4 Status

Suppress routine `Ready` text from the composer. Show only:

- generating
- stopping
- saving/unsaved
- provider error
- attachment error
- other actionable/transient state

## 23.5 Header

Compact header should have no heavy bottom divider if the native/window chrome already provides sufficient separation.

Expanded/Workspace may keep one structural divider.

## 23.6 Sidebar

Keep one sidebar boundary. Do not add per-conversation card borders.

Selected conversation should use a quiet selection surface rather than primary fill.

---

# 24. Responsive and DPI rules

The design must be tested at:

- 100%
- 125%
- 150%
- 200%

and on:

- 1920×1080
- 2560×1440
- a small work area where the preferred Workspace dimensions do not fit
- a monitor with negative desktop coordinates
- multi-monitor layouts

Rules:

- clamp Shell to visible work area
- preserve minimum useful content size
- truncate titles before controls disappear
- Sidebar may shrink within its allowed range before main content becomes unusable
- do not assume 1 physical pixel = 1 DIP

---

# 25. Accessibility

- Keyboard navigation for all primary actions.
- Visible focus in light and dark themes.
- Do not communicate status by color alone.
- Minimum body text contrast should remain suitable for normal desktop reading.
- Muted text must still be readable.
- Tooltips explain icon-only controls.
- Destructive confirmation names the object being deleted.
- Hit targets should remain comfortable even when icons are visually compact.

---

# 26. Anti-patterns

Do not ship any of the following without explicit design review:

- every section is a card
- every card has a border and shadow
- gradient background behind ordinary app content
- permanent prompt-suggestion chips
- saturated selection rows
- bright accent-colored sidebar
- 16+ px radius on ordinary controls
- nested scroll regions without a strong reason
- toolbar containing every available command
- independent styling for every plugin
- custom controls that duplicate GPUI-Kit
- raw colors in view code
- `rems()` used casually for alignment-critical Windows chrome
- placeholder buttons for features that do not exist
- a second native window for the same Main Shell experience
- visual changes that break IME, focus, or retained state

---

# 27. Visual review checklist

Before accepting a UI change:

### Hierarchy

- [ ] Is the primary task obvious without color?
- [ ] Is there one dominant action per region?
- [ ] Are secondary controls actually secondary?
- [ ] Did we avoid adding a card merely to group nearby content?

### Theme

- [ ] No raw colors outside `theme.rs`
- [ ] Light mode checked
- [ ] Dark mode checked
- [ ] Hover/selected/focus states checked
- [ ] Muted text remains readable
- [ ] No stock GPUI-Kit palette leaks through an unmapped component role

### Geometry

- [ ] Uses 4 px spacing rhythm
- [ ] Uses prescribed radii
- [ ] Header and sidebar dimensions remain stable
- [ ] Long labels truncate correctly
- [ ] Works at 125% and 150% scale

### GPUI

- [ ] Stateful Entity created once
- [ ] Stable element IDs
- [ ] No I/O in render
- [ ] Existing GPUI-Kit component reused
- [ ] One `Root` per native window
- [ ] Overlay is not clipped by an unnecessary parent

### Shell continuity

- [ ] Compact → Expanded preserves draft
- [ ] Expanded → Workspace preserves conversation
- [ ] No duplicate request
- [ ] No new native Chat window
- [ ] HiddenWarm reopening restores visual state correctly

---

# 28. Final visual target

LexWisp should read as:

> a native Windows AI utility that can start as a small, calm input surface and unfold into a capable workbench without changing its visual language.

Dark mode is deep, layered, and technical without becoming neon or game-like.

Light mode is clean, neutral, and fast without becoming sterile or overly macOS-like.

The interface earns complexity only when the user expands into it.

