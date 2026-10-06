//! Bounded cache for prepared article bodies.

use super::content::{TRANSLATION_MARKER_END, TRANSLATION_MARKER_START, prepare_body};
use panda_core::{Article, TranslationLayout};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyPrepKey {
    pub article_id: i64,
    pub has_extracted: bool,
    pub show_translation: bool,
    pub immersive: bool,
    pub hide_images: bool,
    pub paragraph_indent: bool,
    /// Cheap fingerprint so extract/translate invalidates the cache.
    pub content_rev: [u8; 32],
}

impl BodyPrepKey {
    pub fn from_article(
        article: &Article,
        show_translation: bool,
        layout: TranslationLayout,
        hide_images: bool,
        paragraph_indent: bool,
    ) -> Self {
        Self {
            article_id: article.summary.id,
            has_extracted: article.extracted_html.is_some(),
            show_translation,
            immersive: layout == TranslationLayout::Immersive,
            hide_images,
            paragraph_indent,
            content_rev: content_revision(article),
        }
    }
}

fn content_revision(article: &Article) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for value in [
        article.content_html.as_str(),
        article.source_html.as_deref().unwrap_or_default(),
        article.source_page_html.as_deref().unwrap_or_default(),
        article.extracted_html.as_deref().unwrap_or_default(),
        article.effective_html.as_deref().unwrap_or_default(),
        article.translated_html.as_deref().unwrap_or_default(),
        article
            .translation_source_hash
            .as_deref()
            .unwrap_or_default(),
    ] {
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.finalize().into()
}

#[derive(Default)]
pub struct BodyCache {
    entries: HashMap<BodyPrepKey, PreparedBody>,
    order: Vec<BodyPrepKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedBody {
    pub html: String,
    pub markdown: String,
    pub image_urls: Vec<String>,
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
        paragraph_indent: bool,
    ) -> PreparedBody {
        if let Some(body) = self.entries.get(&key) {
            return body.clone();
        }
        let html = prepare_body(
            article,
            show_translation,
            layout,
            hide_images,
            paragraph_indent,
        );
        // TextView's HTML reader does not treat <pre> as a code block. Convert
        // the sanitized body off the UI thread so fenced code keeps its
        // whitespace, monospace styling, and block layout.
        let markdown_source = preserve_figure_boundaries(&html);
        let markdown = quick_html2md::html_to_markdown(&markdown_source);
        let markdown = wrap_translation_markdown(&markdown);
        let (markdown, image_urls) = add_image_view_links(&markdown_source, markdown);
        let markdown = if paragraph_indent {
            indent_markdown_paragraphs(&markdown)
        } else {
            markdown
        };
        let body = PreparedBody {
            html,
            markdown,
            image_urls,
        };
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

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

fn wrap_translation_markdown(markdown: &str) -> String {
    const FENCE: &str = "````````````panda-translation";
    let mut output = String::with_capacity(markdown.len());
    for line in markdown.lines() {
        match line.trim() {
            TRANSLATION_MARKER_START => {
                while output.ends_with('\n') && !output.ends_with("\n\n") {
                    output.push('\n');
                }
                if !output.is_empty() && !output.ends_with("\n\n") {
                    output.push_str("\n\n");
                }
                output.push_str(FENCE);
                output.push('\n');
            }
            TRANSLATION_MARKER_END => {
                while output.ends_with('\n') && !output.ends_with("\n\n") {
                    output.pop();
                }
                if !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("````````````\n\n");
            }
            _ => {
                output.push_str(line);
                output.push('\n');
            }
        }
    }
    output
}

fn preserve_figure_boundaries(html: &str) -> String {
    html.replace("<figure", "\n\n<figure")
        .replace("</figure>", "\n\n</figure>\n\n")
        .replace("<figcaption", "\n\n<figcaption")
        .replace("</figcaption>", "</figcaption>\n\n")
}

fn add_image_view_links(source_html: &str, markdown: String) -> (String, Vec<String>) {
    let fragment = scraper::Html::parse_fragment(source_html);
    let Ok(selector) = scraper::Selector::parse("img[src]") else {
        return (markdown, Vec::new());
    };
    let mut markdown = markdown;
    let mut cursor = 0;
    let mut image_urls = Vec::new();

    for image in fragment.select(&selector) {
        let Some(src) = image.value().attr("src") else {
            continue;
        };

        let Some(relative_start) = markdown[cursor..].find("![") else {
            continue;
        };
        let start = cursor + relative_start;
        let Some(label_end_relative) = markdown[start..].find("](") else {
            continue;
        };
        let url_start = start + label_end_relative + 2;
        let Some(url_end_relative) = markdown[url_start..].find(')') else {
            continue;
        };
        let end = url_start + url_end_relative + 1;
        let image_markdown = &markdown[start..end];
        let image_index = image_urls.len();
        image_urls.push(src.to_owned());
        let target = format!("panda-image://{image_index}");
        let replacement =
            if image.value().attr("width").is_some() || image.value().attr("height").is_some() {
                let mut tag = format!("<img src=\"{}\"", escape_html_attr(src));
                if let Some(alt) = image.value().attr("alt") {
                    tag.push_str(&format!(" alt=\"{}\"", escape_html_attr(alt)));
                }
                if let Some(width) = image.value().attr("width") {
                    tag.push_str(&format!(" width=\"{}\"", escape_html_attr(width)));
                }
                if let Some(height) = image.value().attr("height") {
                    tag.push_str(&format!(" height=\"{}\"", escape_html_attr(height)));
                }
                tag.push_str(" />");
                format!("[{tag}]({target})")
            } else {
                format!("[{image_markdown}]({target})")
            };
        markdown.replace_range(start..end, &replacement);
        cursor = start + replacement.len();
    }

    (markdown, image_urls)
}

fn escape_html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Apply paragraph indentation after HTML conversion. The Markdown converter
/// trims leading whitespace inside blockquotes, which otherwise drops the
/// indentation from the first paragraph in those blocks.
fn indent_markdown_paragraphs(markdown: &str) -> String {
    const INDENT: &str = "\u{3000}\u{3000}";

    let mut out = String::with_capacity(markdown.len() + 64);
    let mut in_fence = false;
    let mut previous_blank = true;
    let mut previous_quote = false;

    for line in markdown.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let ending = if line.ends_with('\n') { "\n" } else { "" };
        let trimmed = content.trim_start();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            previous_blank = false;
            previous_quote = false;
            continue;
        }
        if in_fence {
            out.push_str(line);
            continue;
        }

        if trimmed.is_empty() {
            out.push_str(line);
            previous_blank = true;
            previous_quote = false;
            continue;
        }

        let (quote_prefix, text) = markdown_quote_prefix(content);
        let is_quote = !quote_prefix.is_empty();
        if is_quote && text.trim().is_empty() {
            out.push_str(line);
            previous_blank = true;
            previous_quote = false;
            continue;
        }
        let starts_paragraph = if is_quote {
            previous_blank || !previous_quote
        } else {
            previous_blank
        };
        let text = text.trim_start_matches([' ', '\t', '\u{3000}', '\u{00A0}']);

        if starts_paragraph && is_prose_line(text) {
            out.push_str(quote_prefix);
            out.push_str(INDENT);
            out.push_str(text);
        } else {
            out.push_str(content);
        }
        out.push_str(ending);
        previous_blank = false;
        previous_quote = is_quote;
    }

