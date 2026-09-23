use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Task,
    Window, div, px,
};
use lexwisp_core::{
    AppSettings, GlobalHotkey, HistoryUiPort, ProviderDraft, ProviderUiPort, SettingsUiPort,
    ThemePreference,
};

use crate::{theme::apply_theme, ui_metrics};

pub struct SettingsView {
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    history: Arc<dyn HistoryUiPort>,
    draft: AppSettings,
    provider_id: String,
    name: Entity<InputState>,
    url: Entity<InputState>,
    model: Entity<InputState>,
    key: Entity<InputState>,
    budget: Entity<InputState>,
    proxy: Entity<InputState>,
    connect_timeout: Entity<InputState>,
    total_timeout: Entity<InputState>,
    event_timeout: Entity<InputState>,
    temperature: Entity<InputState>,
    max_output: Entity<InputState>,
    stream: bool,
    authentication: bool,
    busy: bool,
    status: SharedString,
    task: Option<Task<()>>,
}

impl SettingsView {
    pub fn new(
        settings: Arc<dyn SettingsUiPort>,
        providers: Arc<dyn ProviderUiPort>,
        history: Arc<dyn HistoryUiPort>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = settings.snapshot().settings().clone();
        let provider = providers.snapshot();
        Self {
            settings,
            providers,
            history,
            draft,
            provider_id: provider.id.clone(),
            name: Self::new_input(provider.display_name, "Provider name", window, cx),
            url: Self::new_input(provider.base_url, "https://api.openai.com/v1", window, cx),
            model: Self::new_input(provider.model_id, "Model ID", window, cx),
            key: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(if provider.has_saved_credential {
                        "Saved in Windows Credential Manager"
                    } else {
                        "API key"
                    })
                    .masked(true)
            }),
            budget: Self::new_input(provider.context_budget.to_string(), "8192", window, cx),
            proxy: Self::new_input(
                provider.proxy_url,
                "Use system proxy when empty",
                window,
                cx,
            ),
            connect_timeout: Self::new_input(
                provider.connect_timeout_seconds.to_string(),
                "30",
                window,
                cx,
            ),
            total_timeout: Self::new_input(
                provider.total_timeout_seconds.to_string(),
                "120",
                window,
                cx,
            ),
            event_timeout: Self::new_input(
                provider.event_timeout_seconds.to_string(),
                "30",
                window,
                cx,
            ),
            temperature: Self::new_input(
                provider
                    .temperature_milli
                    .map(|v| format!("{:.3}", f64::from(v) / 1000.0))
                    .unwrap_or_default(),
                "Provider default",
                window,
                cx,
            ),
            max_output: Self::new_input(
                provider
                    .max_output_tokens
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                "Provider default",
                window,
                cx,
            ),
            stream: provider.stream,
            authentication: provider.use_authentication,
            busy: false,
            status: "".into(),
            task: None,
        }
    }

    fn new_input(
        value: String,
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value)
        })
    }

    fn provider_draft(&self, cx: &Context<Self>) -> ProviderDraft {
        ProviderDraft {
            id: self.provider_id.clone(),
            display_name: self.name.read(cx).value().to_string(),
            base_url: self.url.read(cx).value().to_string(),
            model_id: self.model.read(cx).value().to_string(),
            api_key: Some(self.key.read(cx).value().to_string()),
            context_budget: self.budget.read(cx).value().to_string(),
            proxy_url: self.proxy.read(cx).value().to_string(),
            connect_timeout_seconds: self.connect_timeout.read(cx).value().to_string(),
            total_timeout_seconds: self.total_timeout.read(cx).value().to_string(),
            event_timeout_seconds: self.event_timeout.read(cx).value().to_string(),
            temperature: self.temperature.read(cx).value().to_string(),
            max_output_tokens: self.max_output.read(cx).value().to_string(),
            stream: self.stream,
            use_authentication: self.authentication,
        }
    }

    fn save_general(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Saving settings…".into();
        let settings = self.settings.clone();
        let draft = settings
            .snapshot()
            .settings()
            .clone()
            .with_hotkey(self.draft.hotkey())
            .with_theme(self.draft.theme())
            .with_launch_at_startup(self.draft.launch_at_startup())
            .with_popup_retention_seconds(self.draft.popup_retention_seconds())
            .with_recording_enabled(self.draft.recording_enabled());
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = settings.apply(draft).await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                view.status = match result {
                    Ok(snapshot) => {
                        view.draft = snapshot.settings().clone();
                        apply_theme(snapshot.settings().theme(), window, cx);
                        "Settings saved".into()
                    }
                    Err(error) => format!("Couldn’t save settings: {error}").into(),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn test_provider(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Testing provider…".into();
        let providers = self.providers.clone();
        let draft = self.provider_draft(cx);
        self.task = Some(cx.spawn(async move |view, cx| {
            let result = providers.test(draft).await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                view.status = match result {
                    Ok(result) => format!(
                        "Connected to {}: {}",
                        result.model_id, result.response_preview
                    )
                    .into(),
                    Err(error) => format!("Provider test failed: {error}").into(),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn save_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Saving provider…".into();
        let providers = self.providers.clone();
        let draft = self.provider_draft(cx);
        let key = self.key.clone();
        self.task = Some(cx.spawn_in(window, async move |view, cx| {
            let result = providers.save(draft).await;
            let _ = view.update_in(cx, |view, window, cx| {
                view.busy = false;
                view.status = match result {
                    Ok(snapshot) => {
                        key.update(cx, |key, cx| key.set_value("", window, cx));
                        format!("Saved {} / {}", snapshot.display_name, snapshot.model_id).into()
                    }
                    Err(error) => format!("Couldn’t save provider: {error}").into(),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn backup(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "Creating backup…".into();
        let history = self.history.clone();
        self.task = Some(cx.spawn(async move |view, cx| {
            let result = history.create_backup().await;
            let _ = view.update(cx, |view, cx| {
                view.busy = false;
                view.status = match result {
                    Ok(path) => format!("Backup created at {}", path.display()).into(),
                    Err(error) => format!("Couldn’t create backup: {error}").into(),
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn field(label: &'static str, input: &Entity<InputState>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_1()
            .child(div().text_sm().child(label))
            .child(Input::new(input).aria_label(label))
    }

    fn setting_row(
        label: &'static str,
        control: impl IntoElement,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .min_h(px(ui_metrics::SETTINGS_ROW_HEIGHT))
            .justify_between()
            .gap_3()
            .px_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().text_sm().child(label))
            .child(control)
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.to_string().to_ascii_lowercase();
        let general_feedback = status.contains("settings");
        let provider_feedback = status.contains("provider")
            || status.starts_with("connected")
            || status.starts_with("saved ");
        let backup_feedback = status.contains("backup");
        v_flex().size_full().min_h_0().overflow_y_scrollbar().gap_5().p_5()
            .bg(cx.theme().background)
            .child(v_flex().gap_2()
                .child(div().font_medium().child("General"))
                .child(v_flex().overflow_hidden().rounded(px(ui_metrics::RADIUS_PANEL))
                    .border_1().border_color(cx.theme().border).bg(cx.theme().group_box)
                    .child(Self::setting_row("Global hotkey",
                        Button::new("hotkey").ghost().small().label(self.draft.hotkey().label())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let current = this.draft.hotkey();
                                let next = GlobalHotkey::ALL[(GlobalHotkey::ALL.iter().position(|item| *item == current).unwrap_or(0) + 1) % GlobalHotkey::ALL.len()];
                                this.draft = this.draft.clone().with_hotkey(next); cx.notify();
                            })), cx))
                    .child(Self::setting_row("Appearance",
                        Button::new("theme").ghost().small().label(self.draft.theme().label())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let current = this.draft.theme();
                                let next = ThemePreference::ALL[(ThemePreference::ALL.iter().position(|item| *item == current).unwrap_or(0) + 1) % ThemePreference::ALL.len()];
                                this.draft = this.draft.clone().with_theme(next); cx.notify();
                            })), cx))
                    .child(Self::setting_row("Keep hidden window",
                        Button::new("popup-retention").ghost().small()
                            .label(format!("{} s", self.draft.popup_retention_seconds()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                const SECONDS: [u64; 4] = [0, 30, 120, 600];
                                let current = this.draft.popup_retention_seconds();
                                let next = SECONDS[(SECONDS.iter().position(|seconds| *seconds == current).unwrap_or(0) + 1) % SECONDS.len()];
                                this.draft = this.draft.clone().with_popup_retention_seconds(next); cx.notify();
                            })), cx))
                    .child(Self::setting_row("Launch at startup",
                        Switch::new("launch-at-startup").accessibility_label("Launch at startup")
                            .checked(self.draft.launch_at_startup()).on_change(cx.listener(|this, value, _, cx| {
                                this.draft = this.draft.clone().with_launch_at_startup(*value); cx.notify();
                            })), cx))
                    .child(Self::setting_row("Save conversations locally",
                        Switch::new("recording").accessibility_label("Save conversations locally")
                            .checked(self.draft.recording_enabled()).on_change(cx.listener(|this, value, _, cx| {
                                this.draft = this.draft.clone().with_recording_enabled(*value); cx.notify();
                            })), cx)))
                .child(h_flex().justify_end().child(Button::new("save-general").outline().small().label("Save settings")
                    .disabled(self.busy).on_click(cx.listener(|this, _, window, cx| this.save_general(window, cx)))))
                .when(general_feedback, |section| section.child(div().text_sm()
                    .text_color(cx.theme().muted_foreground).child(self.status.clone()))))
            .child(v_flex().gap_3().rounded(px(ui_metrics::RADIUS_PANEL))
                .border_1().border_color(cx.theme().border).bg(cx.theme().group_box).p_3()
                .child(div().font_medium().child("Provider and model"))
                .child(Self::field("Name", &self.name))
                .child(Self::field("Base URL", &self.url))
                .child(Self::field("Model ID", &self.model))
                .child(Switch::new("authentication").label("Bearer authentication")
                    .checked(self.authentication).on_change(cx.listener(|this, value, _, cx| {
                        this.authentication = *value; cx.notify();
                    })))
                .child(Self::field("API key", &self.key))
                .child(Switch::new("stream").label("Stream responses")
                    .checked(self.stream).on_change(cx.listener(|this, value, _, cx| {
                        this.stream = *value; cx.notify();
                    })))
                .child(Self::field("Context budget", &self.budget))
                .child(Self::field("Proxy URL", &self.proxy))
                .child(h_flex().gap_2().child(Self::field("Connect timeout (s)", &self.connect_timeout))
                    .child(Self::field("Total timeout (s)", &self.total_timeout)))
                .child(Self::field("Event timeout (s)", &self.event_timeout))
                .child(h_flex().gap_2().child(Self::field("Temperature", &self.temperature))
                    .child(Self::field("Max output tokens", &self.max_output)))
                .child(h_flex().gap_2()
                    .child(Button::new("test-provider").outline().small().label("Test")
                        .disabled(self.busy).on_click(cx.listener(|this, _, _, cx| this.test_provider(cx))))
                    .child(Button::new("save-provider").primary().small().label("Save provider")
                        .disabled(self.busy).on_click(cx.listener(|this, _, window, cx| this.save_provider(window, cx)))))
                .when(provider_feedback, |section| section.child(div().text_sm()
                    .text_color(cx.theme().muted_foreground).child(self.status.clone()))))
            .child(v_flex().gap_2()
                .child(div().font_medium().child("History and data"))
                .child(div().text_sm().text_color(cx.theme().muted_foreground)
                    .child("Delete a conversation from History. Backups include saved conversations and replies."))
                .child(Button::new("backup").outline().small().label("Create backup")
                    .disabled(self.busy).on_click(cx.listener(|this, _, _, cx| this.backup(cx))))
                .when(backup_feedback, |section| section.child(div().text_sm()
                    .text_color(cx.theme().muted_foreground).child(self.status.clone()))))
    }
}
