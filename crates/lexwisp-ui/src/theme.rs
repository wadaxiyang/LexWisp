//! Semantic projection for the Fluent shell and neutral chat canvas.
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Rgba, Window, px};
use lexwisp_core::ThemePreference;

pub fn apply_theme(preference: ThemePreference, window: &mut Window, cx: &mut App) {
    let mode = match preference {
        ThemePreference::System => window.appearance().into(),
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
    };
    Theme::change(mode, Some(window), cx);
    apply_palette(cx);
}

pub fn sync_system_theme(window: &mut Window, cx: &mut App) {
    apply_theme(ThemePreference::System, window, cx);
}

struct Palette {
    shell: Hsla,
    canvas: Hsla,
    surface: Hsla,
    overlay: Hsla,
    text: Hsla,
    muted: Hsla,
    border: Hsla,
    hover: Hsla,
    selected: Hsla,
    accent: Hsla,
    accent_hover: Hsla,
    accent_active: Hsla,
    danger: Hsla,
    danger_active: Hsla,
    white: Hsla,
}

impl Palette {
    fn dark() -> Self {
        Self {
            shell: ca(0x1C1C1C, 0.86),
            canvas: c(0x191919),
            surface: c(0x252525),
            overlay: c(0x2B2B2B),
            text: c(0xECECEC),
            muted: c(0xA4A4A4),
            border: c(0x3A3A3A),
            hover: c(0x343434),
            selected: c(0x343C48),
            accent: c(0x78A9EF),
            accent_hover: c(0x8AB7F3),
            accent_active: c(0x6595D8),
            danger: c(0xC94B52),
            danger_active: c(0xB43E45),
            white: c(0xFFFFFF),
        }
    }
    fn light() -> Self {
        Self {
            shell: ca(0xF3F3F3, 0.86),
            canvas: c(0xFAFAFA),
            surface: c(0xFFFFFF),
            overlay: c(0xFFFFFF),
            text: c(0x202020),
            muted: c(0x646464),
            border: c(0xDEDEDE),
            hover: c(0xEAEAEA),
            selected: c(0xE5EDF8),
            accent: c(0x3268B8),
            accent_hover: c(0x4078C9),
            accent_active: c(0x28579E),
            danger: c(0xC83C42),
            danger_active: c(0xAD3037),
            white: c(0xFFFFFF),
        }
    }
}

fn apply_palette(cx: &mut App) {
    let p = if Theme::global(cx).is_dark() {
        Palette::dark()
    } else {
        Palette::light()
    };
    {
        let t = Theme::global_mut(cx);
        t.font_family = ".SystemUIFont".into();
        t.font_size = px(14.0);
        t.mono_font_size = px(13.0);
        t.radius = px(6.0);
        t.radius_lg = px(10.0);
        t.tile_radius = px(8.0);
        t.tile_shadow = false;
        t.shadow = true;
        t.focus_ring = true;
        t.background = p.canvas;
        t.foreground = p.text;
        t.muted_foreground = p.muted;
        t.border = p.border;
        t.input = p.border;
        t.ring = p.accent;
        t.title_bar = p.shell;
        t.title_bar_border = p.border;
        t.sidebar = p.shell;
        t.sidebar_foreground = p.text;
        t.sidebar_border = p.border;
        t.group_box = p.surface;
        t.group_box_foreground = p.text;
        t.popover = p.overlay;
        t.popover_foreground = p.text;
        t.muted = p.surface;
        t.accent = p.hover;
        t.accent_foreground = p.text;
        t.primary = p.accent;
        t.primary_hover = p.accent_hover;
        t.primary_active = p.accent_active;
        t.primary_foreground = p.white;
        t.button_primary = p.accent;
        t.button_primary_hover = p.accent_hover;
        t.button_primary_active = p.accent_active;
        t.button_primary_foreground = p.white;
        t.button = p.surface;
        t.button_foreground = p.text;
        t.button_hover = p.hover;
        t.button_active = p.selected;
        t.button_secondary = p.surface;
        t.button_secondary_foreground = p.text;
        t.button_secondary_hover = p.hover;
        t.button_secondary_active = p.selected;
        t.secondary = p.surface;
        t.secondary_foreground = p.text;
        t.secondary_hover = p.hover;
        t.secondary_active = p.selected;
        t.colors.list = p.canvas;
        t.list_even = p.canvas;
        t.list_head = p.canvas;
        t.list_hover = p.hover;
        t.list_active = p.selected;
        t.list_active_border = p.border;
        t.selection = p.selected;
        t.caret = p.accent;
        t.link = p.accent;
        t.link_hover = p.accent_hover;
        t.link_active = p.accent_active;
        t.drag_border = p.accent;
        t.drop_target = p.selected;
        t.danger = p.danger;
        t.danger_hover = p.danger;
        t.danger_active = p.danger_active;
        t.danger_foreground = p.white;
        t.button_danger = p.danger;
        t.button_danger_hover = p.danger;
        t.button_danger_active = p.danger_active;
        t.button_danger_foreground = p.white;
        t.window_border = p.border;
        t.switch = p.border;
        t.switch_thumb = p.white;
        t.scrollbar_thumb = p.border;
        t.scrollbar_thumb_hover = p.muted;
        t.tab = p.canvas;
        t.tab_bar = p.canvas;
        t.tab_foreground = p.muted;
        t.tab_active = p.selected;
        t.tab_active_foreground = p.text;
        t.transparent = ca(0xFFFFFF, 0.0);
        t.scrollbar = t.transparent;
        // Kit 0.6.1 retains paint tokens beside ThemeColor.
        t.tokens = t.colors.into();
    }
    Theme::sync_base(cx);
    cx.refresh_windows();
}

fn c(value: u32) -> Hsla {
    ca(value, 1.0)
}
fn ca(value: u32, alpha: f32) -> Hsla {
    Hsla::from(Rgba {
        r: ((value >> 16) & 0xff) as f32 / 255.0,
        g: ((value >> 8) & 0xff) as f32 / 255.0,
        b: (value & 0xff) as f32 / 255.0,
        a: alpha,
    })
}
