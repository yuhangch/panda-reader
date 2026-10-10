//! Bounded cache for prepared article bodies.

use super::content::{TRANSLATION_MARKER_END, TRANSLATION_MARKER_START, render_article};
use panda_core::{Article, ContentRevision, RenderDocument, RenderOptions, TranslationLayout};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderCacheKey {
    pub article_id: i64,
    pub canonical_revision: ContentRevision,
    pub translation_revision: Option<String>,
    pub options: RenderOptions,
}

impl RenderCacheKey {
    pub fn from_article(
        article: &Article,
        show_translation: bool,
        layout: TranslationLayout,
        hide_images: bool,
        paragraph_indent: bool,
    ) -> Self {
        Self {
            article_id: article.summary.id,
            canonical_revision: article
                .canonical
                .as_ref()
                .map(|canonical| canonical.revision.clone())
                .unwrap_or_else(|| ContentRevision(Arc::from("missing-canonical"))),
            translation_revision: (show_translation
                && article
                    .translated_html
                    .as_deref()
                    .is_some_and(|html| !html.trim().is_empty())
                && article
                    .canonical
                    .as_ref()
                    .zip(article.translation_source_hash.as_deref())
                    .is_some_and(|(canonical, revision)| {
                        canonical.matches_translation_revision(revision)
                    }))
            .then(|| article.translation_source_hash.clone())
            .flatten(),
            options: RenderOptions {
                show_translation,
                translation_layout: layout,
                hide_images,
                paragraph_indent,
            },
        }
    }
}

#[derive(Default)]
pub struct RenderCache {
    entries: HashMap<RenderCacheKey, Arc<RenderDocument>>,
    order: Vec<RenderCacheKey>,
    bytes: usize,
}

impl RenderCache {
    const CAPACITY_BYTES: usize = 32 * 1024 * 1024;

    /// Cached article bodies are disposable, so recover cleanly if an older
    /// renderer panicked while holding this lock.
    pub fn lock(cache: &Mutex<Self>) -> MutexGuard<'_, Self> {
        match cache.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                guard.clear();
                cache.clear_poison();
                eprintln!("article body cache recovered after a rendering panic");
                guard
            }
        }
    }

    pub fn get(&mut self, key: &RenderCacheKey) -> Option<Arc<RenderDocument>> {
        let value = self.entries.get(key).cloned()?;
        self.order.retain(|old| old != key);
        self.order.push(key.clone());
        Some(value)
    }

    /// Prepare a body outside the shared cache lock. A panic in a parser or
    /// converter is contained to this article request.
    pub fn prepare(article: &Article, options: RenderOptions) -> Result<RenderDocument, String> {
        if article.canonical.is_none() {
            return Err("article must be canonicalized before rendering".to_owned());
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Self::prepare_uncached(article, options)
        }))
        .map_err(|_| "article body rendering failed for this article".to_owned())
    }

    fn prepare_uncached(article: &Article, options: RenderOptions) -> RenderDocument {
        let canonical = article
            .canonical
            .as_ref()
            .expect("prepare validates that an article is canonicalized");
        let html = render_article(
            canonical,
            article.translated_html.as_deref(),
            article.translation_source_hash.as_deref(),
            &article.summary.snippet,
            options,
        );
        // TextView's HTML reader does not treat <pre> as a code block. Convert
        // the sanitized body off the UI thread so fenced code keeps its
        // whitespace, monospace styling, and block layout.
        let markdown_source = preserve_figure_boundaries(&html);
        let markdown = quick_html2md::html_to_markdown(&markdown_source);
        let (markdown, image_urls) = process_markdown_images(markdown);
        let markdown = wrap_translation_markdown(&markdown);
        let markdown = if options.paragraph_indent {
            indent_markdown_paragraphs(&markdown)
        } else {
            markdown
        };
        RenderDocument {
            html: Arc::from(html),
            markdown: Arc::from(markdown),
            image_urls: Arc::from(image_urls),
        }
    }

    pub fn insert(&mut self, key: RenderCacheKey, body: RenderDocument) -> Arc<RenderDocument> {
        if let Some(body) = self.entries.get(&key) {
            return body.clone();
        }
        let body = Arc::new(body);
        let size = body_size(&body);
        if size > Self::CAPACITY_BYTES {
            return body;
        }
        while self.bytes.saturating_add(size) > Self::CAPACITY_BYTES && !self.order.is_empty() {
            let old = self.order.remove(0);
            if let Some(evicted) = self.entries.remove(&old) {
                self.bytes = self.bytes.saturating_sub(body_size(&evicted));
            }
        }
        self.bytes = self.bytes.saturating_add(size);
        self.order.push(key.clone());
        self.entries.insert(key, body.clone());
        body
    }

    #[cfg(test)]
    pub fn get_or_insert(
        &mut self,
        key: RenderCacheKey,
        article: &Article,
        show_translation: bool,
        layout: TranslationLayout,
        hide_images: bool,
        paragraph_indent: bool,
    ) -> RenderDocument {
        if let Some(body) = self.get(&key) {
            return body.as_ref().clone();
        }
        let body = Self::prepare_uncached(
            article,
            RenderOptions {
                show_translation,
                translation_layout: layout,
                hide_images,
                paragraph_indent,
            },
        );
        self.insert(key, body).as_ref().clone()
    }

    pub fn invalidate_article(&mut self, article_id: i64) {
        self.order.retain(|key| key.article_id != article_id);
        let mut released = 0usize;
        self.entries.retain(|key, body| {
            if key.article_id == article_id {
                released = released.saturating_add(body_size(body));
                false
            } else {
                true
            }
        });
        self.bytes = self.bytes.saturating_sub(released);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.bytes = 0;
    }
}

