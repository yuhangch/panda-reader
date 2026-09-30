//! Split article HTML into top-level blocks for paragraph-level translation.

use sha2::{Digest, Sha256};

/// Fingerprint of the exact source that a translation was produced from.
pub fn source_hash(html: &str, title: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(html.as_bytes());
    hasher.update([0]);
    hasher.update(title.as_bytes());
    hex::encode(hasher.finalize())
}

/// Split HTML into top-level block elements (and bare text wrapped as `<p>`).
pub fn split_blocks(html: &str) -> Vec<String> {
    let mut blocks = Vec::new();
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
                blocks.push(block.to_owned());
            }
            rest = &rest[end..];
        } else {
            blocks.push(rest.to_owned());
            break;
        }
    }
    blocks
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
    fn skips_void_and_empty() {
        assert!(!block_needs_translation("<hr>"));
        assert!(!block_needs_translation("<p>   </p>"));
        assert!(block_needs_translation("<p>Hello</p>"));
    }
}
