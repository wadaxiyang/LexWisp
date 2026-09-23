//! Shared visual metrics for the single LexWisp popup.
//!
//! Keep these values small and intentional. Do not turn this file into a
//! generic CSS replacement. Semantic color belongs in `theme.rs`.

use gpui_kit::{Pixels, Size, px, size};

pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_5: f32 = 20.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_8: f32 = 32.0;

pub const FONT_DISPLAY: f32 = 22.0;
pub const FONT_PAGE_TITLE: f32 = 18.0;
pub const FONT_SECTION: f32 = 15.0;
pub const FONT_BODY: f32 = 14.0;
pub const FONT_COMPACT: f32 = 13.0;
pub const FONT_CAPTION: f32 = 12.0;
pub const FONT_MICRO: f32 = 11.0;
pub const FONT_CODE: f32 = 13.0;

pub const RADIUS_TINY: f32 = 4.0;
pub const RADIUS_CONTROL: f32 = 6.0;
pub const RADIUS_PANEL: f32 = 8.0;
pub const RADIUS_OVERLAY: f32 = 10.0;
pub const RADIUS_COMPOSER: f32 = 12.0;
pub const RADIUS_SHELL: f32 = 16.0;

pub const BORDER: f32 = 1.0;
pub const FOCUS_BORDER: f32 = 2.0;

pub const HEADER_HEIGHT: f32 = 42.0;
pub const CAPTION_WIDTH: f32 = 46.0;
pub const HEADER_ACTION_SIZE: f32 = 30.0;
pub const TRANSCRIPT_INSET: f32 = 20.0;
pub const MESSAGE_TURN_GAP: f32 = 24.0;
pub const COMPOSER_INSET: f32 = 16.0;
pub const SETTINGS_ROW_HEIGHT: f32 = 48.0;
pub const HISTORY_ROW_HEIGHT: f32 = 40.0;
pub const SWITCHER_WIDTH: f32 = 560.0;
pub const MAIN_WINDOW_WIDTH: f32 = 680.0;
pub const MAIN_WINDOW_HEIGHT: f32 = 640.0;

pub fn main_window_size() -> Size<Pixels> {
    size(px(MAIN_WINDOW_WIDTH), px(MAIN_WINDOW_HEIGHT))
}
