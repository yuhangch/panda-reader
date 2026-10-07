use crate::services::Command;
use crate::ui::i18n;
use crate::ui::window::ReaderWindow;
use gpui_kit::*;
use panda_core::Scope;
use panda_providers::{ProviderKind, ProviderSettings};
use tokio::sync::oneshot;

impl ReaderWindow {
    pub(in crate::ui) fn connect_provider(&mut self, cx: &mut Context<Self>) {
        if self.settings.is_connecting {
            return;
        }
        let Some(kind) = self.settings.library_source.provider() else {
            return;
        };
        let endpoint = self
            .settings
            .provider_url_input
            .read(cx)
            .value()
            .to_string();
        let username = self
            .settings
            .provider_username_input
            .read(cx)
            .value()
            .to_string();
        let secret = self
            .settings
            .provider_secret_input
            .read(cx)
            .value()
            .to_string();
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Connect {
            kind,
            endpoint: endpoint.clone(),
            username: username.clone(),
            secret: secret.clone(),
            reply,
        });
        self.settings.is_connecting = true;
        self.set_busy(self.t("Connecting upstream…"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".into()));
            let _ = this.update(cx, |this, cx| {
                this.settings.is_connecting = false;
                match result {
                    Ok(outcome) => {
                        this.settings.provider_settings.insert(
                            kind,
                            ProviderSettings {
                                endpoint,
                                username: if kind == ProviderKind::FreshRss {
                                    username
                                } else {
                                    String::new()
                                },
                                secret,
                            },
                        );
                        if let Some(error) = outcome.initial_sync_error {
                            this.set_error(format!("Initial sync failed: {error}"));
                        } else {
                            this.record_refresh_success();
                            this.set_flash(
                                i18n::format(
                                    this.preferences.language,
                                    "Connected: {}",
                                    outcome.account_name,
                                ),
                                cx,
                            );
                        }
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn disconnect_provider(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.settings.library_source.provider() else {
            return;
        };
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Disconnect { kind, reply });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".into()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.settings.provider_settings.remove(&kind);
                        this.set_flash(
                            this.t("Disconnected; cached articles remain available"),
                            cx,
                        );
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn add_feed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.settings.add_input.read(cx).value().trim().to_string();
        if url.is_empty() {
            return;
        }
        let _ = self
            .settings
            .add_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        let (reply, response) = oneshot::channel();
        self.services.send(Command::AddFeed { url, reply });
        self.set_busy(self.t("Adding feed…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.set_flash(this.t("Feed added"), cx);
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn remove_feed(&mut self, id: i64, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.services.send(Command::RemoveFeed { id, reply });
        self.set_busy(self.t("Removing feed…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                this.sidebar.pending_remove_feed = None;
                match result {
                    Ok(()) => {
                        this.reader.request_epoch.next();
                        this.reader.is_extracting = false;
                        this.reader.article = None;
                        this.reader.body_html = SharedString::default();
                        this.reader.body_markdown = SharedString::default();
                        this.reader.showing_translation = false;
                        this.list.scope = Scope::All;
                        this.set_flash(this.t("Feed removed from provider and locally"), cx);
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_with_force(false, cx);
    }

    pub(in crate::ui) fn refresh_with_force(&mut self, force: bool, cx: &mut Context<Self>) {
        if self.sidebar.is_refreshing {
            return;
        }
        let (reply, response) = oneshot::channel();
        self.services.send(Command::Refresh { force, reply });
        self.sidebar.is_refreshing = true;
        self.set_busy(self.t("Syncing…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                this.sidebar.is_refreshing = false;
                match result {
                    Ok(count) => {
                        this.record_refresh_success();
                        this.set_flash(
                            i18n::format(this.preferences.language, "Synced {} articles", count),
                            cx,
                        );
                    }
                    Err(error) => this.set_error(error),
                }
                this.load_snapshot(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn import_opml(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        let services = self.services.clone();
        cx.spawn(async move |_, cx| {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter("OPML", &["opml", "xml"])
                .pick_file()
                .await
            else {
                return;
            };
            let content = String::from_utf8_lossy(&file.read().await).into_owned();
            let (reply, response) = oneshot::channel();
            services.send(Command::ImportOpml { content, reply });
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(count) => {
                        this.set_flash(
                            i18n::format(this.preferences.language, "Imported {} feeds", count),
                            cx,
                        );
                    }
                    Err(error) => this.set_error(error),
                }
                this.load_snapshot(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn export_opml(&mut self, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.services.send(Command::ExportOpml { reply });
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            match result {
                Ok(content) => {
                    if let Some(file) = rfd::AsyncFileDialog::new()
                        .set_file_name("panda-reader.opml")
                        .add_filter("OPML", &["opml"])
                        .save_file()
                        .await
                    {
                        match file.write(content.as_bytes()).await {
                            Ok(()) => {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_flash(this.t("OPML exported"), cx);
                                    cx.notify();
                                });
                            }
                            Err(error) => {
                                let _ = weak.update(cx, |this, cx| {
                                    this.set_error(error.to_string());
                                    cx.notify();
                                });
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = weak.update(cx, |this, cx| {
                        this.set_error(error);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(in crate::ui) fn upstream_provider_label(&self) -> Option<&'static str> {
        self.settings.library_source.provider().and_then(|kind| {
            self.settings
                .provider_settings
                .contains_key(&kind)
                .then_some(self.settings.library_source.label())
        })
    }
}
impl ReaderWindow {
    pub(in crate::ui) fn refresh_one_feed(&mut self, id: i64, cx: &mut Context<Self>) {
        let (reply, response) = oneshot::channel();
        self.services.send(Command::RefreshFeed { id, reply });
        self.set_busy(self.t("Refreshing feed…"));
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".to_string()));
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.record_refresh_success();
                        this.set_flash(this.t("Feed refreshed"), cx);
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_error(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn set_feed_auto_translate_titles(
        &mut self,
        id: i64,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let (reply, response) = oneshot::channel();
        self.services
            .send(Command::SetFeedAutoTranslateTitles { id, enabled, reply });
        cx.spawn(async move |this, cx| {
            let result = response
                .await
                .unwrap_or_else(|_| Err("Background service stopped".into()));
            let _ = this.update(cx, |this, cx| match result {
                Ok(()) => {
                    if this.editor.feed_id == Some(id) {
                        this.editor.feed_id = None;
                    }
                    this.set_flash(
                        this.t(if enabled {
                            "Title auto-translation enabled for feed"
                        } else {
                            "Title auto-translation disabled for feed"
                        }),
                        cx,
                    );
                    this.load_snapshot(cx);
                }
                Err(error) => this.set_error(error),
            });
        })
        .detach();
    }
}
