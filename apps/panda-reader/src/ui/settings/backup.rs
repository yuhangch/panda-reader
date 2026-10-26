use super::Settings;
use crate::services::Command;
use crate::ui::components::text_input;
use crate::ui::window::ReaderWindow;
use anyhow::Context as _;
use gpui_kit::base::StyledExt as _;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    select::Select,
    switch::Switch,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use panda_backup::{BackupMode, BackupPayload};
use panda_store::Store;
use std::collections::BTreeMap;
use tokio::sync::oneshot;

impl Settings {
    pub(in crate::ui) fn render_backup_settings(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(self.settings_heading(owner, "Backup"))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Create a local backup file or restore one. Configuration only merges your Local subscriptions and settings. Full backup includes articles and reading history and restores at next launch."))
            .child(div().pb_1().text_sm().child(owner.t("Backup type")))
            .child(div().w(px(320.)).child(Select::new(&self.backup_mode_select).small().placeholder(owner.t("Backup type"))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Local RSS URLs can contain access tokens. Plain local backups are not encrypted."))
            .child(h_flex().items_center().justify_between().py_2()
                .child(div().text_sm().child("Encrypt local backup with age"))
                .child(Switch::new("backup-encrypt-local").checked(self.backup_encrypt_local).accessibility_label("Encrypt local backup with age").on_change(cx.listener(|this, checked, _, cx| { this.settings.backup_encrypt_local = *checked; cx.notify(); }))))
            .when(self.backup_encrypt_local, |view| view.child(text_input(&self.backup_local_password_input).mask_toggle()))
            .child(h_flex().gap_2().pt_2()
                .child(Button::new("backup-export").small().primary().label("Export backup").on_click(cx.listener(|this, _, window, cx| this.export_backup(window, cx))))
                .child(Button::new("backup-import").small().secondary().label("Import backup").on_click(cx.listener(|this, _, window, cx| this.import_backup(window, cx)))))
            .child(div().my_4().border_t_1().border_color(cx.theme().border))
            .child(div().font_semibold().child("Cloud storage"))
            .child(div().pb_1().text_sm().child("Storage target"))
            .child(div().w(px(320.)).child(Select::new(&self.backup_target_select).small().placeholder("Storage target")))
            .child(text_input(&self.backup_cloud_endpoint_input))
            .when(!self.backup_cloud_s3, |view| view.child(text_input(&self.backup_cloud_username_input)).child(text_input(&self.backup_cloud_password_input).mask_toggle()).child(text_input(&self.backup_cloud_directory_input)))
            .when(self.backup_cloud_s3, |view| view.child(text_input(&self.backup_cloud_region_input)).child(text_input(&self.backup_cloud_bucket_input)).child(text_input(&self.backup_cloud_prefix_input)).child(text_input(&self.backup_cloud_access_input).mask_toggle()).child(text_input(&self.backup_cloud_secret_input).mask_toggle()).child(text_input(&self.backup_cloud_session_input).mask_toggle()).child(h_flex().items_center().justify_between().py_1().child(div().text_sm().child("Use path style (MinIO / R2)")).child(Switch::new("backup-cloud-path-style").checked(self.backup_cloud_path_style).on_change(cx.listener(|this, checked, _, cx| { this.settings.backup_cloud_path_style = *checked; cx.notify(); })))))
            .child(h_flex().items_center().justify_between().py_1().child(div().text_sm().child("Allow insecure HTTP for this endpoint")).child(Switch::new("backup-cloud-insecure-http").checked(self.backup_cloud_allow_insecure_http).on_change(cx.listener(|this, checked, _, cx| { this.settings.backup_cloud_allow_insecure_http = *checked; cx.notify(); }))))
            .when(self.backup_cloud_credentials_saved, |view| view.child(div().text_xs().text_color(cx.theme().muted_foreground).child("Cloud credentials are saved. Leave secret fields empty to keep them.")))
            .child(h_flex().gap_2().pt_2().child(Button::new("backup-cloud-test").small().secondary().label("Test connection").on_click(cx.listener(|this, _, window, cx| this.test_backup_target(window, cx)))).child(Button::new("backup-cloud-save").small().secondary().label("Save target").on_click(cx.listener(|this, _, window, cx| this.save_backup_target(window, cx)))))
            .child(div().pt_2().text_sm().child("Cloud backups always require an age password. It is never saved."))
            .child(text_input(&self.backup_password_input).mask_toggle())
            .child(h_flex().gap_2().pt_2().child(Button::new("backup-cloud-now").small().primary().label("Back up now").on_click(cx.listener(|this, _, window, cx| this.backup_to_cloud(window, cx)))).child(Button::new("backup-cloud-history").small().secondary().label("Refresh history").on_click(cx.listener(|this, _, window, cx| this.refresh_cloud_history(window, cx)))))
            .child(v_flex().gap_2().pt_2().children(self.backup_cloud_items.iter().cloned().map(|item| {
                let label = format!("{} · {} · {} bytes", item.created_at, item.mode, item.encrypted_bytes);
                let download_item = item.clone();
                let restore_item = item.clone();
                h_flex().items_center().justify_between().child(div().text_xs().child(label)).child(h_flex().gap_1().child(Button::new(format!("backup-download-{}", item.archive_name)).small().secondary().label("Download").on_click(cx.listener(move |this, _, window, cx| this.download_cloud_backup(download_item.clone(), window, cx)))).child(Button::new(format!("backup-restore-{}", item.archive_name)).small().secondary().label("Restore").on_click(cx.listener(move |this, _, window, cx| this.restore_cloud_backup(restore_item.clone(), window, cx)))).child(Button::new(format!("backup-delete-{}", item.archive_name)).small().secondary().label("Delete").on_click(cx.listener(move |this, _, window, cx| this.delete_cloud_backup(item.clone(), window, cx)))))
            })))
            .when_some(self.backup_status.clone(), |view, status| view.child(div().pt_2().text_sm().child(status)))
    }
}

impl ReaderWindow {
    fn build_backup_target(&self, cx: &App) -> anyhow::Result<panda_backup::BackupTargetConfig> {
        let existing = panda_backup::load_targets(&self.data_dir.join("backup-targets.json"))?
            .into_iter()
            .next();
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        if self.settings.backup_cloud_s3 {
            let previous = match existing {
                Some(panda_backup::BackupTargetConfig::S3(target)) => Some(target),
                _ => None,
            };
            let access = value(&self.settings.backup_cloud_access_input);
            let secret = value(&self.settings.backup_cloud_secret_input);
            let token = value(&self.settings.backup_cloud_session_input);
            let endpoint = value(&self.settings.backup_cloud_endpoint_input);
            Ok(panda_backup::BackupTargetConfig::S3(
                panda_backup::S3Target {
                    endpoint: (!endpoint.is_empty()).then_some(endpoint),
                    region: value(&self.settings.backup_cloud_region_input),
                    bucket: value(&self.settings.backup_cloud_bucket_input),
                    prefix: value(&self.settings.backup_cloud_prefix_input),
                    access_key: if access.is_empty() {
                        previous
                            .as_ref()
                            .map(|old| old.access_key.clone())
                            .unwrap_or_default()
                    } else {
                        access
                    },
                    secret_key: if secret.is_empty() {
                        previous
                            .as_ref()
                            .map(|old| old.secret_key.clone())
                            .unwrap_or_default()
                    } else {
                        secret
                    },
                    session_token: if token.is_empty() {
                        previous.and_then(|old| old.session_token)
                    } else {
                        Some(token)
                    },
                    path_style: self.settings.backup_cloud_path_style,
                    allow_insecure_http: self.settings.backup_cloud_allow_insecure_http,
                },
            ))
        } else {
            let previous = match existing {
                Some(panda_backup::BackupTargetConfig::WebDav(target)) => Some(target),
                _ => None,
            };
            let password = value(&self.settings.backup_cloud_password_input);
            Ok(panda_backup::BackupTargetConfig::WebDav(
                panda_backup::WebDavTarget {
                    endpoint: value(&self.settings.backup_cloud_endpoint_input),
                    username: value(&self.settings.backup_cloud_username_input),
                    password: if password.is_empty() {
                        previous.map(|old| old.password).unwrap_or_default()
                    } else {
                        password
                    },
                    directory: value(&self.settings.backup_cloud_directory_input),
                    allow_insecure_http: self.settings.backup_cloud_allow_insecure_http,
                },
            ))
        }
    }

    pub(in crate::ui) fn save_backup_target(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.build_backup_target(cx) {
            Ok(target) => match panda_backup::save_targets(
                &self.data_dir.join("backup-targets.json"),
                &[target],
            ) {
                Ok(()) => {
                    self.settings.backup_cloud_credentials_saved = true;
                    for input in [
                        &self.settings.backup_cloud_password_input,
                        &self.settings.backup_cloud_access_input,
                        &self.settings.backup_cloud_secret_input,
                        &self.settings.backup_cloud_session_input,
                    ] {
                        input.update(cx, |state, cx| state.set_value("", window, cx));
                    }
                    self.settings.backup_status = Some("Cloud backup target saved".into());
                }
                Err(_) => {
                    self.settings.backup_status = Some("Cloud target could not be saved".into())
                }
            },
            Err(_) => self.settings.backup_status = Some("Cloud target is invalid".into()),
        }
        cx.notify();
    }

    pub(in crate::ui) fn test_backup_target(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match self.build_backup_target(cx) {
            Ok(target) => target,
            Err(_) => {
                self.settings.backup_status = Some("Cloud target is invalid".into());
                cx.notify();
                return;
            }
        };
        self.settings.backup_status = Some("Testing cloud connection…".into());
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let result = cx.background_executor().spawn(async move {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                runtime.block_on(async { panda_backup::RemoteStorage::new(&target)?.test_connection().await })
            }).await;
            let _ = weak.update(cx, |this, cx| {
                this.settings.backup_status = Some(match result { Ok(()) => "Cloud storage connection succeeded".into(), Err(_) => "Cloud storage connection failed; check endpoint, credentials, permissions and network".into() });
                cx.notify();
            });
        }).detach();
    }

