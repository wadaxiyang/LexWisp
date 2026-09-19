#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui_kit::{Global, QuitMode, Subscription};
use std::{cell::Cell, process::ExitCode, rc::Rc};

mod startup_error;

struct ApplicationLifetime {
    _window_closed: Subscription,
}
impl Global for ApplicationLifetime {}

fn main() -> ExitCode {
    let startup_failed = Rc::new(Cell::new(false));
    let failed = startup_failed.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_quit_mode(QuitMode::Explicit);
            // Stage 0 has no tray or hotkey to recover an invisible process.
            let window_closed = cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            });
            cx.set_global(ApplicationLifetime {
                _window_closed: window_closed,
            });
            if let Err(error) = lexwisp_ui::open_probe_window(cx) {
                failed.set(true);
                startup_error::show(&format!(
                    "LexWisp could not open its native window.\n\n{error:#}"
                ));
                cx.quit();
            }
        });
    if startup_failed.get() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
