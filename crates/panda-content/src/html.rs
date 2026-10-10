//! Split article HTML into top-level blocks for paragraph-level translation.

use anyhow::bail;
use sha2::{Digest, Sha256};
use std::future::Future;

/// Fingerprint of the exact source that a translation was produced from.
pub fn source_hash(html: &str, title: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(html.as_bytes());
    hasher.update([0]);
    hasher.update(title.as_bytes());
    hex::encode(hasher.finalize())
}

/// Cache identity for a translation, including the backend that produced it.
pub fn translation_cache_hash(html: &str, title: &str, provider: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(html.as_bytes());
    hasher.update([0]);
    hasher.update(title.as_bytes());
    hasher.update([0]);
    hasher.update(provider.as_bytes());
    hex::encode(hasher.finalize())
}

/// Cache identity for a translation derived from a canonical article revision.
/// The target language is part of the key so switching languages cannot reuse
/// a translation produced for another target.
pub fn translation_revision_hash(
    canonical_revision: &str,
    provider: &str,
    target_lang: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"panda-translation-target-v1\0");
    hasher.update(provider.as_bytes());
    hasher.update([0]);
    hasher.update(target_lang.as_bytes());
    format!(
        "panda-translation-v1:{canonical_revision}:{}",
        hex::encode(hasher.finalize())
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranslationInput {
    Html,
    PlainText,
}

#[derive(Clone, Debug, Default)]
pub struct HtmlTranslation {
    pub html: String,
    pub detected_source_lang: Option<String>,
}

/// A stable, host-owned HTML block prepared for a translation provider.
/// `input` contains placeholders for markup so the provider can translate prose
/// without becoming responsible for preserving tags or attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlTranslationSegment {
    pub id: String,
    pub source_hash: String,
    pub input: String,
    pub block_index: usize,
    placeholders: Vec<String>,
    source_block: String,
}

impl HtmlTranslationSegment {
    /// Restore the source markup into translated text. Placeholder loss,
    /// duplication, or reordering is rejected so invalid markup is never saved.
    pub fn restore(&self, translated: &str) -> anyhow::Result<String> {
        let mut output = translated.trim().to_owned();
        let mut positions = Vec::with_capacity(self.placeholders.len());
        for index in 0..self.placeholders.len() {
            let token = placeholder_token(index);
            let mut matches = output.match_indices(&token);
            let Some((position, _)) = matches.next() else {
                bail!("Translation response is missing HTML placeholder {index}");
            };
            if matches.next().is_some() {
                bail!("Translation response duplicated HTML placeholder {index}");
            }
            positions.push(position);
        }
        if positions.windows(2).any(|window| window[0] > window[1]) {
            bail!("Translation response reordered HTML placeholders");
        }
        for (index, raw) in self.placeholders.iter().enumerate().rev() {
            output = output.replace(&placeholder_token(index), raw);
        }
        if output.trim().is_empty() {
            bail!("Translation response is empty");
        }
        // The outer block is owned by the source document. Require the model
        // to return its original wrapper and replace that wrapper verbatim.
        let source_name = leading_tag_name(&self.source_block);
        if let Some(tag) = source_name {
            let open_end = find_tag_end(&self.source_block, 0).unwrap_or(0);
            let close_start = self
                .source_block
                .to_ascii_lowercase()
                .rfind(&format!("</{tag}"));
            if open_end > 0
                && let Some(close_start) = close_start
            {
                let inner = output
                    .trim()
                    .strip_prefix(&self.source_block[..open_end])
                    .and_then(|value| value.strip_suffix(&self.source_block[close_start..]));
                if let Some(inner) = inner {
                    return Ok(format!(
                        "{}{}{}",
                        &self.source_block[..open_end],
                        inner,
                        &self.source_block[close_start..]
                    ));
                }
            }
        }
        Ok(output)
    }
}