    pub(in crate::ui) fn backup_to_cloud(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let target = match self.build_backup_target(cx) {
            Ok(target) => target,
            Err(_) => {
                self.settings.backup_status = Some("Cloud target is invalid".into());
                cx.notify();
                return;
            }
        };
        let password = self
            .settings
            .backup_password_input
            .read(cx)
            .value()
            .to_string();
        if password.chars().count() < 8 {
            self.settings.backup_status =
                Some("Enter a cloud backup password with at least 8 characters".into());
            cx.notify();
            return;
        }
        let mode = if self.settings.backup_full_mode {
            BackupMode::Full
        } else {
            BackupMode::Config
        };
        let data_dir = self.data_dir.clone();
        let preferences = serde_json::to_value(&self.preferences)
            .map(|value| {
                serde_json::to_vec_pretty(&panda_backup::safe_preferences(&value))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let translator = serde_json::to_value(&self.translator_config)
            .map(|value| {
                serde_json::to_vec_pretty(&panda_backup::safe_translator_config(&value))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        let hints = serde_json::to_value(&self.settings.provider_settings)
            .map(|value| {
                serde_json::to_vec_pretty(&panda_backup::safe_provider_hints(&value))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        self.settings.backup_status = Some("Creating encrypted cloud backup…".into());
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let scratch = tempfile::tempdir()?;
                    let archive_path = scratch.path().join(panda_backup::suggested_filename(mode));
                    let manifest = write_backup_archive(
                        &data_dir,
                        mode,
                        &archive_path,
                        preferences,
                        translator,
                        hints,
                        scratch.path(),
                    )?;
                    let encrypted_name = format!(
                        "{}.age",
                        archive_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    );
                    let encrypted_path = scratch.path().join(&encrypted_name);
                    panda_backup::encrypt_file(&archive_path, &encrypted_path, &password)?;
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    let item = runtime.block_on(async {
                        panda_backup::RemoteStorage::new(&target)?
                            .upload(
                                &encrypted_path,
                                &encrypted_name,
                                mode.label(),
                                &manifest.created_at,
                            )
                            .await
                    })?;
                    Ok::<_, anyhow::Error>(item)
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(item) => {
                        this.settings.backup_status =
                            Some("Encrypted cloud backup uploaded".into());
                        this.settings.backup_cloud_items.insert(0, item);
                    }
                    Err(_) => {
                        this.settings.backup_status =
                            Some("Cloud backup failed; check target permissions and network".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn refresh_cloud_history(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match self.build_backup_target(cx) {
            Ok(target) => target,
            Err(_) => {
                self.settings.backup_status = Some("Cloud target is invalid".into());
                cx.notify();
                return;
            }
        };
        self.settings.backup_status = Some("Loading cloud backup history…".into());
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime
                        .block_on(async { panda_backup::RemoteStorage::new(&target)?.list().await })
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(items) => {
                        this.settings.backup_cloud_items = items;
                        this.settings.backup_status = Some("Cloud backup history updated".into());
                    }
                    Err(_) => {
                        this.settings.backup_status =
                            Some("Cloud backup history could not be loaded".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn download_cloud_backup(
        &mut self,
        item: panda_backup::BackupItem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match self.build_backup_target(cx) {
            Ok(target) => target,
            Err(_) => {
                self.settings.backup_status = Some("Cloud target is invalid".into());
                cx.notify();
                return;
            }
        };
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_file_name(&item.archive_name)
                .add_filter("Encrypted backup", &["age"])
                .save_file()
                .await
            else {
                return;
            };
            let destination = file.path().to_path_buf();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(async {
                        panda_backup::RemoteStorage::new(&target)?
                            .download(&item, &destination)
                            .await
                    })
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                this.settings.backup_status = Some(match result {
                    Ok(()) => "Encrypted cloud backup downloaded".into(),
                    Err(_) => {
                        "Cloud backup download failed checksum or transport validation".into()
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn restore_cloud_backup(
        &mut self,
        item: panda_backup::BackupItem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match self.build_backup_target(cx) {
            Ok(target) => target,
            Err(_) => {
                self.settings.backup_status = Some("Cloud target is invalid".into());
                cx.notify();
                return;
            }
        };
        let password = self
            .settings
            .backup_password_input
            .read(cx)
            .value()
            .to_string();
        if password.chars().count() < 8 {
            self.settings.backup_status = Some("Enter the cloud backup password first".into());
            cx.notify();
            return;
        }
        let data_dir = self.data_dir.clone();
        let services = self.services.clone();
        let restore_confirmation = rfd::AsyncMessageDialog::new()
            .set_title("Confirm Panda Reader backup restore")
            .set_description("Configuration only merges Local subscriptions and preferences while keeping existing articles and reading state. A Full backup replaces the entire library after restart. Continue with restore?")
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_parent(_window);
        self.settings.backup_status = Some("Downloading and validating cloud backup…".into());
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let loaded = cx.background_executor().spawn(async move {
                let scratch = tempfile::tempdir()?;
                let encrypted = scratch.path().join(&item.archive_name);
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
                runtime.block_on(async { panda_backup::RemoteStorage::new(&target)?.download(&item, &encrypted).await })?;
                let decrypted = scratch.path().join("decrypted.prbackup");
                panda_backup::decrypt_file(&encrypted, &decrypted, &password)?;
                panda_backup::read_archive(&decrypted)
            }).await;
            let archive = match loaded { Ok(archive) => archive, Err(_) => { let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some("Cloud restore failed; check password, integrity and network".into()); cx.notify(); }); return; } };
            if restore_confirmation.show().await != rfd::MessageDialogResult::Yes {
                let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some("Cloud restore cancelled".into()); cx.notify(); });
                return;
            }
            if archive.manifest.mode == BackupMode::Full {
                let result = stage_full_restore(&data_dir, archive);
                let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some(match result { Ok(()) => "Full backup staged. Restart Panda Reader to complete the restore.".into(), Err(_) => "Full restore could not be staged".into() }); cx.notify(); });
                return;
            }
            let feeds: Vec<panda_store::BackupFeed> = match archive.files.get("local-feeds.json").and_then(|bytes| serde_json::from_slice(bytes).ok()) { Some(feeds) => feeds, None => { let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some("Cloud backup has invalid Local subscriptions".into()); cx.notify(); }); return; } };
            let mut prefs: crate::app::preferences::Preferences = match archive.files.get("preferences.json").and_then(|bytes| serde_json::from_slice(bytes).ok()) { Some(prefs) => prefs, None => { let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some("Cloud backup has invalid preferences".into()); cx.notify(); }); return; } };
            prefs.library_source = crate::app::preferences::LibrarySource::Local;
            prefs.last_refresh_at = None;
            if let Err(error) = (crate::services::ArticleFetchNetwork {
                mode: prefs.article_network_mode,
                proxy_url: prefs.article_proxy_url.clone(),
                trusted_fake_ip_ranges: prefs.trusted_proxy_fake_ip_ranges.clone(),
            }).validate() {
                let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some(format!("Cloud backup has invalid network preferences: {error}")); cx.notify(); });
                return;
            }
            let translator = archive.files.get("translator-public.json").and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok()).map(|value| panda_backup::safe_translator_config(&value));
            let (reply, response) = oneshot::channel();
            services.send(Command::ImportBackupFeeds { feeds, reply });
            let result = response.await.unwrap_or_else(|_| Err("Background service stopped".to_owned()));
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(count) => {
                        this.preferences = prefs;
                        this.save_preferences();
                        this.services.set_workspace("local");
                        this.services.set_article_fetch_network(crate::services::ArticleFetchNetwork {
                            mode: this.preferences.article_network_mode,
                            proxy_url: this.preferences.article_proxy_url.clone(),
                            trusted_fake_ip_ranges: this.preferences.trusted_proxy_fake_ip_ranges.clone(),
                        });
                        this.services.set_detailed_sync_logging(this.preferences.detailed_sync_logging);
                        this.services.set_log_retention_days(this.preferences.sync_log_retention_days);
                        let imported_preferences = this.preferences.clone();
                        this.update_article_reader(cx, |reader| { reader.preferences = imported_preferences; });
                        if let Some(public) = translator {
                            if let Ok(mut current) = serde_json::to_value(&this.translator_config) {
                                if let (Some(current), Some(public)) = (current.as_object_mut(), public.as_object()) { for (key, value) in public { current.insert(key.clone(), value.clone()); } }
                                if let Ok(config) = serde_json::from_value(current) {
                                    this.translator_config = config;
                                    this.save_translator_config();
                                    let imported_translator = this.translator_config.clone();
                                    this.update_article_reader(cx, |reader| { reader.translator_config = imported_translator; });
                                }
                            }
                        }
                        this.settings.backup_status = Some(format!("Cloud configuration restored; merged {count} Local subscriptions. No feeds were fetched."));
                        this.load_snapshot(cx);
                    }
                    Err(_) => this.settings.backup_status = Some("Cloud configuration restore failed".into()),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(in crate::ui) fn delete_cloud_backup(
        &mut self,
        item: panda_backup::BackupItem,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match self.build_backup_target(cx) {
            Ok(target) => target,
            Err(_) => {
                self.settings.backup_status = Some("Cloud target is invalid".into());
                cx.notify();
                return;
            }
        };
        let archive_name = item.archive_name.clone();
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()?;
                    runtime.block_on(async {
                        panda_backup::RemoteStorage::new(&target)?
                            .delete(&item)
                            .await
                    })
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.settings
                            .backup_cloud_items
                            .retain(|item| item.archive_name != archive_name);
                        this.settings.backup_status = Some("Cloud backup deleted".into());
                    }
                    Err(_) => {
                        this.settings.backup_status =
                            Some("Cloud backup could not be deleted".into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn export_backup(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<ReaderWindow>,
    ) {
        let mode = if self.settings.backup_full_mode {
            BackupMode::Full
        } else {
            BackupMode::Config
        };
        let encrypt_local = self.settings.backup_encrypt_local;
        let password = if encrypt_local {
            self.settings
                .backup_local_password_input
                .read(cx)
                .value()
                .to_string()
        } else {
            String::new()
        };
        if encrypt_local && password.chars().count() < 8 {
            self.settings.backup_status =
                Some("Enter a backup password with at least 8 characters".into());
            cx.notify();
            return;
        }
        let data_dir = self.data_dir.clone();
        let prefs = match serde_json::to_value(&self.preferences) {
            Ok(value) => panda_backup::safe_preferences(&value),
            Err(error) => {
                self.settings.backup_status = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let translator = match serde_json::to_value(&self.translator_config) {
            Ok(value) => panda_backup::safe_translator_config(&value),
            Err(error) => {
                self.settings.backup_status = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let provider_hints = match serde_json::to_value(&self.settings.provider_settings) {
            Ok(value) => panda_backup::safe_provider_hints(&value),
            Err(error) => {
                self.settings.backup_status = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let prefs = serde_json::to_vec_pretty(&prefs).unwrap_or_default();
        let translator = serde_json::to_vec_pretty(&translator).unwrap_or_default();
        let provider_hints = serde_json::to_vec_pretty(&provider_hints).unwrap_or_default();
        self.settings.backup_status = Some("Preparing backup…".into());
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let mut suggested = panda_backup::suggested_filename(mode);
            if encrypt_local { suggested.push_str(".age"); }
            let dialog = rfd::AsyncFileDialog::new().set_file_name(&suggested);
            let dialog = if encrypt_local { dialog.add_filter("Encrypted Panda Reader backup", &["age"]) } else { dialog.add_filter("Panda Reader backup", &["prbackup"]) };
            let Some(file) = dialog.save_file().await
            else {
                return;
            };
            let destination = file.path().to_path_buf();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let db = data_dir.join("panda-reader.sqlite3");
                    let scratch = tempfile::tempdir()?;
                    let archive_path = if encrypt_local { scratch.path().join("unencrypted.prbackup") } else { destination.clone() };
                    let (local_feeds, database, record_counts) = match mode {
                        BackupMode::Config => {
                            let store = Store::open_read_workspace(&db, "local")?;
                            let feeds = store.export_local_backup_feeds()?;
                            let count = feeds.len() as u64;
                            (
                                Some(serde_json::to_vec_pretty(&feeds)?),
                                None,
                                BTreeMap::from([("local_feeds".into(), count)]),
                            )
                        }
                        BackupMode::Full => {
                            let snapshot = scratch.path().join("library.sqlite3");
                            panda_backup::create_sqlite_snapshot(&db, &snapshot)?;
                            let connection = rusqlite::Connection::open_with_flags(&snapshot, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                            let mut counts = BTreeMap::new();
                            for table in ["feeds", "articles", "article_reading_positions", "article_translation_segments"] {
                                let exists: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)", [table], |row| row.get(0))?;
                                if exists {
                                    let count: i64 = connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))?;
                                    counts.insert(table.to_owned(), count as u64);
                                }
                            }
                            (None, Some(snapshot), counts)
                        }
                    };
                    panda_backup::create_archive(
                        &archive_path,
                        BackupPayload {
                            preferences: prefs,
                            translator_public: translator,
                            provider_hints,
                            local_feeds,
                            database,
                            app_version: env!("CARGO_PKG_VERSION").into(),
                            record_counts,
                        },
                    )?;
                    if encrypt_local { panda_backup::encrypt_file(&archive_path, &destination, &password)?; }
                    Ok::<(), anyhow::Error>(())
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                this.settings.backup_status = Some(match result {
                    Ok(()) => "Backup exported".into(),
                    Err(error) => format!("Backup failed: {error:#}"),
                });
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn import_backup(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<ReaderWindow>,
    ) {
        let data_dir = self.data_dir.clone();
        let services = self.services.clone();
        let restore_confirmation = rfd::AsyncMessageDialog::new()
            .set_title("Confirm Panda Reader backup restore")
            .set_description("Configuration only merges Local subscriptions and preferences while keeping existing articles and reading state. A Full backup replaces the entire library after restart. Continue with restore?")
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_parent(_window);
        let password = self
            .settings
            .backup_password_input
            .read(cx)
            .value()
            .to_string();
        self.settings.backup_status = Some("Select a backup to import…".into());
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let Some(file) = rfd::AsyncFileDialog::new().add_filter("Panda Reader backup", &["prbackup", "age"]).pick_file().await else { return; };
            let source = file.path().to_path_buf();
            let encrypted = source.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("age"));
            let validated = cx.background_executor().spawn(async move {
                let scratch = tempfile::tempdir()?;
                let archive_path = if encrypted {
                    anyhow::ensure!(password.chars().count() >= 8, "enter the backup password first");
                    let archive_path = scratch.path().join("decrypted.prbackup");
                    panda_backup::decrypt_file(&source, &archive_path, &password)?;
                    archive_path
                } else { source };
                panda_backup::read_archive(&archive_path)
            }).await;
            let archive = match validated {
                Ok(archive) => archive,
                Err(error) => { let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some(format!("Backup rejected: {error:#}")); cx.notify(); }); return; }
            };
            if restore_confirmation.show().await != rfd::MessageDialogResult::Yes {
                let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some("Restore cancelled".into()); cx.notify(); });
                return;
            }
            if archive.manifest.mode == BackupMode::Full {
                let result = stage_full_restore(&data_dir, archive);
                let _ = weak.update(cx, |this, cx| {
                    this.settings.backup_status = Some(match result { Ok(()) => "Full backup validated and staged. Restart Panda Reader to complete the restore.".into(), Err(error) => format!("Full restore could not be staged: {error:#}") });
                    cx.notify();
                });
                return;
            }
            let feeds: Vec<panda_store::BackupFeed> = match archive.files.get("local-feeds.json").and_then(|bytes| serde_json::from_slice(bytes).ok()) {
                Some(feeds) => feeds,
                None => { let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some("Backup rejected: invalid Local subscriptions".into()); cx.notify(); }); return; }
            };
            let prefs: crate::app::preferences::Preferences = match archive.files.get("preferences.json").and_then(|bytes| serde_json::from_slice(bytes).ok()) {
                Some(prefs) => prefs,
                None => { let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some("Backup rejected: invalid preferences".into()); cx.notify(); }); return; }
            };
            let mut prefs = prefs;
            prefs.library_source = crate::app::preferences::LibrarySource::Local;
            prefs.last_refresh_at = None;
            if let Err(error) = (crate::services::ArticleFetchNetwork {
                mode: prefs.article_network_mode,
                proxy_url: prefs.article_proxy_url.clone(),
                trusted_fake_ip_ranges: prefs.trusted_proxy_fake_ip_ranges.clone(),
            }).validate() {
                let _ = weak.update(cx, |this, cx| { this.settings.backup_status = Some(format!("Backup rejected: invalid network preferences: {error}")); cx.notify(); });
                return;
            }
            let translator = archive.files.get("translator-public.json").and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok()).map(|value| panda_backup::safe_translator_config(&value));
            let (reply, response) = oneshot::channel();
            services.send(Command::ImportBackupFeeds { feeds, reply });
            let result = response.await.unwrap_or_else(|_| Err("Background service stopped".to_owned()));
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(count) => {
                        this.preferences = prefs;
                        this.save_preferences();
                        this.services.set_workspace("local");
                        this.services.set_article_fetch_network(crate::services::ArticleFetchNetwork {
                            mode: this.preferences.article_network_mode,
                            proxy_url: this.preferences.article_proxy_url.clone(),
                            trusted_fake_ip_ranges: this.preferences.trusted_proxy_fake_ip_ranges.clone(),
                        });
                        this.services.set_detailed_sync_logging(this.preferences.detailed_sync_logging);
                        this.services.set_log_retention_days(this.preferences.sync_log_retention_days);
                        let imported_preferences = this.preferences.clone();
                        this.update_article_reader(cx, |reader| { reader.preferences = imported_preferences; });
                        if let Some(public) = translator {
                            if let Ok(mut current) = serde_json::to_value(&this.translator_config) {
                                if let (Some(current), Some(public)) = (current.as_object_mut(), public.as_object()) {
                                    for (key, value) in public { current.insert(key.clone(), value.clone()); }
                                }
                                if let Ok(config) = serde_json::from_value(current) {
                                    this.translator_config = config;
                                    this.save_translator_config();
                                    let imported_translator = this.translator_config.clone();
                                    this.update_article_reader(cx, |reader| { reader.translator_config = imported_translator; });
                                }
                            }
                        }
                        this.settings.library_source = crate::app::preferences::LibrarySource::Local;
                        this.settings.backup_status = Some(format!("Configuration restored; merged {count} Local subscriptions. No feeds were fetched."));
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.settings.backup_status = Some(format!("Restore failed: {error}")),
                }
                cx.notify();
            });
        }).detach();
    }
}