    out
}

fn markdown_quote_prefix(line: &str) -> (&str, &str) {
    let mut prefix_len = 0;
    loop {
        let rest = &line[prefix_len..];
        let marker = rest.trim_start_matches([' ', '\t']);
        let indentation = rest.len() - marker.len();
        if !marker.starts_with('>') {
            break;
        }
        prefix_len += indentation + 1;
        if line[prefix_len..].starts_with(' ') {
            prefix_len += 1;
        }
    }
    (&line[..prefix_len], &line[prefix_len..])
}

fn is_prose_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    !trimmed.is_empty()
        && !trimmed.starts_with('#')
        && !trimmed.starts_with("- ")
        && !trimmed.starts_with("* ")
        && !trimmed.starts_with("+ ")
        && !trimmed.starts_with("![")
        && !trimmed.starts_with('|')
        && !trimmed.starts_with("---")
        && !trimmed.starts_with("***")
        && !trimmed.starts_with("___")
        && !trimmed.starts_with("<")
        && !is_ordered_list_item(trimmed)
}

fn is_ordered_list_item(line: &str) -> bool {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0
        && line
            .as_bytes()
            .get(digits)
            .is_some_and(|byte| *byte == b'.' || *byte == b')')
}

#[cfg(test)]
mod tests {
    use super::super::test_article;
    use super::*;

    #[test]
    fn cache_distinguishes_display_options_and_invalidates_every_article_variant() {
        let mut article = test_article();
        let mut cache = BodyCache::default();
        for (translated, layout, hide_images) in [
            (false, TranslationLayout::Immersive, false),
            (false, TranslationLayout::Immersive, true),
            (true, TranslationLayout::Immersive, true),
            (true, TranslationLayout::Replaced, true),
        ] {
            let key = BodyPrepKey::from_article(&article, translated, layout, hide_images, true);
            let prepared =
                cache.get_or_insert(key, &article, translated, layout, hide_images, true);
            assert_eq!(
                prepared.html,
                prepare_body(&article, translated, layout, hide_images, true)
            );
        }
        assert_eq!(cache.entries.len(), 4);
        cache.invalidate_article(article.summary.id);
        assert!(cache.entries.is_empty());
        assert!(cache.order.is_empty());
        article.content_html = "<p>Updated body</p>".into();
        let key =
            BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false, true);
        let prepared = cache.get_or_insert(
            key,
            &article,
            false,
            TranslationLayout::Immersive,
            false,
            true,
        );
        assert!(prepared.html.contains("Updated body"));
    }

    #[test]
    fn cache_key_changes_when_equal_length_content_changes() {
        let mut article = test_article();
        article.content_html = "<p>First</p>".into();
        let original =
            BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false, true);
        let mut cache = BodyCache::default();
        assert!(
            cache
                .get_or_insert(
                    original,
                    &article,
                    false,
                    TranslationLayout::Immersive,
                    false,
                    true,
                )
                .html
                .contains("First")
        );

        article.content_html = "<p>Other</p>".into();
        let updated =
            BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false, true);
        assert_ne!(original, updated);
        assert!(
            cache
                .get_or_insert(
                    updated,
                    &article,
                    false,
                    TranslationLayout::Immersive,
                    false,
                    true,
                )
                .html
                .contains("Other")
        );
    }

    #[test]
    fn prepared_markdown_keeps_html_preformatted_code_as_a_fenced_block() {
        let mut article = test_article();
        article.content_html =
            "<pre><code>fn main() {\n    println!(\"hello\");\n}</code></pre>".into();
        let mut cache = BodyCache::default();
        let key =
            BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false, true);
        let prepared = cache.get_or_insert(
            key,
            &article,
            false,
            TranslationLayout::Immersive,
            false,
            true,
        );

        assert!(prepared.html.contains("<pre><code>"));
        assert!(prepared.markdown.contains("```"));
        assert!(prepared.markdown.contains("    println!(\"hello\");"));
        assert!(prepared.markdown.contains("fn main() {\n"));
    }
}