/// Prepare stable translation units from canonical article HTML. Non-prose
/// blocks such as code are deliberately excluded.
pub fn prepare_html_translation_segments(html: &str) -> Vec<HtmlTranslationSegment> {
    let mut segments = Vec::new();
    let mut occurrences = std::collections::HashMap::<String, usize>::new();
    for (block_index, block) in split_blocks(html).into_iter().enumerate() {
        if !block_needs_translation(&block) || is_code_block(&block) {
            continue;
        }
        let mut input = String::with_capacity(block.len());
        let mut placeholders = Vec::new();
        let mut cursor = 0;
        while cursor < block.len() {
            let Some(relative_start) = block[cursor..].find('<') else {
                input.push_str(&block[cursor..]);
                break;
            };
            let start = cursor + relative_start;
            input.push_str(&block[cursor..start]);
            if let Some((end, raw)) = protected_markup_at(&block, start) {
                let index = placeholders.len();
                input.push_str(&placeholder_token(index));
                placeholders.push(raw);
                cursor = end;
            } else {
                input.push('<');
                cursor = start + 1;
            }
        }
        let source_hash = hex::encode(Sha256::digest(block.as_bytes()));
        let occurrence = occurrences.entry(source_hash.clone()).or_default();
        let id = format!("s{source_hash}-{}", *occurrence);
        *occurrence += 1;
        segments.push(HtmlTranslationSegment {
            id,
            source_hash,
            input,
            block_index,
            placeholders,
            source_block: block,
        });
    }
    segments
}

/// Assemble validated, restored translations using the source block order.
pub fn assemble_html_translation(
    html: &str,
    segments: &[HtmlTranslationSegment],
    translations: &std::collections::HashMap<String, String>,
) -> anyhow::Result<String> {
    let blocks = split_blocks(html);
    let mut output = blocks.clone();
    for segment in segments {
        let translated = translations
            .get(&segment.id)
            .ok_or_else(|| anyhow::anyhow!("Translation is missing segment {}", segment.id))?;
        output[segment.block_index] = translated.clone();
    }
    Ok(output.join("\n"))
}

/// Assemble any completed block translations while leaving unfinished blocks
/// in their original positions. This is intended for progressive rendering;
/// callers must still keep the result marked as incomplete until all segments
/// have been translated.
pub fn assemble_partial_html_translation(
    html: &str,
    translations: &std::collections::HashMap<String, String>,
) -> anyhow::Result<String> {
    let blocks = split_blocks(html);
    let mut output = blocks.clone();
    for segment in prepare_html_translation_segments(html) {
        if let Some(translated) = translations.get(&segment.id) {
            output[segment.block_index] = translated.clone();
        }
    }
    Ok(output.join("\n"))
}

fn placeholder_token(index: usize) -> String {
    format!("⟪PANDA_HTML_{index:04}⟫")
}

fn protected_markup_at(html: &str, start: usize) -> Option<(usize, String)> {
    let lower = html[start..].to_ascii_lowercase();
    // Preserve code, including its contents, as one indivisible unit.
    for tag in ["code", "pre", "math", "svg"] {
        if lower.starts_with(&format!("<{tag}")) {
            let close = lower.find(&format!("</{tag}>"))? + tag.len() + 3;
            return Some((start + close, html[start..start + close].to_owned()));
        }
    }
    let end = find_tag_end(html, start)?;
    Some((end, html[start..end].to_owned()))
}

fn find_tag_end(html: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, ch) in html[start..].char_indices() {
        match (quote, ch) {
            (Some(active), current) if active == current => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => return Some(start + offset + 1),
            _ => {}
        }
    }
    None
}

