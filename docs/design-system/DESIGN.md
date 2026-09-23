# LexWisp Design System

This is the normative visual and interaction contract for the LexWisp product popup. Its visual identity is **Fluent shell + Zed content**. The independent UI lab is a historical reference, not a product dependency or a competing design system.

## 1. Product shape

LexWisp has one 680 × 640 DIP floating native window. Its pages are Chat, History, Settings, and About. Launch starts in the notification area without opening the popup. The popup may be hidden while the process continues through the tray and hotkey. Chat keeps the same conversation and in-flight run when hidden or reopened. There is no workspace presentation, second native panel, action chooser, or selected-text capture.

The global hotkey toggles the popup. A left click on the notification-area icon opens Chat. Its right-click menu opens Chat, Settings, or About, or quits the process. Launching the executable again leaves the running instance in its current state. The shell, content canvas, working surfaces, and overlays form a clear hierarchy. Keep the visual center on the answer and composer, with Send as the primary action. Settings and History are accessible but visually quieter.

## 2. Sources of truth

- `crates/lexwisp-ui/src/theme.rs` owns raw product colors and light/dark theme projection.
- `crates/lexwisp-ui/src/ui_metrics.rs` owns window, header, spacing, typography, border, and radius metrics.
- The locked GPUI Kit 0.6.1 family provides controls and layout primitives. Import GPUI through `gpui_kit` only.
- The product view in `crates/lexwisp-ui/src/chat.rs` translates the lab composition to real capabilities. Keep any lab-only demonstration behavior out of the product.

Views use `cx.theme()` semantic roles. `title_bar` is the shell, `background` the stable content canvas, `group_box` the selective raised working surface, and `popover` the overlay. Do not put hex/rgb/hsl colors into view code. Keep one 4 px spacing rhythm, 6 px control radii, 8 px surface radii, and 10–12 px overlay/composer radii. Borders define structure; permanent panels have no shadow. Accent marks focus, selection, links, and the primary action rather than decorating whole regions. Both themes use the same geometry.

## 3. Window and header

The popup has a 42 DIP shell header. Its title is plain text with a subtle switcher affordance, and its unused center is draggable. New chat, Settings, and Pin are compact secondary actions. Close occupies a Windows caption-sized target, uses `WindowControlArea::Close`, and turns destructive red on hover. The existing close interception hides the popup and retains its warm window for the configured interval. Icon-only controls need accessible labels and tooltips. Esc closes the model menu first, then the conversation switcher, then returns from History/Settings to Chat, then hides the popup.

Keep one top-level Kit `Root` with its standard dialog, sheet, and notification layers. Dialogs and menus belong inside that window. The window is centered in the active display's visible work area; coordinates may be negative. Do not add a hidden keeper window or a second Chat window.

## 4. Chat page

The transcript occupies the flexible canvas. Empty Chat shows only LexWisp and a short prompt. Assistant output reads as document content directly on the canvas, with comfortable line height, compact Markdown headings, distinct monospace code surfaces, and no enclosing bubble. User content is a compact neutral tinted surface on the right. Turns separate through whitespace. Preserve selectable Markdown, answer copy, and code-block copy. Status appears only when useful: generating/stopping, partial failure, context warning, attachment error, save failure, or an unsaved result. Completed output becomes ordinary content.

Use Kit `MessageScroller` for message bodies. A user who scrolls upward must not be forced to the bottom by streaming. Coalesce UI updates around 33 ms and flush terminal states immediately. Avoid rebuilding message bodies without a content/version change.

The composer is the strongest persistent working surface below the transcript: one semantic raised background and border, visually integrated Textarea, attachment strip, quiet model trigger, and Send/Stop/Retry. Only Send has a strong accent. Focus is visible around the composer without nested bright outlines. The Textarea retains its entity and focus state. Enter sends a committed message; Shift+Enter inserts a newline; an IME candidate confirmation must not send. Disable Send for empty content or while the active conversation cannot send.

Only the composer drop zone accepts files. Image and text attachments have visible names/previews and removal controls. Inspect type and size before reading, then send content rather than local paths. Any error appears near the composer and leaves the draft usable.

## 5. Conversation switcher and History

The header's conversation control and Ctrl+K open a popover over Chat. It contains search, New chat, recent conversations, and a link to full History. Search and conversation identity come from ChatController; do not maintain a second conversation store in the view.

History is a page inside the popup with search, Recent/Favorites filters, stable flat conversation rows, and a clear selected state. Favorites persist. Rename and Delete are real operations; Delete requires confirmation. Long lists should be paged or virtualized. A row uses neutral hover fill and truncated title rather than a bordered card.

## 6. Settings page

Settings is inside the same popup. Group controls by General, Provider and model, and History/data. General uses compact Windows-style setting rows with labels and right-aligned values or controls. The provider form remains technical but orderly, with consistent field height and rhythm. General includes hotkey, Windows appearance, startup, local recording, and hidden-window retention. Provider controls include Base URL, model ID, optional Bearer key, stream, timeouts, proxy, context budget, temperature, and output limit. Test performs a real request. Save persists a valid provider and key through the designated services; the API key input is masked and cleared after saving. Backup creates a consistent local SQLite backup. Busy/error feedback stays near its section.

Do not present implementation concepts such as registries, packages, or scripting to product users. Do not show a clickable control unless it performs its named operation.

## 7. Typography, motion, and accessibility

Use shared body and caption scales from `ui_metrics`; reserve display sizing for genuine empty states. The accent marks Send, focus, and selected state sparingly. Light and dark appearances share geometry. Use semantic theme roles so Windows appearance changes reapply LexWisp's theme projection.

Keep keyboard navigation and visible focus across Chat, overlays, History, and Settings. Icon-only controls require labels and tooltips. Never communicate state only by color. Avoid animation that delays sending, stopping, or hiding. Layout must remain usable at 100%, 125%, and 150% display scaling and on a small work area.

## 8. Implementation discipline

Render describes UI and does no I/O, file reads, random ID creation, task creation, or subscription setup. Stateful Kit entities and subscriptions are owned once by their view. Background updates use weak view references and respect surface/request lifetime. Keep the Host's typed ports and ChatController as state owners. UI code does not branch on backend implementation details.

Before changing UI, read the GPUI Kit skills and required references, this document, theme, metrics, the locked Kit source, and the relevant lab sections. After changes, run formatting, check, Clippy, and tests; launch the packaged Release build for visual and native behavior checks. Record which light/dark, DPI, IME, and Windows checks were actually performed.
