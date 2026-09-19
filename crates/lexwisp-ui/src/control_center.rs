use std::{rc::Rc, sync::Arc};

use gpui_kit::component::{
    ActiveTheme, Disableable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    radio::Radio,
    scroll::ScrollableElement,
    switch::Switch,
};
use gpui_kit::{
    AnyElement, AnyWindowHandle, AppContext, ClipboardItem, Context, Entity, IntoElement,
    ParentElement, Render, SharedString, Styled, Task, Window, div, px, relative, size,
};
use lexwisp_core::{
    ActionDescriptor, ActionKind, AppSettings, ClearHistoryMode, DismissPolicy, ExecutionStatus,
    GlobalHotkey, HistoryCursor, HistoryDetail, HistoryPage, HistoryQuery, HistoryUiPort,
    LaunchMode, ParameterKind, ProviderDraft, ProviderUiPort, SettingsUiPort, ThemePreference,
};

use crate::surface::apply_theme;

pub struct ControlCenter {
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    history: Arc<dyn HistoryUiPort>,
    actions: Vec<ActionDescriptor>,
    draft: AppSettings,
    provider_name: Entity<InputState>,
    provider_url: Entity<InputState>,
    provider_model: Entity<InputState>,
    provider_key: Entity<InputState>,
    provider_context_budget: Entity<InputState>,
    provider_proxy: Entity<InputState>,
    provider_connect_timeout: Entity<InputState>,
    provider_total_timeout: Entity<InputState>,
    provider_event_timeout: Entity<InputState>,
    provider_temperature: Entity<InputState>,
    provider_max_output_tokens: Entity<InputState>,
    provider_stream: bool,
    provider_authentication: bool,
    status: SharedString,
    provider_status: SharedString,
    saving: bool,
    provider_busy: bool,
    save_task: Option<Task<()>>,
    provider_task: Option<Task<()>>,
    page: ControlPage,
    history_search: Entity<InputState>,
    favorite_note: Entity<InputState>,
    action_parameter_inputs: Vec<(String, String, Entity<InputState>)>,
    history_page: HistoryPage,
    history_detail: Option<HistoryDetail>,
    history_cursor: Option<HistoryCursor>,
    history_back: Vec<Option<HistoryCursor>>,
    history_status_filter: Option<ExecutionStatus>,
    history_plugin_filter: Option<String>,
    favorites_only: bool,
    history_busy: bool,
    history_status: SharedString,
    history_task: Option<Task<()>>,
    delete_armed: bool,
    clear_armed: Option<ClearHistoryMode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlPage {
    Settings,
    History,
    Privacy,
}

impl ControlCenter {
    pub fn new(
        settings: Arc<dyn SettingsUiPort>,
        providers: Arc<dyn ProviderUiPort>,
        history: Arc<dyn HistoryUiPort>,
        actions: Vec<ActionDescriptor>,
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
        let provider_context_budget = cx.new(|cx| {
            InputState::new(window, cx).default_value(provider.context_budget.to_string())
        });
        let provider_proxy = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Use system proxy when empty")
                .default_value(provider.proxy_url)
        });
        let provider_connect_timeout = cx.new(|cx| {
            InputState::new(window, cx).default_value(provider.connect_timeout_seconds.to_string())
        });
        let provider_total_timeout = cx.new(|cx| {
            InputState::new(window, cx).default_value(provider.total_timeout_seconds.to_string())
        });
        let provider_event_timeout = cx.new(|cx| {
            InputState::new(window, cx).default_value(provider.event_timeout_seconds.to_string())
        });
        let provider_temperature = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Provider default")
                .default_value(
                    provider
                        .temperature_milli
                        .map(|value| format!("{:.3}", f64::from(value) / 1000.0))
                        .unwrap_or_default(),
                )
        });
        let provider_max_output_tokens = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Provider default")
                .default_value(
                    provider
                        .max_output_tokens
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                )
        });
        let history_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search titles and saved text"));
        let favorite_note =
            cx.new(|cx| InputState::new(window, cx).placeholder("Favorite annotation"));
        let mut action_parameter_inputs = Vec::new();
        for descriptor in &actions {
            let ActionKind::Declarative(definition) = descriptor.kind() else {
                continue;
            };
            let action = descriptor.qualified_id().to_string();
            for parameter in &definition.parameters {
                if matches!(parameter.kind, ParameterKind::Enum | ParameterKind::Boolean) {
                    continue;
                }
                let value = draft
                    .action_parameter_defaults(&action)
                    .and_then(|values| values.get(&parameter.key))
                    .cloned()
                    .or_else(|| parameter.default_value.clone())
                    .unwrap_or_default();
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(parameter.label.clone())
                        .default_value(value)
                });
                action_parameter_inputs.push((action.clone(), parameter.key.clone(), input));
            }
        }
        Self {
            settings,
            providers,
            history,
            actions,
            draft,
            provider_name,
            provider_url,
            provider_model,
            provider_key,
            provider_context_budget,
            provider_proxy,
            provider_connect_timeout,
            provider_total_timeout,
            provider_event_timeout,
            provider_temperature,
            provider_max_output_tokens,
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
            page: ControlPage::Settings,
            history_search,
            favorite_note,
            action_parameter_inputs,
            history_page: HistoryPage {
                items: Vec::new(),
                next_cursor: None,
            },
            history_detail: None,
            history_cursor: None,
            history_back: Vec::new(),
            history_status_filter: None,
            history_plugin_filter: None,
            favorites_only: false,
            history_busy: false,
            history_status: "Choose Refresh to load saved history.".into(),
            history_task: None,
            delete_armed: false,
            clear_armed: None,
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
            context_budget: self.provider_context_budget.read(cx).value().to_string(),
            proxy_url: self.provider_proxy.read(cx).value().to_string(),
            connect_timeout_seconds: self.provider_connect_timeout.read(cx).value().to_string(),
            total_timeout_seconds: self.provider_total_timeout.read(cx).value().to_string(),
            event_timeout_seconds: self.provider_event_timeout.read(cx).value().to_string(),
            temperature: self.provider_temperature.read(cx).value().to_string(),
            max_output_tokens: self.provider_max_output_tokens.read(cx).value().to_string(),
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
        let mut draft = self.draft.clone();
        for (action, key, input) in &self.action_parameter_inputs {
            draft = draft.with_action_parameter_default(
                action.clone(),
                key.clone(),
                input.read(cx).value().to_string(),
            );
        }
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

    fn load_history(&mut self, cursor: Option<HistoryCursor>, cx: &mut Context<Self>) {
        if self.history_busy {
            return;
        }
        self.history_busy = true;
        self.history_status = "Loading history…".into();
        let history = self.history.clone();
        let query = HistoryQuery {
            search: self.history_search.read(cx).value().to_string(),
            plugin_id: self.history_plugin_filter.clone(),
            status: self.history_status_filter,
            favorites_only: self.favorites_only,
            cursor: cursor.clone(),
            limit: 50,
        };
        self.history_task = Some(cx.spawn(async move |view, cx| {
            let result = history.page(query).await;
            let _ = view.update(cx, |view, cx| {
                view.history_busy = false;
                match result {
                    Ok(page) => {
                        view.history_cursor = cursor;
                        view.history_status = format!("{} saved runs", page.items.len()).into();
                        view.history_page = page;
                        view.history_detail = None;
                        view.delete_armed = false;
                    }
                    Err(error) => view.history_status = format!("History failed: {error}").into(),
                }
                cx.notify();
            });
        }));
    }

    fn select_history(
        &mut self,
        invocation_id: lexwisp_core::InvocationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let history = self.history.clone();
        let note = self.favorite_note.clone();
        let window_handle: AnyWindowHandle = window.window_handle();
        self.history_status = "Loading details…".into();
        self.history_task = Some(cx.spawn(async move |view, cx| {
            let result = history.detail(invocation_id).await;
            let note_value = result
                .as_ref()
                .ok()
                .map(|detail| detail.item.favorite_note.clone());
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(detail) => {
                        view.history_status = "History detail loaded.".into();
                        view.history_detail = Some(detail);
                        view.delete_armed = false;
                    }
                    Err(error) => view.history_status = format!("Detail failed: {error}").into(),
                }
                cx.notify();
            });
            if let Some(value) = note_value {
                let _ = window_handle.update(cx, |_, window, cx| {
                    note.update(cx, |state, cx| state.set_value(value, window, cx));
                });
            }
        }));
    }

    fn save_favorite(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.history_detail.clone() else {
            return;
        };
        let history = self.history.clone();
        let note = self.favorite_note.read(cx).value().to_string();
        let favorite = !detail.item.favorite || note != detail.item.favorite_note;
        self.history_task = Some(cx.spawn(async move |view, cx| {
            let result = history
                .set_favorite(detail.item.invocation_id, favorite, note)
                .await;
            let _ = view.update(cx, |view, cx| {
                view.history_status = match result {
                    Ok(()) if favorite => "Favorite and annotation saved.".into(),
                    Ok(()) => "Favorite removed.".into(),
                    Err(error) => format!("Favorite failed: {error}").into(),
                };
                view.load_history(view.history_cursor.clone(), cx);
            });
        }));
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.history_detail.clone() else {
            return;
        };
        if !self.delete_armed {
            self.delete_armed = true;
            self.history_status = "Choose Confirm delete to permanently remove this run.".into();
            cx.notify();
            return;
        }
        self.delete_armed = false;
        let history = self.history.clone();
        self.history_task = Some(cx.spawn(async move |view, cx| {
            let result = history.delete(detail.item.invocation_id).await;
            let _ = view.update(cx, |view, cx| {
                view.history_status = match result {
                    Ok(()) => "History item deleted; late writes are blocked.".into(),
                    Err(error) => format!("Delete failed: {error}").into(),
                };
                view.load_history(view.history_cursor.clone(), cx);
            });
        }));
    }

    fn clear_history(&mut self, mode: ClearHistoryMode, cx: &mut Context<Self>) {
        if self.clear_armed != Some(mode) {
            self.clear_armed = Some(mode);
            self.history_status = match mode {
                ClearHistoryMode::PreserveFavorites => {
                    "Choose the same Clear button again; favorite-referenced content is preserved."
                }
                ClearHistoryMode::IncludeFavorites => {
                    "Choose the same Delete all button again; favorites are also removed."
                }
            }
            .into();
            cx.notify();
            return;
        }
        self.clear_armed = None;
        let history = self.history.clone();
        self.history_task = Some(cx.spawn(async move |view, cx| {
            let result = history.clear(mode).await;
            let _ = view.update(cx, |view, cx| {
                view.history_back.clear();
                view.history_cursor = None;
                view.history_status = match result {
                    Ok(()) => "History cleared with a new retention generation.".into(),
                    Err(error) => format!("Clear failed: {error}").into(),
                };
                view.load_history(None, cx);
            });
        }));
    }

    fn retry_selected(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.history_detail.clone() else {
            return;
        };
        let history = self.history.clone();
        self.history_status = "Retrying with the registered action…".into();
        self.history_task = Some(cx.spawn(async move |view, cx| {
            let result = history.retry(detail.item.invocation_id).await;
            let _ = view.update(cx, |view, cx| {
                view.history_status = match result {
                    Ok(output) => {
                        format!("Retry completed · {} characters", output.chars().count()).into()
                    }
                    Err(error) => format!("Retry failed: {error}").into(),
                };
                view.load_history(None, cx);
            });
        }));
    }

    fn create_backup(&mut self, cx: &mut Context<Self>) {
        let history = self.history.clone();
        self.history_status = "Creating a consistent SQLite backup…".into();
        self.history_task = Some(cx.spawn(async move |view, cx| {
            let result = history.create_backup().await;
            let _ = view.update(cx, |view, cx| {
                view.history_status = match result {
                    Ok(path) => format!("Backup created: {}", path.display()).into(),
                    Err(error) => format!("Backup failed: {error}").into(),
                };
                cx.notify();
            });
        }));
    }

    fn reload_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.status = "Reloading settings.toml…".into();
        let settings = self.settings.clone();
        let window_handle: AnyWindowHandle = window.window_handle();
        self.save_task = Some(cx.spawn(async move |view, cx| {
            let result = settings.reload().await;
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
                            format!("Reloaded · generation {}", snapshot.generation()).into();
                    }
                    Err(error) => {
                        view.status =
                            format!("Reload refused; original file preserved: {error}").into()
                    }
                }
                cx.notify();
            });
            if let Some(theme) = theme {
                let _ = window_handle.update(cx, |_, window, cx| apply_theme(theme, window, cx));
            }
        }));
    }
}

