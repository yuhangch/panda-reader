//! Prepare article HTML for TextView off the UI thread.

use panda_core::{Article, TranslationLayout};
use panda_translate::html::split_blocks;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyPrepKey {
    pub article_id: i64,
    pub has_extracted: bool,
    pub show_translation: bool,
    pub immersive: bool,
    pub hide_images: bool,
    /// Cheap fingerprint so extract/translate invalidates the cache.
    pub content_rev: u64,
}

impl BodyPrepKey {
    pub fn from_article(
        article: &Article,
        show_translation: bool,
        layout: TranslationLayout,
        hide_images: bool,
    ) -> Self {
        Self {
            article_id: article.summary.id,
            has_extracted: article.extracted_html.is_some(),
            show_translation,
            immersive: layout == TranslationLayout::Immersive,
            hide_images,
            content_rev: content_revision(article),
        }
    }
}

fn content_revision(article: &Article) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    article.content_html.len().hash(&mut hasher);
    article
        .extracted_html
        .as_ref()
        .map(|html| html.len())
        .unwrap_or(0)
        .hash(&mut hasher);
    article
        .translated_html
        .as_ref()
        .map(|html| html.len())
        .unwrap_or(0)
        .hash(&mut hasher);
    article
        .translation_source_hash
        .as_deref()
        .unwrap_or("")
        .hash(&mut hasher);
    hasher.finish()
}

pub fn prepare_body(
    article: &Article,
    show_translation: bool,
    layout: TranslationLayout,
    hide_images: bool,
) -> String {
    let original = article
        .extracted_html
        .as_deref()
        .unwrap_or(&article.content_html);
    let translated = article
        .translated_html
        .as_deref()
        .filter(|html| !html.trim().is_empty());
    let body = if show_translation {
        match (layout, translated) {
            (TranslationLayout::Immersive, Some(translated)) => {
                bilingual_html(original, translated)
            }
            (_, Some(translated)) => translated.to_owned(),
            (_, None) => original.to_owned(),
        }
    } else {
        original.to_owned()
    };
    let prepared = if body.trim().is_empty() {
        format!(
            "<p>This feed did not provide article content. Open the original to read it.</p><p>{}</p>",
            escape_html(&article.summary.snippet)
        )
    } else if hide_images {
        strip_images(&body)
    } else {
        body
    };
    let flat = flatten_reader_html(&prepared);
    with_paragraph_indent(&flat)
}

pub fn bilingual_html(original: &str, translated: &str) -> String {
    let originals = split_blocks(original);
    let translations = split_blocks(translated);
    if originals.is_empty() && translations.is_empty() {
        return String::new();
    }
    if originals.is_empty() {
        return translated.to_owned();
    }
    if translations.is_empty() {
        return original.to_owned();
    }

    let mut out = String::with_capacity(original.len() + translated.len() + 64);
    let pairs = originals.len().max(translations.len());
    for index in 0..pairs {
        if let Some(block) = originals.get(index) {
            out.push_str(block);
            out.push('\n');
        }
        if let Some(block) = translations.get(index) {
            out.push_str(&as_translation_follow(block));
            out.push('\n');
        }
    }
    out
}

fn as_translation_follow(block: &str) -> String {
    let inner = inner_html(block).unwrap_or(block);
    format!("<p><em>{inner}</em></p>")
}

fn inner_html(block: &str) -> Option<&str> {
    let trimmed = block.trim();
    let open_end = trimmed.find('>')?;
    let close_start = trimmed.rfind("</")?;
    if close_start <= open_end + 1 {
        return None;
    }
    Some(&trimmed[open_end + 1..close_start])
}

