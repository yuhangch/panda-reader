//! Pure HTML transformations for the article view; executed off the UI thread.

use panda_core::{Article, TranslationLayout};
use panda_translate::html::split_blocks;
use std::collections::{HashMap, HashSet};

pub const TRANSLATION_MARKER_START: &str = "PANDA_READER_TRANSLATION_START_7D3A";
pub const TRANSLATION_MARKER_END: &str = "PANDA_READER_TRANSLATION_END_7D3A";

pub fn prepare_body(
    article: &Article,
    show_translation: bool,
    layout: TranslationLayout,
    hide_images: bool,
    paragraph_indent: bool,
) -> String {
    let original = article
        .effective_html
        .as_deref()
        .or(article.extracted_html.as_deref())
        .or(article.source_html.as_deref())
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
            (_, Some(translated)) => repair_cached_translation(original, translated),
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
    with_paragraph_indent(&flat, paragraph_indent)
}

pub fn bilingual_html(original: &str, translated: &str) -> String {
    let originals = split_blocks(original);
    let repaired = repair_cached_translation(original, translated);
    let translations = split_blocks(&repaired);
    if originals.is_empty() && translations.is_empty() {
        return String::new();
    }
    if originals.is_empty() {
        return repaired;
    }
    if translations.is_empty() {
        return original.to_owned();
    }

    let mut out = String::with_capacity(original.len() + repaired.len() + 64);
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

fn repair_cached_translation(original: &str, translated: &str) -> String {
    let original_count = split_blocks(original).len();
    let parsed_count = split_blocks(translated).len();
    // Older Volcengine results sometimes lost their opening block tags while
    // retaining only closing tags. The translator stored one result per source
    // block, separated by newlines, so recover that positional structure here.
    if parsed_count < original_count {
        let lines = translated
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if lines.len() == original_count {
            return lines
                .into_iter()
                .map(|line| format!("<p>{line}</p>"))
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
    translated.to_owned()
}

fn as_translation_follow(block: &str) -> String {
    format!("<p>{TRANSLATION_MARKER_START}</p>{block}<p>{TRANSLATION_MARKER_END}</p>")
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

fn with_paragraph_indent(html: &str, enabled: bool) -> String {
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
                if !skip && enabled {
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

#[cfg(test)]
mod tests {
    use super::super::test_article;
    use super::*;

    #[test]
    fn pairs_paragraphs_bilingually() {
        let html = bilingual_html("<p>Hello</p><p>World</p>", "<p>你好</p><p>世界</p>");
        assert!(html.contains("<p>Hello</p>"));
        assert!(html.contains("<p>你好</p>"));
        assert!(!html.contains("<blockquote>"));
        assert!(html.contains(TRANSLATION_MARKER_START));
    }

    #[test]
    fn strips_leading_spaces_before_indent() {
        let html = with_paragraph_indent("<p>  hello</p>", true);
        assert!(html.starts_with("<p>\u{3000}\u{3000}hello</p>"));
    }

    #[test]
    fn reading_modes_keep_the_expected_original_and_translation() {
        let article = test_article();
        let original = prepare_body(&article, false, TranslationLayout::Immersive, false, true);
        assert!(original.contains("Original"));
        assert!(!original.contains("译文"));
        assert!(original.contains("<img"));

        let bilingual = prepare_body(&article, true, TranslationLayout::Immersive, true, true);
        assert!(bilingual.contains("Original"));
        assert!(bilingual.contains("<p>译文</p>"));
        assert!(!bilingual.contains("<blockquote>"));
        assert!(!bilingual.contains("<img"));

        let replaced = prepare_body(&article, true, TranslationLayout::Replaced, false, true);
        assert!(!replaced.contains("Original"));
        assert!(replaced.contains("译文"));
    }

    #[test]
    fn extracted_content_and_missing_translation_use_the_original() {
        let mut article = test_article();
        article.extracted_html = Some("<p>Full article</p>".into());
        article.translated_html = None;
        let prepared = prepare_body(&article, true, TranslationLayout::Replaced, false, true);
        assert!(prepared.contains("Full article"));
        assert!(!prepared.contains("Original"));
    }

    #[test]
    fn empty_content_escapes_the_preview_and_unsafe_html_is_removed() {
        let mut article = test_article();
        article.content_html.clear();
        let empty = prepare_body(&article, false, TranslationLayout::Immersive, false, true);
        assert!(empty.contains("&lt;example&gt; &amp; preview"));

        article.content_html = "<p onclick=\"evil()\">Read me</p><script>evil()</script>".into();
        let prepared = prepare_body(&article, false, TranslationLayout::Immersive, false, true);
        assert!(prepared.contains("Read me"));
        assert!(!prepared.contains("onclick"));
        assert!(!prepared.contains("script"));
    }
}
