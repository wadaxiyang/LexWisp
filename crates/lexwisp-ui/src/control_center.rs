use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    radio::Radio,
    scroll::ScrollableElement,
    switch::Switch,
};
use gpui_kit::{
    AnyWindowHandle, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, Window, div,
};
use lexwisp_core::{
    AppSettings, GlobalHotkey, ProviderDraft, ProviderUiPort, SettingsUiPort, ThemePreference,
};

use crate::surface::apply_theme;

pub struct ControlCenter {
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    draft: AppSettings,
    provider_name: Entity<InputState>,
    provider_url: Entity<InputState>,
    provider_model: Entity<InputState>,
    provider_key: Entity<InputState>,
    provider_stream: bool,
    provider_authentication: bool,
    status: SharedString,
    provider_status: SharedString,
    saving: bool,
    provider_busy: bool,
    save_task: Option<Task<()>>,
    provider_task: Option<Task<()>>,
}

impl ControlCenter {
    pub fn new(
        settings: Arc<dyn SettingsUiPort>,
        providers: Arc<dyn ProviderUiPort>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = settings.snapshot().settings().clone();
        let provider = providers.snapshot();
        let provider_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("OpenAI compatible")
                .default_value(provider.display_name)
        });
        let provider_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://api.openai.com/v1")
                .default_value(provider.base_url)
        });
        let provider_model = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Model ID")
                .default_value(provider.model_id)
        });
        let provider_key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(if provider.has_saved_credential {
                    "Saved in Windows Credential Manager"
                } else {
                    "API key"
                })
                .masked(true)
        });
        Self {
            settings,
            providers,
            draft,
            provider_name,
            provider_url,
            provider_model,
            provider_key,
            provider_stream: provider.stream,
            provider_authentication: provider.use_authentication,
            status: "Changes are applied and saved only after you choose Save.".into(),
            provider_status: if provider.configured {
                "Provider configured · the saved key is never shown here.".into()
            } else {
                "Configure and test an OpenAI-compatible endpoint.".into()
            },
            saving: false,
            provider_busy: false,
            save_task: None,
            provider_task: None,
        }
    }

    fn provider_draft(&self, cx: &Context<Self>) -> ProviderDraft {
        ProviderDraft {
            id: "default".into(),
            display_name: self.provider_name.read(cx).value().to_string(),
            base_url: self.provider_url.read(cx).value().to_string(),
            model_id: self.provider_model.read(cx).value().to_string(),
            stream: self.provider_stream,
            use_authentication: self.provider_authentication,
            api_key: Some(self.provider_key.read(cx).value().to_string()),
        }
    }

    fn test_provider(&mut self, cx: &mut Context<Self>) {
        if self.provider_busy {
            return;
        }
        self.provider_busy = true;
        self.provider_status = "Testing endpoint…".into();
        let providers = self.providers.clone();
        let draft = self.provider_draft(cx);
        self.provider_task = Some(cx.spawn(async move |view, cx| {
            let result = providers.test(draft).await;
            let _ = view.update(cx, |view, cx| {
                view.provider_busy = false;
                view.provider_status = match result {
                    Ok(result) => format!(
                        "Connected to {} · response: {}",
                        result.model_id, result.response_preview
                    )
                    .into(),
                    Err(error) => format!("Test failed: {error}").into(),
                };
                cx.notify();
            });
        }));
    }

    fn save_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.provider_busy {
            return;
        }
        self.provider_busy = true;
        self.provider_status = "Saving provider…".into();
        let providers = self.providers.clone();
        let settings = self.settings.clone();
        let draft = self.provider_draft(cx);
        let key = self.provider_key.clone();
        let window_handle: AnyWindowHandle = window.window_handle();
        self.provider_task = Some(cx.spawn(async move |view, cx| {
            let result = providers.save(draft).await;
            let saved = result.is_ok();
            let _ = view.update(cx, |view, cx| {
                view.provider_busy = false;
                view.provider_status = match result {
                    Ok(snapshot) => {
                        view.draft = settings.snapshot().settings().clone();
                        format!(
                            "Saved {} / {} · generation {}",
                            snapshot.display_name, snapshot.model_id, snapshot.generation
                        )
                        .into()
                    }
                    Err(error) => format!("Could not save provider: {error}").into(),
                };
                cx.notify();
            });
            if saved {
                let _ = window_handle.update(cx, |_, window, cx| {
                    key.update(cx, |key, cx| key.set_value("", window, cx));
                });
            }
        }));
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
                    .child("Provider keys stay in Windows Credential Manager; settings contain references only."),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(div().text_lg().child("Default AI provider"))
                    .child(
                        div()
                            .grid()
                            .grid_cols(2)
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_sm().child("Name"))
                                    .child(Input::new(&self.provider_name).aria_label("Provider name")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(div().text_sm().child("Model ID"))
                                    .child(Input::new(&self.provider_model).aria_label("Model ID")),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child("Base URL"))
                            .child(Input::new(&self.provider_url).aria_label("Provider Base URL")),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child("API key"))
                            .child(
                                Input::new(&self.provider_key)
                                    .aria_label("Provider API key")
                                    .mask_toggle()
                                    .disabled(!self.provider_authentication),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child("Use Bearer authentication")
                            .child(
                                Switch::new("provider-authentication")
                                    .accessibility_label("Use Bearer authentication")
                                    .checked(self.provider_authentication)
                                    .on_change(cx.listener(|this, checked, _, cx| {
                                        this.provider_authentication = *checked;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child("Stream responses")
                            .child(
                                Switch::new("provider-stream")
                                    .accessibility_label("Stream responses")
                                    .checked(self.provider_stream)
                                    .on_change(cx.listener(|this, checked, _, cx| {
                                        this.provider_stream = *checked;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Button::new("test-provider")
                                    .label(if self.provider_busy { "Working…" } else { "Test" })
                                    .disabled(self.provider_busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.test_provider(cx);
                                    })),
                            )
                            .child(
                                Button::new("save-provider")
                                    .primary()
                                    .label("Save provider")
                                    .disabled(self.provider_busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_provider(window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.provider_status.clone()),
                            ),
                    ),
            )
            .child(div().text_lg().child("System"))
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
            .overflow_y_scrollbar()
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