fn body_size(body: &RenderDocument) -> usize {
    body.html.len() + body.markdown.len() + body.image_urls.iter().map(String::len).sum::<usize>()
}

fn wrap_translation_markdown(markdown: &str) -> String {
    const FENCE: &str = "````````````panda-translation";
    let mut output = String::with_capacity(markdown.len());
    let mut rest = markdown;
    let mut in_translation = false;

    loop {
        let next_start = rest
            .find(TRANSLATION_MARKER_START)
            .map(|index| (index, TRANSLATION_MARKER_START, true));
        let next_end = rest
            .find(TRANSLATION_MARKER_END)
            .map(|index| (index, TRANSLATION_MARKER_END, false));
        let next = match (next_start, next_end) {
            (Some(start), Some(end)) => Some(if start.0 <= end.0 { start } else { end }),
            (Some(marker), None) | (None, Some(marker)) => Some(marker),
            (None, None) => None,
        };
        let Some((index, marker, is_start)) = next else {
            output.push_str(rest);
            break;
        };

        output.push_str(&rest[..index]);
        rest = &rest[index + marker.len()..];
        if is_start && !in_translation {
            while output.ends_with('\n') && !output.ends_with("\n\n") {
                output.push('\n');
            }
            if !output.is_empty() && !output.ends_with("\n\n") {
                output.push_str("\n\n");
            }
            output.push_str(FENCE);
            output.push('\n');
            in_translation = true;
        } else if !is_start && in_translation {
            while output.ends_with('\n') {
                output.pop();
            }
            output.push_str("\n````````````\n\n");
            in_translation = false;
        }
    }

    // Do not leak an opening fence if malformed article HTML omitted the end marker.
    if in_translation {
        while output.ends_with('\n') {
            output.pop();
        }
        output.push_str("\n````````````\n");
    }
    output
}

fn preserve_figure_boundaries(html: &str) -> String {
    html.replace("<figure", "\n\n<figure")
        .replace("</figure>", "\n\n</figure>\n\n")
        .replace("<figcaption", "\n\n<figcaption")
        .replace("</figcaption>", "</figcaption>\n\n")
}

/// Turn Markdown image nodes into reader image blocks before translation
/// sections are wrapped in their own Markdown fence. This is a built-in
/// rendering step: feed and site-specific plugins should not need to repair
/// ordinary Markdown image syntax or linked-image wrappers.
fn process_markdown_images(markdown: String) -> (String, Vec<String>) {
    let Ok(tree) = markdown::to_mdast(&markdown, &markdown::ParseOptions::default()) else {
        return (markdown, Vec::new());
    };
    let mut replacements = Vec::new();
    collect_markdown_images(&tree, &mut replacements);
    replacements.sort_by_key(|replacement| replacement.range.start);

    let mut image_urls = Vec::with_capacity(replacements.len());
    let mut output = markdown;
    for (index, replacement) in replacements.iter().enumerate().rev() {
        image_urls.push((index, replacement.url.clone()));
        let alt = replacement.alt.replace(['\r', '\n'], " ");
        let block = format!(
            "\n\n{IMAGE_FENCE}panda-reader-image\npanda-image:{index}\n{alt}\n{IMAGE_FENCE}\n\n"
        );
        output.replace_range(replacement.range.clone(), &block);
    }
    image_urls.sort_by_key(|(index, _)| *index);
    (output, image_urls.into_iter().map(|(_, url)| url).collect())
}

struct MarkdownImageReplacement {
    range: std::ops::Range<usize>,
    url: String,
    alt: String,
}

