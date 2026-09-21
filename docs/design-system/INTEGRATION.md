# Current integration

The one-popup product UI is implemented in `crates/lexwisp-ui`:

| File | Responsibility |
| --- | --- |
| `theme.rs` | LexWisp projection onto Kit light/dark semantic roles |
| `ui_metrics.rs` | 680 × 640 DIP popup, 42 DIP header, 4 px rhythm |
| `surface.rs` | One native window, Kit Root/overlays, hide/show/warm retention, pin |
| `chat.rs` | Lab-inspired header, switcher, transcript, composer, History page |
| `settings_view.rs` | General, Provider, and data settings within the popup |

The lab is a standalone reference and remains outside the product dependency graph. Product controls call `ChatUiPort`, `SettingsUiPort`, `ProviderUiPort`, and `HistoryUiPort`; only the app composes them with Host implementations. The lab's simulated responses and any controls without current product behavior are not carried into LexWisp.

Use Kit components and semantic theme roles. Keep input entities and subscriptions in their owner, and keep render free of I/O. `ChatController` owns conversation state across popup hide/reopen; `SurfaceController` owns the native window lifecycle.
