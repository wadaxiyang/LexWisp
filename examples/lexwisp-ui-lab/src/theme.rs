//! LexWisp application theme for GPUI-Kit 0.6.1.
//!
//! Raw product colors live here. Product views should consume semantic
//! `cx.theme()` roles instead of inventing local RGB/HSL values.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Rgba, Window, px};

/// Theme preference used by this standalone UI lab.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

/// Apply the requested appearance and then project LexWisp's semantic palette
/// over GPUI-Kit's matching light/dark base theme.
///
/// Call this whenever a window is created, shown after a theme change, or the
/// system appearance changes while the preference is `System`.
pub fn apply_theme(preference: ThemePreference, window: &mut Window, cx: &mut App) {
    let mode = match preference {
        ThemePreference::System => window.appearance().into(),
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
    };

    // Load the correct GPUI-Kit mode first. This resets the component palette,
    // so LexWisp overrides must always be applied afterwards.
    Theme::change(mode, Some(window), cx);
    apply_lexwisp_palette(cx);
}

/// Re-apply only when LexWisp follows the system appearance.
pub fn sync_system_theme(window: &mut Window, cx: &mut App) {
    apply_theme(ThemePreference::System, window, cx);
}

fn apply_lexwisp_palette(cx: &mut App) {
    let dark = Theme::global(cx).is_dark();

    {
        let theme = Theme::global_mut(cx);

        if dark {
            apply_dark(theme);
        } else {
            apply_light(theme);
        }

        // Windows-first, cross-platform-safe GPUI system UI alias.
        theme.font_family = ".SystemUIFont".into();
        theme.font_size = px(14.0);
        theme.mono_font_size = px(13.0);

        // Restrained workbench geometry. Large floating surfaces may refine their
        // own radius, but ordinary controls should stay restrained.
        theme.radius = px(6.0);
        theme.radius_lg = px(10.0);
        theme.tile_radius = px(8.0);
        theme.tile_shadow = false;

        // Keep GPUI-Kit's shadows for overlays/dialogs. Persistent LexWisp
        // panels must not add their own shadows.
        theme.shadow = true;
        theme.focus_ring = true;

        // GPUI-Kit 0.6.1 keeps legacy component paint tokens alongside
        // ThemeColor. Direct ThemeColor mutation does not reconcile them, so
        // replace the resolved tokens from the final palette before syncing
        // the Base projection.
        let colors = theme.colors;
        theme.tokens = colors.into();
    }

    Theme::sync_base(cx);
    cx.refresh_windows();
}