fn collect_markdown_images(
    node: &markdown::mdast::Node,
    replacements: &mut Vec<MarkdownImageReplacement>,
) {
    use markdown::mdast::Node;

    // HTML-to-Markdown represents a linked image as [![alt](image)](link).
    // Replace the complete link so its Markdown brackets do not leak into
    // the article as visible punctuation after the image becomes a block.
    if let Node::Link(link) = node
        && let [Node::Image(image)] = link.children.as_slice()
        && let Some(position) = node.position()
    {
        replacements.push(MarkdownImageReplacement {
            range: position.start.offset..position.end.offset,
            url: image.url.clone(),
            alt: image.alt.clone(),
        });
        return;
    }

    if let Node::Image(image) = node
        && let Some(position) = node.position()
    {
        replacements.push(MarkdownImageReplacement {
            range: position.start.offset..position.end.offset,
            url: image.url.clone(),
            alt: image.alt.clone(),
        });
        return;
    }

    if let Some(children) = node.children() {
        for child in children {
            collect_markdown_images(child, replacements);
        }
    }
}

const IMAGE_FENCE: &str = "````````````````";

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

    type BodyCache = RenderCache;
    type BodyPrepKey = RenderCacheKey;
    type PreparedBody = RenderDocument;

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
                Arc::from(render_article(
                    article.canonical.as_ref().unwrap(),
                    article.translated_html.as_deref(),
                    article.translation_source_hash.as_deref(),
                    &article.summary.snippet,
                    RenderOptions {
                        show_translation: translated,
                        translation_layout: layout,
                        hide_images,
                        paragraph_indent: true,
                    },
                ))
            );
        }
        assert_eq!(cache.entries.len(), 4);
        cache.invalidate_article(article.summary.id);
        assert!(cache.entries.is_empty());
        assert!(cache.order.is_empty());
        article.canonical.as_mut().unwrap().html =
            panda_core::CanonicalHtml::new("<p>Updated body</p>");
        article.canonical.as_mut().unwrap().revision =
            ContentRevision(Arc::from("updated-body-v1"));
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
        article.canonical.as_mut().unwrap().html = panda_core::CanonicalHtml::new("<p>First</p>");
        article.canonical.as_mut().unwrap().revision = ContentRevision(Arc::from("first-v1"));
        let original =
            BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false, true);
        let mut cache = BodyCache::default();
        assert!(
            cache
                .get_or_insert(
                    original.clone(),
                    &article,
                    false,
                    TranslationLayout::Immersive,
                    false,
                    true,
                )
                .html
                .contains("First")
        );

        article.canonical.as_mut().unwrap().html = panda_core::CanonicalHtml::new("<p>Other</p>");
        article.canonical.as_mut().unwrap().revision = ContentRevision(Arc::from("other-v1"));
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
    fn cache_shares_entries_and_rejects_bodies_over_the_memory_budget() {
        let article = test_article();
        let key =
            BodyPrepKey::from_article(&article, false, TranslationLayout::Immersive, false, true);
        let mut cache = BodyCache::default();
        let inserted = cache.insert(
            key.clone(),
            PreparedBody {
                html: Arc::from("<p>body</p>"),
                markdown: Arc::from("body"),
                image_urls: Arc::from(Vec::<String>::new()),
            },
        );
        let retrieved = cache.get(&key).unwrap();
        assert!(Arc::ptr_eq(&inserted, &retrieved));

        let large_key = BodyPrepKey {
            article_id: 2,
            ..key.clone()
        };
        let large = "x".repeat(BodyCache::CAPACITY_BYTES + 1);
        let returned = cache.insert(
            large_key.clone(),
            PreparedBody {
                html: Arc::from(large),
                markdown: Arc::from(""),
                image_urls: Arc::from(Vec::<String>::new()),
            },
        );
        assert_eq!(returned.html.len(), BodyCache::CAPACITY_BYTES + 1);
        assert!(!cache.entries.contains_key(&large_key));
        assert!(cache.entries.contains_key(&key));
    }

    #[test]
    fn prepared_markdown_keeps_html_preformatted_code_as_a_fenced_block() {
        let mut article = test_article();
        article.canonical.as_mut().unwrap().html = panda_core::CanonicalHtml::new(
            "<pre><code>fn main() {\n    println!(\"hello\");\n}</code></pre>",
        );
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

    #[test]
    fn translation_markers_are_hidden_when_html_conversion_joins_them_to_text() {
        let markdown = format!(
            "### Orchestration of Finite State Machines\n{}### 有限状态机的编排{}",
            TRANSLATION_MARKER_START, TRANSLATION_MARKER_END
        );

        let wrapped = wrap_translation_markdown(&markdown);

        assert!(!wrapped.contains(TRANSLATION_MARKER_START));
        assert!(!wrapped.contains(TRANSLATION_MARKER_END));
        assert!(wrapped.contains("````````````panda-translation"));
        assert!(wrapped.contains("### 有限状态机的编排"));
    }
}
