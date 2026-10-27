pub use crate::app::preferences::Language;

#[path = "i18n/de.rs"]
mod de;
#[path = "i18n/fr.rs"]
mod fr;
#[path = "i18n/ja.rs"]
mod ja;
#[path = "i18n/zh_cn.rs"]
mod zh_cn;
#[path = "i18n/zh_tw.rs"]
mod zh_tw;

pub fn text(language: Language, english: &'static str) -> &'static str {
    match language {
        Language::English => english,
        Language::ZhCn => lookup(zh_cn::TABLE, english).unwrap_or(english),
        Language::ZhTw => lookup(zh_tw::TABLE, english).unwrap_or(english),
        Language::Japanese => lookup(ja::TABLE, english).unwrap_or(english),
        Language::French => lookup(fr::TABLE, english).unwrap_or(english),
        Language::German => lookup(de::TABLE, english).unwrap_or(english),
    }
}

pub fn format(language: Language, english: &'static str, arg: impl std::fmt::Display) -> String {
    text(language, english).replacen("{}", &arg.to_string(), 1)
}

pub fn error(language: Language, message: String) -> String {
    let table = match language {
        Language::English => return message,
        Language::ZhCn => zh_cn::ERROR_TABLE,
        Language::ZhTw => zh_tw::ERROR_TABLE,
        Language::Japanese => ja::ERROR_TABLE,
        Language::French => fr::ERROR_TABLE,
        Language::German => de::ERROR_TABLE,
    };
    let mut translated = message;
    for (english, localized) in table {
        translated = translated.replace(english, localized);
    }
    translated
}

fn lookup(table: &'static [(&'static str, &'static str)], english: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(key, _)| *key == english)
        .map(|(_, value)| *value)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::preferences::Preferences;

    #[test]
    fn existing_preferences_default_to_english_and_language_round_trips() {
        let existing: Preferences = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(existing.language, Language::English);
        assert_eq!(existing.translation_language, Language::ZhCn);

        let mut translated = existing;
        translated.language = Language::ZhCn;
        let restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&translated).unwrap()).unwrap();
        assert_eq!(restored.language, Language::ZhCn);
        assert_eq!(text(restored.language, "Back to Reader"), "返回阅读");
        assert_eq!(text(Language::Japanese, "Settings"), "設定");
        assert_eq!(
            format(Language::French, "Synced {} articles", 12),
            "12 articles synchronisés"
        );
    }

    #[test]
    fn backup_ui_strings_are_translated_for_every_supported_language() {
        const BACKUP_TEXT_KEYS: &[&str] = &[
            "Backup",
            "Create a local backup file or restore one. Configuration-only backups merge Local subscriptions and settings into the current library. Full backups include articles and reading history and restore on the next launch.",
            "Backup type",
            "Local RSS URLs can contain access tokens. Backups without a password are not encrypted.",
            "Export backup",
            "Import backup",
            "Cloud storage",
            "Storage target",
            "Use an https:// endpoint for S3-compatible storage; leave blank for AWS S3.",
            "Use path style (MinIO / R2)",
            "Allow insecure HTTP for this endpoint",
            "Cloud credentials are saved. Leave secret fields empty to keep them.",
            "Cloud backups always require an age password. It is never saved.",
            "Test connection",
            "Save target",
            "Back up now",
            "Refresh history",
            "Full backup",
            "Configuration only",
            "{} bytes",
            "Download",
            "Restore",
            "Delete",
            "Import encrypted backup",
            "Enter a password to encrypt this backup, or leave it blank to export without encryption.",
            "Enter the password used to encrypt this backup. Encrypted backups require a password.",
            "Choose file",
            "Continue",
            "Optional: leave blank for an unencrypted backup",
            "Backup password",
            "Username",
            "Password",
            "Remote directory",
            "Region",
            "Bucket",
            "Prefix",
            "Access key",
            "Secret key",
            "Session token (optional)",
            "Configuration only (Recommended)",
            "S3 compatible",
            "Testing cloud connection…",
            "Cloud storage connection succeeded",
            "Creating encrypted cloud backup…",
            "Encrypted cloud backup uploaded",
            "Loading cloud backup history…",
            "Cloud backup history updated",
            "Downloading cloud backup…",
            "Encrypted cloud backup downloaded",
            "Downloading and validating cloud backup…",
            "Cloud restore cancelled",
            "Full backup staged. Restart Panda Reader to complete the restore.",
            "Cloud backup deleted",
            "Deleting cloud backup…",
            "Preparing backup…",
            "Export cancelled",
            "Backup exported",
            "Select a backup to import…",
            "Import cancelled",
            "Validating backup…",
            "Restore cancelled",
            "Full backup validated and staged. Restart Panda Reader to complete the restore.",
            "Cloud backup failed; check target permissions and network",
            "Cloud backup history could not be loaded",
            "Cloud backup could not be deleted",
            "Cloud target could not be loaded",
            "Cloud target could not be saved",
            "Cloud configuration restored; merged {} Local subscriptions. No feeds were fetched.",
            "Configuration restored; merged {} Local subscriptions. No feeds were fetched.",
            "Confirm Panda Reader backup restore",
            "Configuration-only backups merge Local subscriptions and preferences while keeping existing articles and reading state. Full backups replace the entire library after restart. Continue with restore?",
            "Panda Reader backup",
            "API token or API password",
            "One CIDR per line",
            "Azure Translator key",
            "Feed title",
            "Plugin folder, ZIP path, or HTTPS ZIP URL",
            "Browse folder",
            "Browse ZIP",
            "Import",
            "Reload",
            "Community plugins",
            "Loading community plugins…",
            "No community plugins found.",
            "Loading plugins…",
            "{} plugins",
            "Checking…",
            "Check for updates",
            "Requires newer app",
            "Installed",
            "Update",
            "Install",
            "HTTPS: {}",
            "Remove",
            "Panda Reader on GitHub",
            "Papr on GitHub",
            "Version {} · gpui-kit",
            "About",
            "Updates",
            "Check for stable releases and download updates in the background.",
            "Check for updates automatically",
            "Updates have not been checked yet.",
            "Checking for updates…",
            "Check now",
            "Update and restart",
            "Next launch",
            "Later",
            "Open releases",
            "Update ready",
            "Update available",
            "Update on next launch",
            "Feeds, articles and settings are stored on this device.",
            "Papr reference",
            "Papr informed our feed parsing, article extraction, and store design. Panda Reader's implementation is independent and does not include papr-core.",
            "TTY7 reference",
            "TTY7 inspired our packaging and theme design.",
            "Community plugins are up to date.",
            "{} community plugin update(s) available. Open Plugins to update.",
            "http://127.0.0.1:7890 or socks5h://127.0.0.1:1080",
            "Access Key ID",
            "Secret Access Key",
        ];
        for language in [
            Language::ZhCn,
            Language::ZhTw,
            Language::Japanese,
            Language::French,
            Language::German,
        ] {
            let table = match language {
                Language::English => unreachable!(),
                Language::ZhCn => zh_cn::TABLE,
                Language::ZhTw => zh_tw::TABLE,
                Language::Japanese => ja::TABLE,
                Language::French => fr::TABLE,
                Language::German => de::TABLE,
            };
            for key in BACKUP_TEXT_KEYS {
                assert!(
                    lookup(table, key).is_some(),
                    "missing {language:?} translation for backup UI string: {key}"
                );
            }
        }
    }

    #[test]
    fn backup_errors_translate_their_labels_and_keep_diagnostics() {
        const BACKUP_ERROR_KEYS: &[&str] = &[
            "Cloud target is invalid",
            "Cloud target could not be loaded",
            "Cloud target could not be saved",
            "Cloud storage connection failed: ",
            "Enter a cloud backup password with at least 8 characters",
            "Cloud backup failed; check target permissions and network",
            "Cloud backup history could not be loaded",
            "Cloud backup download failed: ",
            "Enter the cloud backup password first",
            "Cloud restore failed: ",
            "Full restore could not be staged: ",
            "Cloud backup has invalid Local subscriptions",
            "Cloud backup has invalid preferences",
            "Cloud backup has invalid network preferences: ",
            "Cloud configuration restore failed: ",
            "Cloud backup could not be deleted",
            "Enter a backup password with at least 8 characters",
            "Backup failed: ",
            "Backup rejected: ",
            "Backup rejected: invalid Local subscriptions",
            "Backup rejected: invalid preferences",
            "Backup rejected: invalid network preferences: ",
            "Restore failed: ",
            "Select a backup to import",
            "Use at least 8 characters, or leave the password blank",
            "Enter a password with at least 8 characters",
        ];
        for (language, prefix) in [
            (Language::ZhCn, "云端恢复失败： "),
            (Language::ZhTw, "雲端還原失敗： "),
            (
                Language::Japanese,
                "クラウドバックアップの復元に失敗しました： ",
            ),
            (Language::French, "Échec de la restauration cloud : "),
            (Language::German, "Cloud-Wiederherstellung fehlgeschlagen: "),
        ] {
            let message = error(
                language,
                "Cloud restore failed: reader got too little data".into(),
            );
            assert!(
                message.starts_with(prefix.trim_end()),
                "unexpected {language:?} cloud restore translation: {message}"
            );
            assert!(message.contains("reader got too little data"));
            for key in BACKUP_ERROR_KEYS {
                let translated = error(language, (*key).to_owned());
                assert_ne!(
                    translated, *key,
                    "missing {language:?} translation for backup error: {key}"
                );
            }
        }
    }
}