fn apply_dark(theme: &mut Theme) {
    let canvas = c(0x0F1115);
    let foreground = c(0xE7EAF0);
    let sidebar = c(0x11141A);
    let sidebar_foreground = c(0xCDD4DE);
    let raised = c(0x1A1F27);
    let popover = c(0x1D232C);
    let border = c(0x2A303A);
    let input = c(0x343C49);
    let muted_foreground = c(0x939EAC);

    let hover = c(0x181D24);
    let active = c(0x232A34);
    let selected = c(0x202B3E);
    let selected_border = c(0x2B3A54);

    let primary = c(0x6689DF);
    let primary_hover = c(0x7698EA);
    let primary_active = c(0x5678C9);
    let primary_foreground = c(0xFFFFFF);

    let danger = c(0xE26D78);
    let danger_hover = c(0xEE7A85);
    let danger_active = c(0xC95D67);
    let success = c(0x4FC28B);
    let warning = c(0xD9A441);
    let info = c(0x65A8E8);

    theme.background = canvas;
    theme.foreground = foreground;
    theme.border = border;
    theme.input = input;
    theme.ring = primary;
    theme.transparent = ca(0x000000, 0.0);

    // The shell chrome overlays the platform backdrop. Keep foreground and
    // interaction states opaque so navigation remains readable.
    theme.sidebar = ca(0x11141A, 0.82);
    theme.sidebar_foreground = sidebar_foreground;
    theme.sidebar_border = c(0x202630);
    theme.sidebar_accent = c(0x1C222B);
    theme.sidebar_accent_foreground = foreground;
    theme.sidebar_primary = primary;
    theme.sidebar_primary_foreground = primary_foreground;

    // `Theme` itself also has `list: ListSettings`, therefore the color role
    // must be addressed through `colors`.
    theme.colors.list = canvas;
    theme.list_even = canvas;
    theme.list_head = canvas;
    theme.list_hover = hover;
    theme.list_active = selected;
    theme.list_active_border = selected_border;

    theme.table = canvas;
    theme.table_even = canvas;
    theme.table_head = sidebar;
    theme.table_head_foreground = sidebar_foreground;
    theme.table_foot = sidebar;
    theme.table_foot_foreground = sidebar_foreground;
    theme.table_hover = hover;
    theme.table_active = selected;
    theme.table_active_border = selected_border;
    theme.table_row_border = border;

    theme.muted = raised;
    theme.muted_foreground = muted_foreground;
    theme.accent = c(0x1E242D);
    theme.accent_foreground = foreground;

    theme.primary = primary;
    theme.primary_hover = primary_hover;
    theme.primary_active = primary_active;
    theme.primary_foreground = primary_foreground;

    theme.secondary = c(0x1B2028);
    theme.secondary_hover = active;
    theme.secondary_active = c(0x2B333F);
    theme.secondary_foreground = c(0xE3E7ED);

    // Persistent contained work such as the composer may use group_box.
    theme.group_box = raised;
    theme.group_box_foreground = foreground;
    theme.popover = popover;
    theme.popover_foreground = foreground;
    theme.accordion = raised;

    theme.selection = ca(0x6689DF, 0.30);
    theme.caret = primary;
    theme.drag_border = primary;
    theme.drop_target = ca(0x6689DF, 0.16);

    theme.button = c(0x1B2028);
    theme.button_foreground = c(0xE3E7ED);
    theme.button_hover = active;
    theme.button_active = c(0x2B333F);

    theme.button_primary = primary;
    theme.button_primary_foreground = primary_foreground;
    theme.button_primary_hover = primary_hover;
    theme.button_primary_active = primary_active;

    theme.button_secondary = c(0x1B2028);
    theme.button_secondary_foreground = c(0xE3E7ED);
    theme.button_secondary_hover = active;
    theme.button_secondary_active = c(0x2B333F);

    theme.danger = danger;
    theme.danger_hover = danger_hover;
    theme.danger_active = danger_active;
    theme.danger_foreground = c(0xFFFFFF);
    theme.button_danger = danger;
    theme.button_danger_hover = danger_hover;
    theme.button_danger_active = danger_active;
    theme.button_danger_foreground = c(0xFFFFFF);

    theme.success = success;
    theme.success_hover = c(0x60CE99);
    theme.success_active = c(0x3EAA77);
    theme.success_foreground = c(0x0D241A);
    theme.button_success = success;
    theme.button_success_hover = c(0x60CE99);
    theme.button_success_active = c(0x3EAA77);
    theme.button_success_foreground = c(0x0D241A);

    theme.warning = warning;
    theme.warning_hover = c(0xE4B252);
    theme.warning_active = c(0xBE8E31);
    theme.warning_foreground = c(0x241C08);
    theme.button_warning = warning;
    theme.button_warning_hover = c(0xE4B252);
    theme.button_warning_active = c(0xBE8E31);
    theme.button_warning_foreground = c(0x241C08);

    theme.info = info;
    theme.info_hover = c(0x75B5F1);
    theme.info_active = c(0x5296D5);
    theme.info_foreground = c(0x071923);
    theme.button_info = info;
    theme.button_info_hover = c(0x75B5F1);
    theme.button_info_active = c(0x5296D5);
    theme.button_info_foreground = c(0x071923);

    theme.link = c(0x86A8F3);
    theme.link_hover = c(0x9AB8F7);
    theme.link_active = primary;

    theme.title_bar = theme.sidebar;
    theme.title_bar_border = c(0x202630);
    theme.status_bar = sidebar;
    theme.status_bar_border = c(0x202630);

    theme.scrollbar = ca(0x000000, 0.0);
    theme.scrollbar_thumb = c(0x3A424F);
    theme.scrollbar_thumb_hover = c(0x4B5564);

    theme.skeleton = c(0x202630);
    theme.switch = c(0x303744);
    theme.switch_thumb = c(0xA8B0BC);
    theme.slider_bar = c(0x303744);
    theme.slider_thumb = primary;
    theme.progress_bar = primary;

    theme.tab = sidebar;
    theme.tab_bar = sidebar;
    theme.tab_bar_segmented = c(0x1B2028);
    theme.tab_foreground = muted_foreground;
    theme.tab_active = active;
    theme.tab_active_foreground = foreground;

    theme.tiles = raised;
    theme.description_list_label = c(0x171B22);
    theme.description_list_label_foreground = muted_foreground;

    theme.overlay = ca(0x000000, 0.60);
    theme.window_border = border;

    // Base hue family used by component helpers.
    theme.red = danger;
    theme.red_light = ca(0xE26D78, 0.18);
    theme.green = success;
    theme.green_light = ca(0x4FC28B, 0.18);
    theme.blue = primary;
    theme.blue_light = ca(0x6689DF, 0.18);
    theme.yellow = warning;
    theme.yellow_light = ca(0xD9A441, 0.18);
    theme.magenta = c(0xB383D7);
    theme.magenta_light = ca(0xB383D7, 0.18);
    theme.cyan = c(0x54B6C9);
    theme.cyan_light = ca(0x54B6C9, 0.18);
}