fn stage_full_restore(
    data_dir: &std::path::Path,
    archive: panda_backup::ValidatedArchive,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !data_dir.join("restore-ready").exists(),
        "a Full restore is already staged; restart Panda Reader first"
    );
    let database = archive
        .database
        .as_deref()
        .context("Full backup database is missing")?;
    let version = archive
        .manifest
        .schema_version
        .context("Full backup schema version is missing")?;
    anyhow::ensure!(
        version <= panda_store::CURRENT_SCHEMA_VERSION,
        "backup database was created by a newer Panda Reader version"
    );
    let db_temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::copy(database, db_temp.path())?;
    panda_backup::reset_provider_sync_state(db_temp.path())?;
    let preferences = archive
        .files
        .get("preferences.json")
        .context("Full backup preferences are missing")?;
    let preferences_value: serde_json::Value = serde_json::from_slice(preferences)?;
    let mut preferences: crate::app::preferences::Preferences =
        serde_json::from_value(panda_backup::safe_preferences(&preferences_value))?;
    preferences.library_source = crate::app::preferences::LibrarySource::Local;
    preferences.last_refresh_at = None;
    crate::services::ArticleFetchNetwork {
        mode: preferences.article_network_mode,
        proxy_url: preferences.article_proxy_url.clone(),
        trusted_fake_ip_ranges: preferences.trusted_proxy_fake_ip_ranges.clone(),
    }
    .validate()?;
    let pref_temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::write(pref_temp.path(), serde_json::to_vec_pretty(&preferences)?)?;
    let translator = archive
        .files
        .get("translator-public.json")
        .context("Full backup public translator settings are missing")?;
    let translator: serde_json::Value = serde_json::from_slice(translator)?;
    let translator = panda_backup::safe_translator_config(&translator);
    let translator_temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::write(
        translator_temp.path(),
        serde_json::to_vec_pretty(&translator)?,
    )?;
    db_temp
        .persist(data_dir.join("panda-reader.sqlite3.restore-pending"))
        .map_err(|error| error.error)?;
    pref_temp
        .persist(data_dir.join("preferences.json.restore-pending"))
        .map_err(|error| error.error)?;
    translator_temp
        .persist(data_dir.join("translator-public.json.restore-pending"))
        .map_err(|error| error.error)?;
    let ready_temp = tempfile::NamedTempFile::new_in(data_dir)?;
    std::fs::write(ready_temp.path(), b"v1")?;
    ready_temp
        .persist(data_dir.join("restore-ready"))
        .map_err(|error| error.error)?;
    Ok(())
}

