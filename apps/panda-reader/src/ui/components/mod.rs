mod clipboard_input;
mod fonts;
mod icons;

pub(in crate::ui) use clipboard_input::text_input;
pub(in crate::ui) use fonts::{app_ui_font, article_font, sidebar_font};
pub(in crate::ui) use icons::{APP_ICON, tty_icon};

pub(in crate::ui) fn preview_text(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let mut preview: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

pub(in crate::ui) fn letter_avatar(title: &str) -> String {
    title
        .chars()
        .find(|ch| ch.is_alphanumeric())
        .map(|ch| ch.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into())
}