fn leading_tag_name(block: &str) -> Option<String> {
    let rest = block.trim_start().strip_prefix('<')?;
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .collect();
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

fn is_code_block(block: &str) -> bool {
    matches!(
        leading_tag_name(block).as_deref(),
        Some("pre" | "code" | "math" | "svg")
    )
}

/// Translate top-level HTML blocks while preserving their position and markup.
/// Provider-specific request formats and limits are supplied by the caller.
pub async fn translate_html_blocks<F, Fut>(
    html: &str,
    input: TranslationInput,
    max_batch_items: usize,
    max_batch_chars: usize,
    max_item_chars: usize,
    mut translate_batch: F,
) -> anyhow::Result<HtmlTranslation>
where
    F: FnMut(&[String]) -> Fut,
    Fut: Future<Output = anyhow::Result<(Vec<String>, Option<String>)>>,
{
    let blocks = split_blocks(html);
    let mut output = blocks.clone();
    let mut chunk_blocks = Vec::new();
    let mut chunks = Vec::new();

    for (index, block) in blocks.iter().enumerate() {
        if !block_needs_translation(block) {
            continue;
        }
        let parts = match input {
            TranslationInput::Html => vec![block.clone()],
            TranslationInput::PlainText => {
                split_text_for_translate(&html_block_to_text(block), max_item_chars)
            }
        };
        for part in parts {
            chunk_blocks.push(index);
            chunks.push(part);
        }
    }

    let mut translated_chunks = vec![Vec::<String>::new(); blocks.len()];
    let max_batch_items = max_batch_items.max(1);
    let max_batch_chars = max_batch_chars.max(1);
    let mut cursor = 0;
    let mut detected_source_lang = None;
    while cursor < chunks.len() {
        let mut end = cursor;
        let mut chars = 0;
        while end < chunks.len() && end - cursor < max_batch_items {
            let next = chunks[end].chars().count();
            if end > cursor && chars + next > max_batch_chars {
                break;
            }
            chars += next;
            end += 1;
        }
        if end == cursor {
            end += 1;
        }

        let (parts, source) = translate_batch(&chunks[cursor..end]).await?;
        if parts.len() != end - cursor {
            bail!(
                "Translator returned {} results for {} inputs",
                parts.len(),
                end - cursor
            );
        }
        if detected_source_lang.is_none() {
            detected_source_lang = source;
        }
        for (offset, part) in parts.into_iter().enumerate() {
            translated_chunks[chunk_blocks[cursor + offset]].push(part);
        }
        cursor = end;
    }

    for (index, parts) in translated_chunks.into_iter().enumerate() {
        if parts.is_empty() {
            continue;
        }
        let translated = parts.concat();
        output[index] = match input {
            TranslationInput::Html => translated,
            TranslationInput::PlainText => wrap_block_translation(&blocks[index], &translated),
        };
    }
    Ok(HtmlTranslation {
        html: output.join("\n"),
        detected_source_lang,
    })
}

pub fn split_text_for_translate(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    if text.chars().count() <= max_chars {
        return vec![text.to_owned()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for ch in text.chars() {
        let near_limit = current_len >= max_chars.saturating_sub(200);
        if near_limit && current_len > 0 && ch.is_whitespace() {
            current.push(ch);
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
            continue;
        }
        if current_len >= max_chars {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current.push(ch);
        current_len += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn html_block_to_text(block: &str) -> String {
    let text = plain_text_with_block_spacing(block);
    let mut decoded = text;
    for (entity, character) in [
        ("&nbsp;", " "),
        ("&ensp;", " "),
        ("&emsp;", " "),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&#x27;", "'"),
        ("&amp;", "&"),
    ] {
        decoded = decoded.replace(entity, character);
    }
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn plain_text_with_block_spacing(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                let name = tag
                    .trim_start_matches('/')
                    .split(|ch: char| ch.is_whitespace() || ch == '/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if is_text_separator(&name) {
                    out.push(' ');
                }
                in_tag = false;
            }
            _ if in_tag => tag.push(ch),
            _ => out.push(ch),
        }
    }
    out
}

fn is_text_separator(tag: &str) -> bool {
    matches!(
        tag,
        "br" | "p"
            | "div"
            | "li"
            | "ul"
            | "ol"
            | "blockquote"
            | "pre"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "tr"
            | "td"
            | "th"
    )
}

fn wrap_block_translation(source: &str, translation: &str) -> String {
    let name = source
        .trim_start()
        .strip_prefix('<')
        .map(|rest| {
            rest.chars()
                .take_while(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .unwrap_or_default();
    let tag = match name.as_str() {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "blockquote" | "figcaption" => {
            name.as_str()
        }
        _ => "p",
    };
    format!("<{tag}>{}</{tag}>", escape_html(translation.trim()))
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Split HTML into top-level block elements (and bare text wrapped as `<p>`).
pub fn split_blocks(html: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    collect_blocks(html, &mut blocks);
    blocks
}

fn collect_blocks(html: &str, blocks: &mut Vec<String>) {
    let mut rest = html.trim_start();
    while !rest.is_empty() {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if !rest.starts_with('<') {
            if let Some(next_tag) = rest.find('<') {
                let (text, next) = rest.split_at(next_tag);
                let text = text.trim();
                if !text.is_empty() {
                    blocks.push(format!("<p>{text}</p>"));
                }
                rest = next;
            } else {
                let text = rest.trim();
                if !text.is_empty() {
                    blocks.push(format!("<p>{text}</p>"));
                }
                break;
            }
            continue;
        }
        let Some((tag, open_end)) = read_open_tag(rest) else {
            blocks.push(rest.to_owned());
            break;
        };
        if is_void_tag(tag)
            || rest[..open_end].as_bytes().get(open_end.saturating_sub(2)) == Some(&b'/')
        {
            blocks.push(rest[..open_end].to_owned());
            rest = &rest[open_end..];
            continue;
        }
        if let Some(end) = find_matching_close(rest, tag, open_end) {
            let block = rest[..end].trim();
            if !block.is_empty() {
                if is_transparent_container(tag) {
                    let close_start = block.rfind("</").unwrap_or(block.len());
                    if open_end <= close_start {
                        collect_blocks(&block[open_end..close_start], blocks);
                    }
                } else {
                    blocks.push(block.to_owned());
                }
            }
            rest = &rest[end..];
        } else {
            blocks.push(rest.to_owned());
            break;
        }
    }
}

fn is_transparent_container(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "html" | "body" | "div" | "article" | "main" | "section"
    )
}

/// True when a block has alphanumeric text worth sending to a translator.
pub fn block_needs_translation(block: &str) -> bool {
    plain_text(block).chars().any(|ch| ch.is_alphanumeric())
}

pub fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn read_open_tag(html: &str) -> Option<(&str, usize)> {
    if !html.starts_with('<') || html.starts_with("</") {
        return None;
    }
    let close = html.find('>')?;
    let name_end = html[1..=close]
        .find(|ch: char| ch.is_whitespace() || ch == '/' || ch == '>')
        .map(|index| index + 1)
        .unwrap_or(close);
    let tag = html[1..name_end].trim();
    if tag.is_empty() {
        return None;
    }
    Some((tag, close + 1))
}

fn find_matching_close(html: &str, tag: &str, mut index: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let open_prefix = format!("<{tag}").to_ascii_lowercase();
    let close_tag = format!("</{tag}>").to_ascii_lowercase();
    let open_prefix_b = open_prefix.as_bytes();
    let close_b = close_tag.as_bytes();
    let mut depth = 1usize;
    while index < bytes.len() {
        if starts_with_ignore_ascii_case(bytes, index, close_b) {
            depth -= 1;
            index += close_b.len();
            if depth == 0 {
                return Some(index);
            }
            continue;
        }
        if starts_with_ignore_ascii_case(bytes, index, open_prefix_b) {
            let after = index + open_prefix_b.len();
            let boundary = bytes.get(after).copied().unwrap_or(b'>');
            if matches!(boundary, b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r') {
                if let Some(rel) = bytes[after..].iter().position(|&b| b == b'>') {
                    let open_end = after + rel;
                    let self_closing = open_end > 0 && bytes[open_end - 1] == b'/';
                    if !self_closing {
                        depth += 1;
                    }
                    index = open_end + 1;
                    continue;
                }
            }
        }
        index += 1;
    }
    None
}

fn starts_with_ignore_ascii_case(haystack: &[u8], index: usize, needle: &[u8]) -> bool {
    let Some(slice) = haystack.get(index..index + needle.len()) else {
        return false;
    };
    slice.eq_ignore_ascii_case(needle)
}

fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "br" | "hr" | "img" | "source" | "meta" | "link"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_paragraphs() {
        let blocks = split_blocks("<p>One</p><p>Two</p>");
        assert_eq!(
            blocks,
            vec!["<p>One</p>".to_owned(), "<p>Two</p>".to_owned()]
        );
    }

    #[test]
    fn hash_changes_with_content() {
        let a = source_hash("<p>hi</p>", "Title");
        let b = source_hash("<p>hi!</p>", "Title");
        let c = source_hash("<p>hi</p>", "Title!");
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(a, source_hash("<p>hi</p>", "Title"));
    }

    #[test]
    fn translation_revision_tracks_canonical_provider_and_target() {
        let revision = translation_revision_hash("canonical-a", "azure", "zh-Hans");
        assert_eq!(
            revision,
            translation_revision_hash("canonical-a", "azure", "zh-Hans")
        );
        assert_ne!(
            revision,
            translation_revision_hash("canonical-b", "azure", "zh-Hans")
        );
        assert_ne!(
            revision,
            translation_revision_hash("canonical-a", "volcengine", "zh-Hans")
        );
        assert_ne!(
            revision,
            translation_revision_hash("canonical-a", "azure", "ja")
        );
    }

    #[test]
    fn skips_void_and_empty() {
        assert!(!block_needs_translation("<hr>"));
        assert!(!block_needs_translation("<p>   </p>"));
        assert!(block_needs_translation("<p>Hello</p>"));
    }

    #[test]
    fn translation_segments_keep_markup_and_inline_code_host_owned() {
        let source = r#"<p>Use <a href="https://example.com?a=1&amp;b=2"><strong>Rust</strong></a> with <code>Vec&lt;T&gt;</code><img src="/chart.png" alt="chart"></p><pre>fn main() {}</pre>"#;
        let segments = prepare_html_translation_segments(source);
        assert_eq!(segments.len(), 1);
        assert!(!segments[0].input.contains("https://example.com"));
        assert!(!segments[0].input.contains("Vec&lt;T&gt;"));
        assert!(segments[0].input.contains("Rust"));

        let translated = segments[0].input.replace("Rust", "Rust语言");
        let restored = segments[0].restore(&translated).unwrap();
        assert!(restored.contains("href=\"https://example.com?a=1&amp;b=2\""));
        assert!(restored.contains("<strong>Rust语言</strong>"));
        assert!(restored.contains("<code>Vec&lt;T&gt;</code>"));
        assert!(restored.contains("<img src=\"/chart.png\" alt=\"chart\">"));
    }

    #[test]
    fn nested_preformatted_code_is_never_sent_as_prose() {
        let segment = prepare_html_translation_segments(
            "<blockquote><pre>fn main() { println!(\"Rust\"); }</pre><p>Explanation</p></blockquote>",
        )
        .remove(0);
        assert!(!segment.input.contains("println!"));
        assert!(segment.input.contains("Explanation"));
        let restored = segment
            .restore(&segment.input.replace("Explanation", "说明"))
            .unwrap();
        assert!(restored.contains("fn main() { println!(\"Rust\"); }"));
        assert!(restored.contains("说明"));
    }

    #[test]
    fn translation_segment_ids_survive_insertions_and_distinguish_duplicates() {
        let original = prepare_html_translation_segments("<p>Same block</p><p>Other block</p>");
        let inserted = prepare_html_translation_segments(
            "<p>New block</p><p>Same block</p><p>Other block</p>",
        );
        assert_eq!(original[0].id, inserted[1].id);
        assert_eq!(original[1].id, inserted[2].id);

        let duplicates = prepare_html_translation_segments("<p>Same block</p><p>Same block</p>");
        assert_ne!(duplicates[0].id, duplicates[1].id);
    }

    #[test]
    fn translation_segment_rejects_lost_or_reordered_markup_tokens() {
        let segment =
            prepare_html_translation_segments("<p><b>one</b> and <i>two</i></p>").remove(0);
        let tokens: Vec<_> = (0..segment.placeholders.len())
            .map(placeholder_token)
            .collect();
        assert!(segment.restore("translated without tokens").is_err());
        assert!(
            segment
                .restore(&format!("{}{}{}{}", tokens[1], "x", tokens[0], "y"))
                .is_err()
        );
    }

    #[test]
    fn assembly_uses_source_order_and_preserves_non_text_blocks() {
        let html = "<p>First</p><img src=\"/cover.png\"><p>Last</p>";
        let segments = prepare_html_translation_segments(html);
        let translations = segments
            .iter()
            .map(|segment| {
                (
                    segment.id.clone(),
                    segment
                        .restore(
                            &segment
                                .input
                                .replace("First", "第一")
                                .replace("Last", "最后"),
                        )
                        .unwrap(),
                )
            })
            .collect();
        let output = assemble_html_translation(html, &segments, &translations).unwrap();
        assert!(output.find("<p>第一</p>").unwrap() < output.find("<img").unwrap());
        assert!(output.find("<img").unwrap() < output.find("<p>最后</p>").unwrap());
    }
}