fn write_backup_archive(
    data_dir: &std::path::Path,
    mode: BackupMode,
    destination: &std::path::Path,
    preferences: Vec<u8>,
    translator: Vec<u8>,
    provider_hints: Vec<u8>,
    scratch: &std::path::Path,
) -> anyhow::Result<panda_backup::BackupManifest> {
    let db = data_dir.join("panda-reader.sqlite3");
    let (local_feeds, database, record_counts) = match mode {
        BackupMode::Config => {
            let store = Store::open_read_workspace(&db, "local")?;
            let feeds = store.export_local_backup_feeds()?;
            let count = feeds.len() as u64;
            (
                Some(serde_json::to_vec_pretty(&feeds)?),
                None,
                BTreeMap::from([("local_feeds".into(), count)]),
            )
        }
        BackupMode::Full => {
            let snapshot = scratch.join("library.sqlite3");
            panda_backup::create_sqlite_snapshot(&db, &snapshot)?;
            let connection = rusqlite::Connection::open_with_flags(
                &snapshot,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let mut counts = BTreeMap::new();
            for table in [
                "feeds",
                "articles",
                "article_reading_positions",
                "article_translation_segments",
            ] {
                let exists: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                    [table],
                    |row| row.get(0),
                )?;
                if exists {
                    let count: i64 = connection.query_row(
                        &format!("SELECT count(*) FROM {table}"),
                        [],
                        |row| row.get(0),
                    )?;
                    counts.insert(table.to_owned(), count as u64);
                }
            }
            (None, Some(snapshot), counts)
        }
    };
    Ok(panda_backup::create_archive(
        destination,
        BackupPayload {
            preferences,
            translator_public: translator,
            provider_hints,
            local_feeds,
            database,
            app_version: env!("CARGO_PKG_VERSION").into(),
            record_counts,
        },
    )?)
}