impl ControlCenter {
    fn render_history(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let sizes = Rc::new(vec![size(px(520.), px(64.)); self.history_page.items.len()]);
        let detail = self.history_detail.clone();
        let mut plugins = self
            .actions
            .iter()
            .map(|action| action.plugin_id().as_str().to_owned())
            .collect::<Vec<_>>();
        plugins.sort();
        plugins.dedup();
        let list = gpui_kit::base::v_virtual_list(
            cx.entity(),
            "history-virtual-list",
            sizes,
            |this, range, _, cx| {
                range
                    .filter_map(|index| this.history_page.items.get(index).cloned())
                    .map(|item| {
                        let id = item.invocation_id.clone();
                        Button::new(format!("history-row-{}", item.invocation_id))
                            .label(format!(
                                "{}  ·  {:?}{}\n{}",
                                item.title,
                                item.status,
                                if item.favorite { "  ★" } else { "" },
                                item.preview
                            ))
                            .w_full()
                            .h(px(60.))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_history(id.clone(), window, cx);
                            }))
                    })
                    .collect::<Vec<_>>()
            },
        );
        let detail_panel = if let Some(detail) = detail {
            let output = detail.output.clone();
            div()
                .flex()
                .flex_col()
                .gap_3()
                .p_4()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_lg()
                .child(div().text_lg().child(detail.item.title.clone()))
                .child(div().text_sm().child(format!(
                    "{} / {} · {:?}",
                    detail.item.provider_id, detail.item.model_id, detail.item.status
                )))
                .child(div().text_sm().child("Input"))
                .child(div().max_h(px(120.)).overflow_y_scrollbar().child(
                    gpui_kit::base::SelectableText::new("history-input-text", detail.input),
                ))
                .child(div().text_sm().child("Output"))
                .child(div().max_h(px(220.)).overflow_y_scrollbar().child(
                    gpui_kit::base::SelectableText::new("history-output-text", output.clone()),
                ))
                .child(Input::new(&self.favorite_note).aria_label("Favorite annotation"))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(Button::new("copy-history-output").label("Copy").on_click(
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(output.clone()));
                            },
                        ))
                        .child(
                            Button::new("save-history-favorite")
                                .label(if detail.item.favorite {
                                    "Save note / Unfavorite"
                                } else {
                                    "Favorite"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.save_favorite(cx))),
                        )
                        .child(
                            Button::new("retry-history")
                                .label("Retry")
                                .on_click(cx.listener(|this, _, _, cx| this.retry_selected(cx))),
                        )
                        .child(
                            Button::new("delete-history")
                                .danger()
                                .label(if self.delete_armed {
                                    "Confirm delete"
                                } else {
                                    "Delete"
                                })
                                .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx))),
                        ),
                )
        } else {
            div()
                .p_4()
                .text_color(cx.theme().muted_foreground)
                .child("Select a saved run to view its authoritative input and output.")
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Input::new(&self.history_search)
                            .aria_label("Search history")
                            .flex_1(),
                    )
                    .child(
                        Button::new("refresh-history")
                            .primary()
                            .label(if self.history_busy {
                                "Loading…"
                            } else {
                                "Search / Refresh"
                            })
                            .disabled(self.history_busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.history_back.clear();
                                this.load_history(None, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child("Plugin")
                    .child(
                        Radio::new("history-plugin-all")
                            .label("All")
                            .checked(self.history_plugin_filter.is_none())
                            .on_change(cx.listener(|this, checked, _, cx| {
                                if *checked {
                                    this.history_plugin_filter = None;
                                    this.load_history(None, cx);
                                }
                            })),
                    )
                    .children(plugins.into_iter().map(|plugin| {
                        let selected = self.history_plugin_filter.as_deref() == Some(&plugin);
                        let id = plugin.clone();
                        Radio::new(format!("history-plugin-{plugin}"))
                            .label(plugin)
                            .checked(selected)
                            .on_change(cx.listener(move |this, checked, _, cx| {
                                if *checked {
                                    this.history_plugin_filter = Some(id.clone());
                                    this.load_history(None, cx);
                                }
                            }))
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Switch::new("favorites-only")
                            .accessibility_label("Favorites only")
                            .checked(self.favorites_only)
                            .on_change(cx.listener(|this, checked, _, cx| {
                                this.favorites_only = *checked;
                                this.load_history(None, cx);
                            })),
                    )
                    .child("Favorites only")
                    .children(
                        [
                            None,
                            Some(ExecutionStatus::Completed),
                            Some(ExecutionStatus::Failed),
                            Some(ExecutionStatus::Cancelled),
                        ]
                        .into_iter()
                        .map(|status| {
                            let selected = self.history_status_filter == status;
                            Radio::new(format!("history-status-{status:?}"))
                                .label(status.map_or("All", |value| match value {
                                    ExecutionStatus::Completed => "Completed",
                                    ExecutionStatus::Failed => "Failed",
                                    ExecutionStatus::Cancelled => "Cancelled",
                                    _ => "Other",
                                }))
                                .checked(selected)
                                .on_change(cx.listener(move |this, checked, _, cx| {
                                    if *checked {
                                        this.history_status_filter = status;
                                        this.load_history(None, cx);
                                    }
                                }))
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .flex_1()
                    .overflow_hidden()
                    .child(div().w(relative(0.46)).h_full().child(list))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .overflow_y_scrollbar()
                            .child(detail_panel),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("history-previous")
                            .label("Previous")
                            .disabled(self.history_back.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(cursor) = this.history_back.pop() {
                                    this.load_history(cursor, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("history-next")
                            .label("Next")
                            .disabled(self.history_page.next_cursor.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(next) = this.history_page.next_cursor.clone() {
                                    this.history_back.push(this.history_cursor.clone());
                                    this.load_history(Some(next), cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("clear-history-preserve-favorites")
                            .danger()
                            .label(
                                if self.clear_armed == Some(ClearHistoryMode::PreserveFavorites) {
                                    "Confirm clear"
                                } else {
                                    "Clear history"
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_history(ClearHistoryMode::PreserveFavorites, cx)
                            })),
                    )
                    .child(
                        Button::new("clear-history-all")
                            .danger()
                            .label(
                                if self.clear_armed == Some(ClearHistoryMode::IncludeFavorites) {
                                    "Confirm delete all"
                                } else {
                                    "Delete all + favorites"
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_history(ClearHistoryMode::IncludeFavorites, cx)
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.history_status.clone()),
                    ),
            )
            .into_any_element()
    }

    fn render_privacy(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let diagnostics = self.history.diagnostics();
        let report = format!(
            "LexWisp diagnostics (redacted)\ndata={}\nsettings={}\ndatabase={}\nrecording={}\nretention_generation={}\nactive_executions={}\nCredentials and content bodies are omitted.",
            diagnostics.data_directory.display(),
            diagnostics.settings_path.display(),
            diagnostics.database_path.display(),
            diagnostics.recording_enabled,
            diagnostics.retention_generation,
            diagnostics.active_executions
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_lg().child("Privacy & diagnostics"))
            .child(
                div().flex().items_center().justify_between()
                    .child(div().flex().flex_col().gap_1().child("Automatically record submitted inputs and results").child(
                        div().text_sm().text_color(cx.theme().muted_foreground).child("Turning this off revokes running write eligibility; already-saved content remains."),
                    ))
                    .child(Switch::new("recording-enabled").accessibility_label("Automatically record history").checked(self.draft.recording_enabled()).on_change(cx.listener(|this, checked, _, cx| {
                        this.draft = this.draft.clone().with_recording_enabled(*checked);
                        cx.notify();
                    }))),
            )
            .child(div().text_sm().child(format!("Data: {}", diagnostics.data_directory.display())))
            .child(div().text_sm().child(format!("Logs: {}", diagnostics.log_directory.display())))
            .child(div().text_sm().child("API keys are stored in Windows Credential Manager and do not move with the portable data directory."))
            .child(
                div().flex().gap_2()
                    .child(Button::new("save-privacy").primary().label("Save settings").disabled(self.saving).on_click(cx.listener(|this, _, window, cx| this.save(window, cx))))
                    .child(Button::new("reload-settings").label("Reload config").disabled(self.saving).on_click(cx.listener(|this, _, window, cx| this.reload_settings(window, cx))))
                    .child(Button::new("create-backup").label("Create backup").on_click(cx.listener(|this, _, _, cx| this.create_backup(cx))))
                    .child(Button::new("copy-diagnostics").label("Copy redacted diagnostics").on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(report.clone())))),
            )
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(self.status.clone()))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(self.history_status.clone()))
            .overflow_y_scrollbar()
            .into_any_element()
    }
}

impl Render for ControlCenter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let navigation = div()
            .flex()
            .gap_2()
            .child(
                Button::new("page-settings")
                    .label("Settings")
                    .primary()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page = ControlPage::Settings;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("page-history")
                    .label("History & favorites")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page = ControlPage::History;
                        this.load_history(None, cx);
                    })),
            )
            .child(
                Button::new("page-privacy")
                    .label("Privacy & diagnostics")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page = ControlPage::Privacy;
                        cx.notify();
                    })),
            );
        if self.page == ControlPage::History {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .p_6()
                .gap_4()
                .child(navigation)
                .child(self.render_history(window, cx))
                .into_any_element();
        }
        if self.page == ControlPage::Privacy {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .p_6()
                .gap_4()
                .child(navigation)
                .child(self.render_privacy(window, cx))
                .into_any_element();
        }
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
        let launch_modes = LaunchMode::ALL.into_iter().map(|mode| {
            Radio::new(format!("launch-mode-{mode:?}"))
                .label(mode.label())
                .checked(self.draft.launch_mode() == mode)
                .on_change(cx.listener(move |this, checked, _, cx| {
                    if *checked {
                        this.draft = this.draft.clone().with_launch_mode(mode);
                        cx.notify();
                    }
                }))
        });
        let default_actions = std::iter::once(
            Radio::new("default-action-none")
                .label("Not configured")
                .checked(self.draft.default_action_id().is_none())
                .on_change(cx.listener(|this, checked, _, cx| {
                    if *checked {
                        this.draft = this.draft.clone().with_default_action_id(None);
                        cx.notify();
                    }
                })),
        )
        .chain(self.actions.clone().into_iter().map(|descriptor| {
            let action = descriptor.qualified_id().to_string();
            let selected = self.draft.default_action_id() == Some(action.as_str());
            Radio::new(format!("default-action-{action}"))
                .label(descriptor.display_name().to_owned())
                .checked(selected)
                .on_change(cx.listener(move |this, checked, _, cx| {
                    if *checked {
                        this.draft = this
                            .draft
                            .clone()
                            .with_default_action_id(Some(action.clone()));
                        cx.notify();
                    }
                }))
        }));
        let dismiss_controls = self
            .actions
            .clone()
            .into_iter()
            .filter(|descriptor| matches!(descriptor.kind(), ActionKind::Declarative(_)))
            .map(|descriptor| {
                let action = descriptor.qualified_id().to_string();
                let current = self.draft.dismiss_override(&action);
                let default_action = action.clone();
                let cancel_action = action.clone();
                let continue_action = action.clone();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child(descriptor.display_name().to_owned()))
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(
                                Radio::new(format!("dismiss-{action}-default"))
                                    .label("Plugin default")
                                    .checked(current.is_none())
                                    .on_change(cx.listener(move |this, checked, _, cx| {
                                        if *checked {
                                            this.draft = this.draft.clone().with_dismiss_override(
                                                default_action.clone(),
                                                None,
                                            );
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                Radio::new(format!("dismiss-{action}-cancel"))
                                    .label("Cancel when hidden")
                                    .checked(current == Some(DismissPolicy::Cancel))
                                    .on_change(cx.listener(move |this, checked, _, cx| {
                                        if *checked {
                                            this.draft = this.draft.clone().with_dismiss_override(
                                                cancel_action.clone(),
                                                Some(DismissPolicy::Cancel),
                                            );
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                Radio::new(format!("dismiss-{action}-continue"))
                                    .label("Continue in background")
                                    .checked(current == Some(DismissPolicy::Continue))
                                    .on_change(cx.listener(move |this, checked, _, cx| {
                                        if *checked {
                                            this.draft = this.draft.clone().with_dismiss_override(
                                                continue_action.clone(),
                                                Some(DismissPolicy::Continue),
                                            );
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
            });
        let parameter_controls = self.actions.clone().into_iter().filter_map(|descriptor| {
            let ActionKind::Declarative(definition) = descriptor.kind().clone() else {
                return None;
            };
            let action = descriptor.qualified_id().to_string();
            let mut group = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().child(descriptor.display_name().to_owned()));
            for parameter in definition.parameters {
                let current = self
                    .draft
                    .action_parameter_defaults(&action)
                    .and_then(|values| values.get(&parameter.key))
                    .cloned()
                    .or_else(|| parameter.default_value.clone())
                    .unwrap_or_default();
                if parameter.kind == ParameterKind::Enum {
                    let action_id = action.clone();
                    let key = parameter.key.clone();
                    let choices = parameter.choices.into_iter().map(|choice| {
                        let action_id = action_id.clone();
                        let key = key.clone();
                        let selected = current == choice;
                        let saved = choice.clone();
                        Radio::new(format!("parameter-{action_id}-{key}-{choice}"))
                            .label(choice)
                            .checked(selected)
                            .on_change(cx.listener(move |this, checked, _, cx| {
                                if *checked {
                                    this.draft = this.draft.clone().with_action_parameter_default(
                                        action_id.clone(),
                                        key.clone(),
                                        saved.clone(),
                                    );
                                    cx.notify();
                                }
                            }))
                    });
                    group = group.child(setting_group("Options", choices));
                } else if let Some((_, _, input)) =
                    self.action_parameter_inputs
                        .iter()
                        .find(|(candidate_action, key, _)| {
                            candidate_action == &action && key == &parameter.key
                        })
                {
                    group = group.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child(parameter.label))
                            .child(Input::new(input).aria_label("Action parameter default")),
                    );
                }
            }
            Some(group)
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .p_6()
            .gap_5()
            .child(navigation)
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
                    .child(div().text_sm().child("Advanced provider settings"))
                    .child(
                        div().grid().grid_cols(2).gap_3()
                            .child(labeled_input("Context budget", &self.provider_context_budget, "Provider context budget"))
                            .child(labeled_input("Explicit proxy", &self.provider_proxy, "Provider proxy URL"))
                            .child(labeled_input("Connect timeout (seconds)", &self.provider_connect_timeout, "Provider connect timeout"))
                            .child(labeled_input("Total timeout (seconds)", &self.provider_total_timeout, "Provider total timeout"))
                            .child(labeled_input("Stream event timeout (seconds)", &self.provider_event_timeout, "Provider event timeout"))
                            .child(labeled_input("Temperature (0–2)", &self.provider_temperature, "Provider temperature"))
                            .child(labeled_input("Max output tokens", &self.provider_max_output_tokens, "Provider max output tokens")),
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
            .child(div().text_lg().child("Quick Shell behavior"))
            .child(setting_group("Shortcut launch mode", launch_modes))
            .child(setting_group("Default action", default_actions))
            .child(setting_group("Dismiss behavior", dismiss_controls))
            .child(setting_group("Action parameter defaults", parameter_controls))
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
            .into_any_element()
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

fn labeled_input(
    label: &'static str,
    state: &Entity<InputState>,
    aria_label: &'static str,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_sm().child(label))
        .child(Input::new(state).aria_label(aria_label))
}
