use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::{Button, ButtonVariants},
    radio::Radio,
    switch::Switch,
};
use gpui_kit::{
    AnyWindowHandle, Context, IntoElement, ParentElement, Render, SharedString, Styled, Task,
    Window, div,
};
use lexwisp_core::{AppSettings, GlobalHotkey, SettingsUiPort, ThemePreference};

use crate::surface::apply_theme;

pub struct ControlCenter {
    settings: Arc<dyn SettingsUiPort>,
    draft: AppSettings,
    status: SharedString,
    saving: bool,
    save_task: Option<Task<()>>,
}

impl ControlCenter {
    pub fn new(settings: Arc<dyn SettingsUiPort>) -> Self {
        let draft = settings.snapshot().settings().clone();
        Self {
            settings,
            draft,
            status: "Changes are applied and saved only after you choose Save.".into(),
            saving: false,
            save_task: None,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.status = "Saving settings…".into();
        let settings = self.settings.clone();
        let draft = self.draft.clone();
        let window_handle: AnyWindowHandle = window.window_handle();
        self.save_task = Some(cx.spawn(async move |view, cx| {
            let result = settings.apply(draft).await;
            let theme = result
                .as_ref()
                .ok()
                .map(|snapshot| snapshot.settings().theme());
            let _ = view.update(cx, |view, cx| {
                view.saving = false;
                match result {
                    Ok(snapshot) => {
                        view.draft = snapshot.settings().clone();
                        view.status =
                            format!("Saved · generation {}", snapshot.generation()).into();
                    }
                    Err(error) => {
                        view.status = format!("Could not save: {error}").into();
                    }
                }
                cx.notify();
            });
            if let Some(theme) = theme {
                let _ = window_handle.update(cx, |_, window, cx| {
                    apply_theme(theme, window, cx);
                });
            }
        }));
    }
}

impl Render for ControlCenter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hotkeys = GlobalHotkey::ALL.into_iter().map(|hotkey| {
            Radio::new(format!("hotkey-{hotkey:?}"))
                .label(hotkey.label())
                .checked(self.draft.hotkey() == hotkey)
                .on_change(cx.listener(move |this, checked, _, cx| {
                    if *checked {
                        this.draft = this.draft.clone().with_hotkey(hotkey);
                        cx.notify();
                    }
                }))
        });
        let themes = ThemePreference::ALL.into_iter().map(|theme| {
            Radio::new(format!("theme-{theme:?}"))
                .label(theme.label())
                .checked(self.draft.theme() == theme)
                .on_change(cx.listener(move |this, checked, _, cx| {
                    if *checked {
                        this.draft = this.draft.clone().with_theme(theme);
                        cx.notify();
                    }
                }))
        });
        let retention = [0_u64, 30, 60].into_iter().map(|seconds| {
            let label = if seconds == 0 {
                "Destroy immediately".to_string()
            } else {
                format!("Keep warm for {seconds} seconds")
            };
            Radio::new(format!("retention-{seconds}"))
                .label(label)
                .checked(self.draft.popup_retention_seconds() == seconds)
                .on_change(cx.listener(move |this, checked, _, cx| {
                    if *checked {
                        this.draft = this.draft.clone().with_popup_retention_seconds(seconds);
                        cx.notify();
                    }
                }))
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .p_6()
            .gap_5()
            .child(div().text_2xl().child("Control Center"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("System shell settings · stored locally as versioned TOML"),
            )
            .child(setting_group("Global shortcut", hotkeys))
            .child(setting_group("Theme", themes))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child("Launch at Windows sign-in")
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Registers LexWisp for the current user."),
                            ),
                    )
                    .child(
                        Switch::new("launch-at-startup")
                            .accessibility_label("Launch at Windows sign-in")
                            .checked(self.draft.launch_at_startup())
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.draft = this.draft.clone().with_launch_at_startup(*checked);
                                cx.notify();
                            })),
                    ),
            )
            .child(setting_group("Quick Shell retention", retention))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("save-settings")
                            .primary()
                            .label(if self.saving { "Saving…" } else { "Save" })
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save(window, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.status.clone()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Settings schema {}", self.draft.schema_version())),
            )
    }
}

fn setting_group(
    title: &'static str,
    controls: impl IntoIterator<Item = impl IntoElement>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().child(title))
        .children(controls)
}
