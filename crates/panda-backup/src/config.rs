use serde_json::{Map, Value};

const PREFERENCE_FIELDS: &[&str] = &[
    "theme",
    "sidebar_collapsed",
    "show_app_icon",
    "ui_font_size",
    "reader_font_family",
    "reader_font_size",
    "reader_line_height",
    "reader_content_width",
    "reader_paragraph_spacing",
    "remember_reading_position",
    "language",
    "translation_language",
    "paragraph_indent",
    "auto_translate_titles",
    "only_translate_future_titles",
    "auto_extract_full_text",
    "content_extractor",
    "hide_images",
    "collapsed_folders",
    "folder_order",
    "folder_icons",
    "vim_navigation",
    "translation_layout",
    "share_template",
    "check_for_updates",
    "detailed_sync_logging",
    "sync_log_retention_days",
    "article_network_mode",
    "article_proxy_url",
    "trusted_proxy_fake_ip_ranges",
];

/// Select only explicitly supported non-secret preference fields and reset the
/// current source and refresh timestamp for a portable restore.
pub fn safe_preferences(preferences: &Value) -> Value {
    let mut result = Map::new();
    for key in PREFERENCE_FIELDS {
        if let Some(value) = preferences.get(*key) {
            result.insert((*key).into(), value.clone());
        }
    }
    result.insert("library_source".into(), Value::String("local".into()));
    result.insert("last_refresh_at".into(), Value::Null);
    Value::Object(result)
}

/// Extract the supported public translator settings, excluding every key field.
pub fn safe_translator_config(config: &Value) -> Value {
    let mut result = Map::new();
    for key in [
        "provider",
        "azure_region",
        "openai_url",
        "openai_model",
        "anthropic_url",
        "anthropic_model",
        "gemini_url",
        "gemini_model",
    ] {
        if let Some(value) = config.get(key) {
            if matches!(key, "openai_url" | "anthropic_url" | "gemini_url") {
                if let Some(clean) = value.as_str().and_then(sanitize_endpoint) {
                    result.insert(key.into(), Value::String(clean));
                }
            } else {
                result.insert(key.into(), value.clone());
            }
        }
    }
    Value::Object(result)
}

/// Convert provider settings to connection hints without secrets.
pub fn safe_provider_hints(settings: &Value) -> Value {
    let mut hints = Map::new();
    if let Some(providers) = settings.as_object() {
        for (provider, value) in providers {
            let mut hint = Map::new();
            for field in ["endpoint", "username"] {
                if let Some(field_value) = value.get(field) {
                    if field == "endpoint" {
                        if let Some(clean) = field_value.as_str().and_then(sanitize_endpoint) {
                            hint.insert(field.into(), Value::String(clean));
                        }
                    } else {
                        hint.insert(field.into(), field_value.clone());
                    }
                }
            }
            if !hint.is_empty() {
                hints.insert(provider.clone(), Value::Object(hint));
            }
        }
    }
    Value::Object(hints)
}

fn sanitize_endpoint(value: &str) -> Option<String> {
    let mut url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_username("").ok()?;
    url.set_password(None).ok()?;
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string().trim_end_matches('/').to_owned())
}