fn apply_light(theme: &mut Theme) {
    let canvas = c(0xF7F8FA);
    let foreground = c(0x171A1F);
    let sidebar = c(0xF1F3F6);
    let sidebar_foreground = c(0x3F4651);
    let raised = c(0xFFFFFF);
    let border = c(0xE1E5EA);
    let input = c(0xD6DCE4);
    let muted_foreground = c(0x68717D);

    let hover = c(0xECEFF3);
    let active = c(0xE4E8ED);
    let selected = c(0xE6ECF8);
    let selected_border = c(0xCCD8EE);

    let primary = c(0x3567CE);
    let primary_hover = c(0x4577D9);
    let primary_active = c(0x2D5AB5);
    let primary_foreground = c(0xFFFFFF);

    let danger = c(0xD84955);
    let danger_hover = c(0xE25762);
    let danger_active = c(0xC33D48);
    let success = c(0x2F9B68);
    let warning = c(0xA36C13);
    let info = c(0x3278C8);

    theme.background = canvas;
    theme.foreground = foreground;
    theme.border = border;
    theme.input = input;
    theme.ring = primary;
    theme.transparent = ca(0xFFFFFF, 0.0);

    // Share one translucent material across the title bar and navigation.
    theme.sidebar = ca(0xF1F3F6, 0.78);
    theme.sidebar_foreground = sidebar_foreground;
    theme.sidebar_border = c(0xDDE2E8);
    theme.sidebar_accent = c(0xE7EAF0);
    theme.sidebar_accent_foreground = foreground;
    theme.sidebar_primary = primary;
    theme.sidebar_primary_foreground = primary_foreground;

    theme.colors.list = raised;
    theme.list_even = raised;
    theme.list_head = raised;
    theme.list_hover = hover;
    theme.list_active = selected;
    theme.list_active_border = selected_border;

    theme.table = raised;
    theme.table_even = raised;
    theme.table_head = sidebar;
    theme.table_head_foreground = sidebar_foreground;
    theme.table_foot = sidebar;
    theme.table_foot_foreground = sidebar_foreground;
    theme.table_hover = hover;
    theme.table_active = selected;
    theme.table_active_border = selected_border;
    theme.table_row_border = border;

    theme.muted = c(0xEDF0F3);
    theme.muted_foreground = muted_foreground;
    theme.accent = c(0xE9EDF2);
    theme.accent_foreground = foreground;

    theme.primary = primary;
    theme.primary_hover = primary_hover;
    theme.primary_active = primary_active;
    theme.primary_foreground = primary_foreground;

    theme.secondary = c(0xEDF0F4);
    theme.secondary_hover = active;
    theme.secondary_active = c(0xDCE1E7);
    theme.secondary_foreground = c(0x2A3038);

    theme.group_box = raised;
    theme.group_box_foreground = foreground;
    theme.popover = raised;
    theme.popover_foreground = foreground;
    theme.accordion = raised;

    theme.selection = ca(0x3567CE, 0.20);
    theme.caret = primary;
    theme.drag_border = primary;
    theme.drop_target = ca(0x3567CE, 0.10);

    theme.button = c(0xEDF0F4);
    theme.button_foreground = c(0x2A3038);
    theme.button_hover = active;
    theme.button_active = c(0xDCE1E7);

    theme.button_primary = primary;
    theme.button_primary_foreground = primary_foreground;
    theme.button_primary_hover = primary_hover;
    theme.button_primary_active = primary_active;

    theme.button_secondary = c(0xEDF0F4);
    theme.button_secondary_foreground = c(0x2A3038);
    theme.button_secondary_hover = active;
    theme.button_secondary_active = c(0xDCE1E7);

    theme.danger = danger;
    theme.danger_hover = danger_hover;
    theme.danger_active = danger_active;
    theme.danger_foreground = c(0xFFFFFF);
    theme.button_danger = danger;
    theme.button_danger_hover = danger_hover;
    theme.button_danger_active = danger_active;
    theme.button_danger_foreground = c(0xFFFFFF);

    theme.success = success;
    theme.success_hover = c(0x3AA976);
    theme.success_active = c(0x27865A);
    theme.success_foreground = c(0xFFFFFF);
    theme.button_success = success;
    theme.button_success_hover = c(0x3AA976);
    theme.button_success_active = c(0x27865A);
    theme.button_success_foreground = c(0xFFFFFF);

    theme.warning = warning;
    theme.warning_hover = c(0xB3791B);
    theme.warning_active = c(0x8E5D0E);
    theme.warning_foreground = c(0xFFFFFF);
    theme.button_warning = warning;
    theme.button_warning_hover = c(0xB3791B);
    theme.button_warning_active = c(0x8E5D0E);
    theme.button_warning_foreground = c(0xFFFFFF);

    theme.info = info;
    theme.info_hover = c(0x4087D6);
    theme.info_active = c(0x2868B4);
    theme.info_foreground = c(0xFFFFFF);
    theme.button_info = info;
    theme.button_info_hover = c(0x4087D6);
    theme.button_info_active = c(0x2868B4);
    theme.button_info_foreground = c(0xFFFFFF);

    theme.link = c(0x2F65D5);
    theme.link_hover = c(0x3F75E0);
    theme.link_active = c(0x2858BC);

    theme.title_bar = theme.sidebar;
    theme.title_bar_border = c(0xDDE2E8);
    theme.status_bar = sidebar;
    theme.status_bar_border = c(0xDDE2E8);

    theme.scrollbar = ca(0xFFFFFF, 0.0);
    theme.scrollbar_thumb = c(0xC6CCD5);
    theme.scrollbar_thumb_hover = c(0xAEB6C1);

    theme.skeleton = c(0xE4E8ED);
    theme.switch = c(0xD8DDE4);
    theme.switch_thumb = c(0xFFFFFF);
    theme.slider_bar = c(0xD8DDE4);
    theme.slider_thumb = primary;
    theme.progress_bar = primary;

    theme.tab = sidebar;
    theme.tab_bar = sidebar;
    theme.tab_bar_segmented = c(0xEDF0F4);
    theme.tab_foreground = muted_foreground;
    theme.tab_active = raised;
    theme.tab_active_foreground = foreground;

    theme.tiles = raised;
    theme.description_list_label = c(0xF1F3F6);
    theme.description_list_label_foreground = muted_foreground;

    theme.overlay = ca(0x000000, 0.28);
    theme.window_border = border;

    theme.red = danger;
    theme.red_light = ca(0xD84955, 0.10);
    theme.green = success;
    theme.green_light = ca(0x2F9B68, 0.10);
    theme.blue = primary;
    theme.blue_light = ca(0x3567CE, 0.10);
    theme.yellow = warning;
    theme.yellow_light = ca(0xA36C13, 0.10);
    theme.magenta = c(0x8858B4);
    theme.magenta_light = ca(0x8858B4, 0.10);
    theme.cyan = c(0x248DA3);
    theme.cyan_light = ca(0x248DA3, 0.10);
}

fn c(hex: u32) -> Hsla {
    Rgba {
        r: ((hex >> 16) & 0xff) as f32 / 255.0,
        g: ((hex >> 8) & 0xff) as f32 / 255.0,
        b: (hex & 0xff) as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

fn ca(hex: u32, alpha: f32) -> Hsla {
    let mut color = c(hex);
    color.a = alpha;
    color
}
