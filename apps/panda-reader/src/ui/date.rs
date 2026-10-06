use crate::app::preferences::Language;
use chrono::{DateTime, NaiveDate, NaiveDateTime};

pub(crate) fn format_published_at(value: &str, language: Language, full: bool) -> Option<String> {
    let (date_time_format, date_format) = match (language, full) {
        (Language::English, false) => ("%b %d, %I:%M %p", "%b %d"),
        (Language::English, true) => ("%B %d, %Y %I:%M %p", "%B %d, %Y"),
        (Language::ZhCn | Language::ZhTw | Language::Japanese, false) => {
            ("%m月%d日 %H:%M", "%m月%d日")
        }
        (Language::ZhCn | Language::ZhTw | Language::Japanese, true) => {
            ("%Y年%m月%d日 %H:%M", "%Y年%m月%d日")
        }
        (Language::French, false) => ("%d/%m %H:%M", "%d/%m"),
        (Language::French, true) => ("%d/%m/%Y %H:%M", "%d/%m/%Y"),
        (Language::German, false) => ("%d.%m. %H:%M", "%d.%m."),
        (Language::German, true) => ("%d.%m.%Y %H:%M", "%d.%m.%Y"),
    };
    if let Ok(date_time) = DateTime::parse_from_rfc3339(value) {
        return Some(date_time.format(date_time_format).to_string());
    }

    for format in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(date_time) = NaiveDateTime::parse_from_str(value, format) {
            return Some(date_time.format(date_time_format).to_string());
        }
    }

    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .map(|date| date.format(date_format).to_string())
}
