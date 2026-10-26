use super::Settings;
use crate::services::{Command, redact_diagnostic_message, write_sync_summary_log};
use crate::ui::window::ReaderWindow;
use crate::ui::{components::text_input, i18n};
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
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(owner.t("Create a local backup file or restore one. Configuration-only backups merge Local subscriptions and settings into the current library. Full backups include articles and reading history and restore on the next launch.")))
            .child(div().pb_1().text_sm().child(owner.t("Backup type")))
            .child(div().w(px(320.)).child(Select::new(&self.backup_mode_select).small().placeholder(owner.t("Backup type"))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(owner.t("Local RSS URLs can contain access tokens. Backups without a password are not encrypted.")))
            .child(h_flex().gap_2().pt_2()
                .child(Button::new("backup-export").small().primary().label(owner.t("Export backup")).on_click(cx.listener(|this, _, window, cx| this.export_backup(window, cx))))
                .child(Button::new("backup-import").small().secondary().label(owner.t("Import backup")).on_click(cx.listener(|this, _, window, cx| this.import_backup(window, cx)))))
            .child(div().my_4().border_t_1().border_color(cx.theme().border))
            .child(div().font_semibold().child(owner.t("Cloud storage")))
            .child(div().pb_1().text_sm().child(owner.t("Storage target")))
            .child(div().w(px(320.)).child(Select::new(&self.backup_target_select).small().placeholder(owner.t("Storage target"))))
            .when(self.backup_cloud_s3, |view| {
                view.child(text_input(&self.backup_s3_endpoint_input))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(owner.t("Use an https:// endpoint for S3-compatible storage; leave blank for AWS S3.")),
                    )
            })
            .when(!self.backup_cloud_s3, |view| {
                view.child(text_input(&self.backup_webdav_endpoint_input))
            })
            .when(!self.backup_cloud_s3, |view| view.child(text_input(&self.backup_cloud_username_input)).child(text_input(&self.backup_cloud_password_input).mask_toggle()).child(text_input(&self.backup_cloud_directory_input)))
            .when(self.backup_cloud_s3, |view| {
                view.child(text_input(&self.backup_cloud_region_input))
                    .child(text_input(&self.backup_cloud_bucket_input))
                    .child(text_input(&self.backup_cloud_prefix_input))
                    .child(text_input(&self.backup_cloud_access_input).mask_toggle())
                    .child(text_input(&self.backup_cloud_secret_input).mask_toggle())
                    .child(text_input(&self.backup_cloud_session_input).mask_toggle())
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .py_1()
                            .child(div().text_sm().child(owner.t("Use path style (MinIO / R2)")))
                            .child(
                                Switch::new("backup-cloud-path-style")
                                    .checked(self.backup_cloud_path_style)
                                    .on_change(cx.listener(|this, checked, _, cx| {
                                        this.settings.backup_cloud_path_style = *checked;
                                        cx.notify();
                                    })),
                            ),
                    )
            })
            .child(h_flex().items_center().justify_between().py_1().child(div().text_sm().child(owner.t("Allow insecure HTTP for this endpoint"))).child(Switch::new("backup-cloud-insecure-http").checked(self.backup_cloud_allow_insecure_http).on_change(cx.listener(|this, checked, _, cx| {
                this.settings.backup_cloud_allow_insecure_http = *checked;
                if this.settings.backup_cloud_s3 {
                    this.settings.backup_s3_insecure_http = *checked;
                } else {
                    this.settings.backup_webdav_insecure_http = *checked;
                }
                cx.notify();
            }))))
            .when(self.backup_cloud_credentials_saved, |view| view.child(div().text_xs().text_color(cx.theme().muted_foreground).child(owner.t("Cloud credentials are saved. Leave secret fields empty to keep them."))))
            .child(h_flex().gap_2().pt_2().child(Button::new("backup-cloud-test").small().secondary().label(owner.t("Test connection")).on_click(cx.listener(|this, _, window, cx| this.test_backup_target(window, cx)))).child(Button::new("backup-cloud-save").small().secondary().label(owner.t("Save target")).on_click(cx.listener(|this, _, window, cx| this.save_backup_target(window, cx)))))
            .child(div().pt_2().text_sm().child(owner.t("Cloud backups always require an age password. It is never saved.")))
            .child(text_input(&self.backup_password_input).mask_toggle())
            .child(h_flex().gap_2().pt_2().child(Button::new("backup-cloud-now").small().primary().label(owner.t("Back up now")).on_click(cx.listener(|this, _, window, cx| this.backup_to_cloud(window, cx)))).child(Button::new("backup-cloud-history").small().secondary().label(owner.t("Refresh history")).on_click(cx.listener(|this, _, window, cx| this.refresh_cloud_history(window, cx)))))
            .child(v_flex().gap_2().pt_2().children(self.backup_cloud_items.iter().cloned().map(|item| {
                let mode = owner.t(if item.mode == "full" { "Full backup" } else { "Configuration only" });
                let size = i18n::format(owner.preferences.language, "{} bytes", item.encrypted_bytes);
                let label = format!("{} · {} · {}", item.created_at, mode, size);
                let download_item = item.clone();
                let restore_item = item.clone();
                h_flex().items_center().justify_between().child(div().text_xs().child(label)).child(h_flex().gap_1().child(Button::new(format!("backup-download-{}", item.archive_name)).small().secondary().label(owner.t("Download")).on_click(cx.listener(move |this, _, window, cx| this.download_cloud_backup(download_item.clone(), window, cx)))).child(Button::new(format!("backup-restore-{}", item.archive_name)).small().secondary().label(owner.t("Restore")).on_click(cx.listener(move |this, _, window, cx| this.restore_cloud_backup(restore_item.clone(), window, cx)))).child(Button::new(format!("backup-delete-{}", item.archive_name)).small().secondary().label(owner.t("Delete")).on_click(cx.listener(move |this, _, window, cx| this.delete_cloud_backup(item.clone(), window, cx)))))
            })))
    }

    pub(in crate::ui) fn render_backup_password_prompt(
        &self,
        owner: &ReaderWindow,
        cx: &mut Context<ReaderWindow>,
    ) -> impl IntoElement {
        let for_export = self.backup_password_prompt_for_export;
        let title = if for_export {
            "Export backup"
        } else {
            "Import encrypted backup"
        };
        let description = if for_export {
            "Enter a password to encrypt this backup, or leave it blank to export without encryption."
        } else {
            "Enter the password used to encrypt this backup. Encrypted backups require a password."
        };
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(cx.theme().background.opacity(0.55))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.cancel_backup_password_prompt(cx);
                }),
            )
            .child(
                v_flex()
                    .w(px(440.))
                    .gap_3()
                    .p_5()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().popover)
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(div().text_lg().font_semibold().child(owner.t(title)))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(owner.t(description)),
                    )
                    .child(text_input(&self.backup_password_prompt_input).mask_toggle())
                    .when_some(self.backup_password_prompt_error.clone(), |view, error| {
                        view.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().danger)
                                .child(i18n::error(owner.preferences.language, error)),
                        )
                    })
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("backup-password-cancel")
                                    .small()
                                    .secondary()
                                    .label(owner.t("Cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_backup_password_prompt(cx);
                                    })),
                            )
                            .child(
                                Button::new("backup-password-continue")
                                    .small()
                                    .primary()
                                    .label(if for_export {
                                        owner.t("Choose file")
                                    } else {
                                        owner.t("Continue")
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_backup_password_prompt(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}

impl ReaderWindow {
    fn set_backup_busy(&mut self, message: &'static str, cx: &mut Context<Self>) {
        self.set_busy(self.t(message));
        cx.notify();
    }

    fn set_backup_flash(&mut self, message: &'static str, cx: &mut Context<Self>) {
        self.set_flash(self.t(message), cx);
        cx.notify();
    }

    fn set_backup_flash_text(&mut self, message: String, cx: &mut Context<Self>) {
        self.set_flash(message, cx);
        cx.notify();
    }

    fn set_backup_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.set_error(message.into());
        cx.notify();
    }

    fn build_backup_target(&self, cx: &App) -> anyhow::Result<panda_backup::BackupTargetConfig> {
        let existing = panda_backup::load_targets(&self.data_dir.join("backup-targets.json"))?;
        let previous_webdav = existing.iter().find_map(|target| match target {
            panda_backup::BackupTargetConfig::WebDav(target) => Some(target.clone()),
            panda_backup::BackupTargetConfig::S3(_) => None,
        });
        let previous_s3 = existing.iter().find_map(|target| match target {
            panda_backup::BackupTargetConfig::S3(target) => Some(target.clone()),
            panda_backup::BackupTargetConfig::WebDav(_) => None,
        });
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
        if self.settings.backup_cloud_s3 {
            let access = value(&self.settings.backup_cloud_access_input);
            let secret = value(&self.settings.backup_cloud_secret_input);
            let token = value(&self.settings.backup_cloud_session_input);
            let endpoint = value(&self.settings.backup_s3_endpoint_input);
            Ok(panda_backup::BackupTargetConfig::S3(
                panda_backup::S3Target {
                    endpoint: (!endpoint.is_empty()).then_some(endpoint),
                    region: value(&self.settings.backup_cloud_region_input),
                    bucket: value(&self.settings.backup_cloud_bucket_input),
                    prefix: value(&self.settings.backup_cloud_prefix_input),
                    access_key: if access.is_empty() {
                        previous_s3
                            .as_ref()
                            .map(|old| old.access_key.clone())
                            .unwrap_or_default()
                    } else {
                        access
                    },
                    secret_key: if secret.is_empty() {
                        previous_s3
                            .as_ref()
                            .map(|old| old.secret_key.clone())
                            .unwrap_or_default()
                    } else {
                        secret
                    },
                    session_token: if token.is_empty() {
                        previous_s3.and_then(|old| old.session_token)
                    } else {
                        Some(token)
                    },
                    path_style: self.settings.backup_cloud_path_style,
                    allow_insecure_http: self.settings.backup_cloud_allow_insecure_http,
                },
            ))
        } else {
            let password = value(&self.settings.backup_cloud_password_input);
            Ok(panda_backup::BackupTargetConfig::WebDav(
                panda_backup::WebDavTarget {
                    endpoint: value(&self.settings.backup_webdav_endpoint_input),
                    username: value(&self.settings.backup_cloud_username_input),
                    password: if password.is_empty() {
                        previous_webdav.map(|old| old.password).unwrap_or_default()
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
        let target = match self.build_backup_target(cx) {
            Ok(target) => target,
            Err(_) => {
                self.set_backup_error("Cloud target is invalid", cx);
                return;
            }
        };
        let credentials_saved = match &target {
            panda_backup::BackupTargetConfig::WebDav(target) => {
                (!target.password.is_empty(), false, false, false)
            }
            panda_backup::BackupTargetConfig::S3(target) => (
                false,
                !target.access_key.is_empty(),
                !target.secret_key.is_empty(),
                target
                    .session_token
                    .as_ref()
                    .is_some_and(|token| !token.is_empty()),
            ),
        };
        let mut targets =
            match panda_backup::load_targets(&self.data_dir.join("backup-targets.json")) {
                Ok(targets) => targets,
                Err(_) => {
                    self.set_backup_error("Cloud target could not be loaded", cx);
                    return;
                }
            };
        let replace_s3 = self.settings.backup_cloud_s3;
        targets.retain(|saved| {
            !matches!(
                (replace_s3, saved),
                (true, panda_backup::BackupTargetConfig::S3(_))
                    | (false, panda_backup::BackupTargetConfig::WebDav(_))
            )
        });
        targets.push(target);
        match panda_backup::save_targets(&self.data_dir.join("backup-targets.json"), &targets) {
            Ok(()) => {
                let (password_saved, access_saved, secret_saved, session_saved) = credentials_saved;
                self.settings.backup_cloud_credentials_saved = if replace_s3 {
                    self.settings.backup_cloud_access_saved = access_saved;
                    self.settings.backup_cloud_secret_saved = secret_saved;
                    self.settings.backup_cloud_session_saved = session_saved;
                    access_saved || secret_saved || session_saved
                } else {
                    self.settings.backup_cloud_password_saved = password_saved;
                    password_saved
                };
                if replace_s3 {
                    if let panda_backup::BackupTargetConfig::S3(saved) = &targets[targets.len() - 1]
                    {
                        self.settings.backup_s3_insecure_http = saved.allow_insecure_http;
                    }
                } else if let panda_backup::BackupTargetConfig::WebDav(saved) =
                    &targets[targets.len() - 1]
                {
                    self.settings.backup_webdav_insecure_http = saved.allow_insecure_http;
                }
                let secret_inputs = if replace_s3 {
                    vec![
                        (
                            &self.settings.backup_cloud_access_input,
                            access_saved,
                            "Access key",
                        ),
                        (
                            &self.settings.backup_cloud_secret_input,
                            secret_saved,
                            "Secret key",
                        ),
                        (
                            &self.settings.backup_cloud_session_input,
                            session_saved,
                            "Session token (optional)",
                        ),
                    ]
                } else {
                    vec![(
                        &self.settings.backup_cloud_password_input,
                        password_saved,
                        "Password",
                    )]
                };
                let language = self.preferences.language;
                for (input, saved, placeholder) in secret_inputs {
                    let placeholder = if saved {
                        "••••••••".to_owned()
                    } else {
                        i18n::text(language, placeholder).to_owned()
                    };
                    input.update(cx, |state, cx| {
                        state.set_value("", window, cx);
                        state.set_placeholder(placeholder, window, cx);
                    });
                }
                self.set_backup_flash("Cloud backup target saved", cx);
            }
            Err(_) => self.set_backup_error("Cloud target could not be saved", cx),
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
                self.set_backup_error("Cloud target is invalid", cx);
                return;
            }
        };
        self.set_backup_busy("Testing cloud connection…", cx);
        let diagnostics_path = self.data_dir.join("panda-reader.sqlite3");
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
                            .test_connection()
                            .await
                    })
                })
                .await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.set_backup_flash("Cloud storage connection succeeded", cx),
                    Err(error) => {
                        let detail = redact_diagnostic_message(&format!("{error:#}"));
                        write_sync_summary_log(
                            &diagnostics_path,
                            "error",
                            format!("cloud backup connection test failed: {detail}"),
                        );
                        this.set_backup_error(
                            format!("Cloud storage connection failed: {detail}"),
                            cx,
                        )
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn backup_to_cloud(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let target = match self.build_backup_target(cx) {
            Ok(target) => target,
            Err(_) => {
                self.set_backup_error("Cloud target is invalid", cx);
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
            self.set_backup_error(
                "Enter a cloud backup password with at least 8 characters",
                cx,
            );
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
        self.set_backup_busy("Creating encrypted cloud backup…", cx);
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let scratch = tempfile::tempdir()?;
                    let archive_path = scratch.path().join("unencrypted.prbackup");
                    let manifest = write_backup_archive(
                        &data_dir,
                        mode,
                        &archive_path,
                        preferences,
                        translator,
                        hints,
                        scratch.path(),
                    )?;
                    let encrypted_name = panda_backup::suggested_filename(mode);
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
                        this.settings.backup_cloud_items.insert(0, item);
                        this.set_backup_flash("Encrypted cloud backup uploaded", cx);
                    }
                    Err(_) => this.set_backup_error(
                        "Cloud backup failed; check target permissions and network",
                        cx,
                    ),
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
                self.set_backup_error("Cloud target is invalid", cx);
                return;
            }
        };
        self.set_backup_busy("Loading cloud backup history…", cx);
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
                        this.set_backup_flash("Cloud backup history updated", cx);
                    }
                    Err(_) => this.set_backup_error("Cloud backup history could not be loaded", cx),
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
                self.set_backup_error("Cloud target is invalid", cx);
                return;
            }
        };
        let archive_name = item.archive_name.clone();
        let diagnostics_path = self.data_dir.join("panda-reader.sqlite3");
        let language = self.preferences.language;
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_file_name(&item.archive_name)
                .add_filter(
                    i18n::text(language, "Panda Reader backup"),
                    &["pdbk", "age"],
                )
                .save_file()
                .await
            else {
                return;
            };
            let destination = file.path().to_path_buf();
            let _ = weak.update(cx, |this, cx| {
                this.set_backup_busy("Downloading cloud backup…", cx);
            });
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
                match result {
                    Ok(()) => this.set_backup_flash("Encrypted cloud backup downloaded", cx),
                    Err(error) => {
                        let detail = redact_diagnostic_message(&format!("{error:#}"));
                        write_sync_summary_log(
                            &diagnostics_path,
                            "error",
                            format!(
                                "cloud backup download failed archive={archive_name}: {detail}"
                            ),
                        );
                        this.set_backup_error(
                            format!("Cloud backup download failed: {detail}"),
                            cx,
                        );
                    }
                }
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
                self.set_backup_error("Cloud target is invalid", cx);
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
            self.set_backup_error("Enter the cloud backup password first", cx);
            return;
        }
        let data_dir = self.data_dir.clone();
        let services = self.services.clone();
        let diagnostics_path = self.data_dir.join("panda-reader.sqlite3");
        let restore_confirmation = restore_confirmation(_window, self.preferences.language);
        self.set_backup_busy("Downloading and validating cloud backup…", cx);
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
            let archive = match loaded {
                Ok(archive) => archive,
                Err(error) => {
                    let detail = redact_diagnostic_message(&format!("{error:#}"));
                    write_sync_summary_log(
                        &diagnostics_path,
                        "error",
                        format!("cloud backup restore validation failed: {detail}"),
                    );
                    let _ = weak.update(cx, |this, cx| {
                        this.set_backup_error(format!("Cloud restore failed: {detail}"), cx);
                    });
                    return;
                }
            };
            if restore_confirmation.show().await != rfd::MessageDialogResult::Yes {
                let _ = weak.update(cx, |this, cx| {
                    this.set_backup_flash("Cloud restore cancelled", cx);
                });
                return;
            }
            if archive.manifest.mode == BackupMode::Full {
                let result = stage_full_restore(&data_dir, archive);
                let _ = weak.update(cx, |this, cx| match result {
                    Ok(()) => this.set_backup_flash(
                        "Full backup staged. Restart Panda Reader to complete the restore.",
                        cx,
                    ),
                    Err(error) => this.set_backup_error(
                        format!("Full restore could not be staged: {error:#}"),
                        cx,
                    ),
                });
                return;
            }
            let feeds: Vec<panda_store::BackupFeed> = match archive
                .files
                .get("local-feeds.json")
                .and_then(|bytes| serde_json::from_slice(bytes).ok())
            {
                Some(feeds) => feeds,
                None => {
                    let _ = weak.update(cx, |this, cx| {
                        this.set_backup_error("Cloud backup has invalid Local subscriptions", cx);
                    });
                    return;
                }
            };
            let mut prefs: crate::app::preferences::Preferences = match archive
                .files
                .get("preferences.json")
                .and_then(|bytes| serde_json::from_slice(bytes).ok())
            {
                Some(prefs) => prefs,
                None => {
                    let _ = weak.update(cx, |this, cx| {
                        this.set_backup_error("Cloud backup has invalid preferences", cx);
                    });
                    return;
                }
            };
            prefs.library_source = crate::app::preferences::LibrarySource::Local;
            prefs.last_refresh_at = None;
            if let Err(error) = (crate::services::ArticleFetchNetwork {
                mode: prefs.article_network_mode,
                proxy_url: prefs.article_proxy_url.clone(),
                trusted_fake_ip_ranges: prefs.trusted_proxy_fake_ip_ranges.clone(),
            }).validate() {
                let detail = redact_diagnostic_message(&error.to_string());
                let _ = weak.update(cx, |this, cx| {
                    this.set_backup_error(
                        format!("Cloud backup has invalid network preferences: {detail}"),
                        cx,
                    );
                });
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
                        this.set_backup_flash_text(
                            i18n::format(
                                this.preferences.language,
                                "Cloud configuration restored; merged {} Local subscriptions. No feeds were fetched.",
                                count,
                            ),
                            cx,
                        );
                        this.load_snapshot(cx);
                    }
                    Err(error) => this.set_backup_error(
                        format!("Cloud configuration restore failed: {error}"),
                        cx,
                    ),
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
                self.set_backup_error("Cloud target is invalid", cx);
                return;
            }
        };
        let archive_name = item.archive_name.clone();
        self.set_backup_busy("Deleting cloud backup…", cx);
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
                        this.set_backup_flash("Cloud backup deleted", cx);
                    }
                    Err(_) => this.set_backup_error("Cloud backup could not be deleted", cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn export_backup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<ReaderWindow>,
    ) {
        self.settings.backup_password_prompt_for_export = true;
        self.settings.backup_password_prompt_open = true;
        self.settings.backup_password_prompt_error = None;
        self.settings.backup_import_pending_path = None;
        let language = self.preferences.language;
        let _ = self
            .settings
            .backup_password_prompt_input
            .update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.set_placeholder(
                    i18n::text(language, "Optional: leave blank for an unencrypted backup"),
                    window,
                    cx,
                );
            });
        cx.notify();
    }

    fn export_backup_with_password(&mut self, password: String, cx: &mut Context<ReaderWindow>) {
        let mode = if self.settings.backup_full_mode {
            BackupMode::Full
        } else {
            BackupMode::Config
        };
        let encrypt_local = !password.is_empty();
        if encrypt_local && password.chars().count() < 8 {
            self.set_backup_error("Enter a backup password with at least 8 characters", cx);
            return;
        }
        let data_dir = self.data_dir.clone();
        let language = self.preferences.language;
        let prefs = match serde_json::to_value(&self.preferences) {
            Ok(value) => panda_backup::safe_preferences(&value),
            Err(error) => {
                self.set_backup_error(error.to_string(), cx);
                return;
            }
        };
        let translator = match serde_json::to_value(&self.translator_config) {
            Ok(value) => panda_backup::safe_translator_config(&value),
            Err(error) => {
                self.set_backup_error(error.to_string(), cx);
                return;
            }
        };
        let provider_hints = match serde_json::to_value(&self.settings.provider_settings) {
            Ok(value) => panda_backup::safe_provider_hints(&value),
            Err(error) => {
                self.set_backup_error(error.to_string(), cx);
                return;
            }
        };
        let prefs = serde_json::to_vec_pretty(&prefs).unwrap_or_default();
        let translator = serde_json::to_vec_pretty(&translator).unwrap_or_default();
        let provider_hints = serde_json::to_vec_pretty(&provider_hints).unwrap_or_default();
        self.set_backup_busy("Preparing backup…", cx);
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let suggested = panda_backup::suggested_filename(mode);
            let dialog = rfd::AsyncFileDialog::new().set_file_name(&suggested);
            let dialog = dialog.add_filter(i18n::text(language, "Panda Reader backup"), &["pdbk"]);
            let Some(file) = dialog.save_file().await
            else {
                let _ = weak.update(cx, |this, cx| {
                    this.set_backup_flash("Export cancelled", cx);
                });
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
                match result {
                    Ok(()) => this.set_backup_flash("Backup exported", cx),
                    Err(error) => this.set_backup_error(format!("Backup failed: {error:#}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::ui) fn import_backup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<ReaderWindow>,
    ) {
        self.settings.backup_import_pending_path = None;
        self.settings.backup_password_prompt_open = false;
        self.settings.backup_password_prompt_for_export = false;
        self.settings.backup_password_prompt_error = None;
        let language = self.preferences.language;
        let _ = self
            .settings
            .backup_password_prompt_input
            .update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.set_placeholder(i18n::text(language, "Backup password"), window, cx);
            });
        self.set_backup_busy("Select a backup to import…", cx);
        let restore_confirmation = restore_confirmation(window, self.preferences.language);
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let Some(file) = rfd::AsyncFileDialog::new()
                .add_filter(
                    i18n::text(language, "Panda Reader backup"),
                    &["pdbk", "prbackup", "age"],
                )
                .pick_file()
                .await
            else {
                let _ = weak.update(cx, |this, cx| {
                    this.set_backup_flash("Import cancelled", cx);
                });
                return;
            };
            let source = file.path().to_path_buf();
            let encrypted = match panda_backup::is_age_encrypted_file(&source) {
                Ok(encrypted) => encrypted,
                Err(error) => {
                    let _ = weak.update(cx, |this, cx| {
                        this.set_backup_error(format!("Backup rejected: {error:#}"), cx);
                    });
                    return;
                }
            };
            if encrypted {
                let _ = weak.update(cx, |this, cx| {
                    this.clear_status();
                    this.settings.backup_import_pending_path = Some(source);
                    this.settings.backup_password_prompt_error = None;
                    this.settings.backup_password_prompt_for_export = false;
                    this.settings.backup_password_prompt_open = true;
                    cx.notify();
                });
                return;
            }
            let _ = weak.update(cx, |this, cx| {
                this.import_backup_file(source, String::new(), restore_confirmation, cx);
            });
        })
        .detach();
    }

    pub(in crate::ui) fn cancel_backup_password_prompt(&mut self, cx: &mut Context<ReaderWindow>) {
        self.settings.backup_import_pending_path = None;
        self.settings.backup_password_prompt_open = false;
        self.settings.backup_password_prompt_error = None;
        self.set_backup_flash(
            if self.settings.backup_password_prompt_for_export {
                "Export cancelled"
            } else {
                "Import cancelled"
            },
            cx,
        );
    }

    pub(in crate::ui) fn confirm_backup_password_prompt(
        &mut self,
        window: &mut Window,
        cx: &mut Context<ReaderWindow>,
    ) {
        let mut password = self
            .settings
            .backup_password_prompt_input
            .read(cx)
            .value()
            .to_string();
        let for_export = self.settings.backup_password_prompt_for_export;
        if for_export && password.trim().is_empty() {
            password.clear();
        }
        if (for_export && !password.is_empty() && password.chars().count() < 8)
            || (!for_export && password.chars().count() < 8)
        {
            self.settings.backup_password_prompt_error = Some(if for_export {
                "Use at least 8 characters, or leave the password blank".into()
            } else {
                "Enter a password with at least 8 characters".into()
            });
            cx.notify();
            return;
        }
        self.settings.backup_password_prompt_open = false;
        self.settings.backup_password_prompt_error = None;
        if for_export {
            self.export_backup_with_password(password, cx);
            return;
        }
        let Some(source) = self.settings.backup_import_pending_path.take() else {
            self.set_backup_error("Select a backup to import", cx);
            return;
        };
        self.import_backup_file(
            source,
            password,
            restore_confirmation(window, self.preferences.language),
            cx,
        );
    }

    fn import_backup_file(
        &mut self,
        source: std::path::PathBuf,
        password: String,
        restore_confirmation: rfd::AsyncMessageDialog,
        cx: &mut Context<ReaderWindow>,
    ) {
        let data_dir = self.data_dir.clone();
        let services = self.services.clone();
        self.set_backup_busy("Validating backup…", cx);
        let weak = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let validated = cx.background_executor().spawn(async move {
                let scratch = tempfile::tempdir()?;
                let encrypted = panda_backup::is_age_encrypted_file(&source)?;
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
                Err(error) => {
                    let detail = redact_diagnostic_message(&format!("{error:#}"));
                    let _ = weak.update(cx, |this, cx| {
                        this.set_backup_error(format!("Backup rejected: {detail}"), cx);
                    });
                    return;
                }
            };
            if restore_confirmation.show().await != rfd::MessageDialogResult::Yes {
                let _ = weak.update(cx, |this, cx| {
                    this.set_backup_flash("Restore cancelled", cx);
                });
                return;
            }
            if archive.manifest.mode == BackupMode::Full {
                let result = stage_full_restore(&data_dir, archive);
                let _ = weak.update(cx, |this, cx| {
                    match result {
                        Ok(()) => this.set_backup_flash(
                            "Full backup validated and staged. Restart Panda Reader to complete the restore.",
                            cx,
                        ),
                        Err(error) => this.set_backup_error(
                            format!("Full restore could not be staged: {error:#}"),
                            cx,
                        ),
                    }
                });
                return;
            }
            let feeds: Vec<panda_store::BackupFeed> = match archive.files.get("local-feeds.json").and_then(|bytes| serde_json::from_slice(bytes).ok()) {
                Some(feeds) => feeds,
                None => {
                    let _ = weak.update(cx, |this, cx| {
                        this.set_backup_error("Backup rejected: invalid Local subscriptions", cx);
                    });
                    return;
                }
            };
            let prefs: crate::app::preferences::Preferences = match archive.files.get("preferences.json").and_then(|bytes| serde_json::from_slice(bytes).ok()) {
                Some(prefs) => prefs,
                None => {
                    let _ = weak.update(cx, |this, cx| {
                        this.set_backup_error("Backup rejected: invalid preferences", cx);
                    });
                    return;
                }
            };
            let mut prefs = prefs;
            prefs.library_source = crate::app::preferences::LibrarySource::Local;
            prefs.last_refresh_at = None;
            if let Err(error) = (crate::services::ArticleFetchNetwork {
                mode: prefs.article_network_mode,
                proxy_url: prefs.article_proxy_url.clone(),
                trusted_fake_ip_ranges: prefs.trusted_proxy_fake_ip_ranges.clone(),
            }).validate() {
                let detail = redact_diagnostic_message(&error.to_string());
                let _ = weak.update(cx, |this, cx| {
                    this.set_backup_error(
                        format!("Backup rejected: invalid network preferences: {detail}"),
                        cx,
                    );
                });
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
                        this.set_backup_flash_text(
                            i18n::format(
                                this.preferences.language,
                                "Configuration restored; merged {} Local subscriptions. No feeds were fetched.",
                                count,
                            ),
                            cx,
                        );
                        this.load_snapshot(cx);
                    }
                    Err(error) => this
                        .set_backup_error(format!("Restore failed: {error}"), cx),
                }
                cx.notify();
            });
        }).detach();
    }
}

fn restore_confirmation(window: &mut Window, language: i18n::Language) -> rfd::AsyncMessageDialog {
    rfd::AsyncMessageDialog::new()
        .set_title(i18n::text(language, "Confirm Panda Reader backup restore"))
        .set_description(i18n::text(language, "Configuration-only backups merge Local subscriptions and preferences while keeping existing articles and reading state. Full backups replace the entire library after restart. Continue with restore?"))
        .set_level(rfd::MessageLevel::Warning)
        .set_buttons(rfd::MessageButtons::YesNo)
        .set_parent(window)
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