fn strip_images(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let rest = &html[i..];
            let lower = rest.to_ascii_lowercase();
            if lower.starts_with("<img")
                || lower.starts_with("<source")
                || lower.starts_with("<picture")
            {
                let tag_end = rest.find('>').map(|n| i + n + 1).unwrap_or(html.len());
                let self_closing = html[..tag_end].trim_end().ends_with("/>")
                    || lower.starts_with("<img")
                    || lower.starts_with("<source");
                if self_closing || lower.starts_with("<img") || lower.starts_with("<source") {
                    i = tag_end;
                    continue;
                }
                let close = if lower.starts_with("<picture") {
                    lower.find("</picture>").map(|n| i + n + "</picture>".len())
                } else {
                    None
                };
                i = close.unwrap_or(tag_end);
                continue;
            }
        }
        let ch = html[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn flatten_reader_html(html: &str) -> String {
    let tags: HashSet<&str> = [
        "p",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "ul",
        "ol",
        "li",
        "blockquote",
        "pre",
        "code",
        "table",
        "thead",
        "tbody",
        "tr",
        "th",
        "td",
        "a",
        "strong",
        "em",
        "b",
        "i",
        "br",
        "hr",
        "img",
        "figure",
        "figcaption",
        "sup",
        "sub",
        "span",
    ]
    .into_iter()
    .collect();
    let mut tag_attributes = HashMap::new();
    tag_attributes.insert("a", ["href"].into_iter().collect());
    tag_attributes.insert("img", ["src", "alt", "title"].into_iter().collect());
    ammonia::Builder::new()
        .tags(tags)
        .tag_attributes(tag_attributes)
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .clean(html)
        .to_string()
}

fn with_paragraph_indent(html: &str) -> String {
    const INDENT: &str = "\u{3000}\u{3000}";
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len() + 64);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<'
            && i + 2 < bytes.len()
            && bytes[i + 1].eq_ignore_ascii_case(&b'p')
            && matches!(bytes[i + 2], b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r')
        {
            if let Some(rel) = html[i..].find('>') {
                let end = i + rel + 1;
                out.push_str(&html[i..end]);
                let after = &html[end..];
                let indent_len = leading_paragraph_indent_len(after);
                let content = &after[indent_len..];
                let skip = content.is_empty()
                    || content.starts_with("</p")
                    || content.starts_with("<img")
                    || content.starts_with("<figure")
                    || content.starts_with("<ul")
                    || content.starts_with("<ol")
                    || content.starts_with("<blockquote")
                    || content.starts_with("<pre")
                    || content.starts_with("<table")
                    || content.starts_with("<h1")
                    || content.starts_with("<h2")
                    || content.starts_with("<h3")
                    || content.starts_with("<h4");
                if !skip {
                    out.push_str(INDENT);
                }
                i = end + indent_len;
                continue;
            }
        }
        let ch = html[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn leading_paragraph_indent_len(s: &str) -> usize {
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(n) = indent_entity_len(rest) {
            i += n;
            continue;
        }
        let ch = rest.chars().next().unwrap();
        match ch {
            ' ' | '\t' | '\n' | '\r' | '\u{00A0}' | '\u{2002}' | '\u{2003}' | '\u{3000}' => {
                i += ch.len_utf8();
            }
            _ => break,
        }
    }
    i
}

fn indent_entity_len(s: &str) -> Option<usize> {
    if !s.starts_with('&') {
        return None;
    }
    for entity in [
        "&nbsp;", "&emsp;", "&ensp;", "&#160;", "&#xA0;", "&#xa0;", "&#12288;", "&#x3000;",
        "&#X3000;",
    ] {
        if s.len() >= entity.len() && s[..entity.len()].eq_ignore_ascii_case(entity) {
            return Some(entity.len());
        }
    }
    None
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[derive(Default)]
pub struct BodyCache {
    entries: HashMap<BodyPrepKey, String>,
    order: Vec<BodyPrepKey>,
}

impl BodyCache {
    const CAPACITY: usize = 64;

    pub fn get_or_insert(
        &mut self,
        key: BodyPrepKey,
        article: &Article,
        show_translation: bool,
        layout: TranslationLayout,
        hide_images: bool,
    ) -> String {
        if let Some(body) = self.entries.get(&key) {
            return body.clone();
        }
        let body = prepare_body(article, show_translation, layout, hide_images);
        if self.order.len() >= Self::CAPACITY {
            if let Some(old) = self.order.first().cloned() {
                self.order.remove(0);
                self.entries.remove(&old);
            }
        }
        self.order.push(key);
        self.entries.insert(key, body.clone());
        body
    }

    pub fn invalidate_article(&mut self, article_id: i64) {
        self.order.retain(|key| key.article_id != article_id);
        self.entries.retain(|key, _| key.article_id != article_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_paragraphs_bilingually() {
        let html = bilingual_html("<p>Hello</p><p>World</p>", "<p>你好</p><p>世界</p>");
        assert!(html.contains("<p>Hello</p>"));
        assert!(html.contains("<p><em>你好</em></p>"));
    }

    #[test]
    fn strips_leading_spaces_before_indent() {
        let html = with_paragraph_indent("<p>  hello</p>");
        assert!(html.starts_with("<p>\u{3000}\u{3000}hello</p>"));
    }
}
