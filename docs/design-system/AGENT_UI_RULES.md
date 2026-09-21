# LexWisp UI rules

1. Read `DESIGN.md`, theme, metrics, the relevant standalone UI lab sections, and the installed GPUI Kit skills and locked APIs before editing.
2. Keep one native 680 × 640 popup. Chat, History, and Settings are pages in it. Use one Kit Root and its overlay layers.
3. Use Kit controls, semantic `cx.theme()` roles, shared metrics, and a 4 px spacing rhythm. Raw product colors belong in `theme.rs`.
4. Create stateful input/scroller entities and subscriptions once. Render performs no I/O or task creation.
5. Preserve the real ChatController and typed ports. Never add demonstration-only UI or controls that do nothing.
6. Keep messages selectable, errors visible, icons labeled, keyboard focus clear, and Escape ordered by overlay/page/window.
7. Verify formatting, check, Clippy, tests, and an actual packaged Release launch. Record actual light/dark, DPI, IME, and native checks; mark anything unverified pending.
