//! Article extraction engines and the ordered cleanup pipeline.

use crate::html::split_blocks;
use ammonia::{Builder as Sanitizer, UrlRelative};
use panda_core::ContentExtractor;
use scraper::{Html, Selector};
use url::Url;

/// Extract, normalize, sanitize, and validate an article body.
pub fn extract_article_html(
    raw: &str,
    base: &str,
    article_title: &str,
    extractor: ContentExtractor,
) -> String {
    let extracted = match extractor {
        ContentExtractor::DomSmoothie => readability_article_html(raw, base),
        ContentExtractor::Decruft => decruft_article_html(raw, base),
        ContentExtractor::Trafilatura => trafilatura_article_html(raw, base),
        ContentExtractor::Heuristic => None,
    }
    .unwrap_or_else(|| heuristic_article_html(raw));

    // Some publishers repeat the feed headline as the first block of the
    // extracted body. Remove that standalone duplicate to avoid showing the
    // same headline twice; keep it when the first block contains more text.
    let without_duplicate_title = remove_leading_duplicate_title(&extracted, article_title);
    sanitize_html(&without_duplicate_title, Some(base))
}

/// Convert HTML text to a normalized representation for comparisons.
pub fn plain_text(html: &str) -> String {
    Html::parse_fragment(html)
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Apply the same safety and URL-rewriting policy to RSS content and extracted pages.
pub fn sanitize_html(html: &str, base: Option<&str>) -> String {
    let lower = html.to_ascii_lowercase();
    let has_embedded_media =
        lower.contains("<video") || lower.contains("<iframe") || lower.contains("<audio");
    let mut sanitizer = Sanitizer::default();
    if let Some(base) = base.and_then(|s| Url::parse(s).ok()) {
        sanitizer.url_relative(UrlRelative::RewriteWithBase(base));
    }
    let mut safe = sanitizer.clean(&html).to_string();
    if has_embedded_media {
        safe.push_str("<p><em>Embedded media is unavailable here. Open the original article to view it.</em></p>");
    }
    safe
}

/// Remove the repeated feed headline from the start of an extracted article body.
///
/// Several sources, including the ScienceNet article that exposed this issue,
/// include their page headline as the first body block even though Panda Reader
/// already displays the feed title above the body. Only an exact match is safe:
/// later mentions and paragraphs that continue after the headline are content.
pub fn remove_leading_duplicate_title(html: &str, article_title: &str) -> String {
    let title = comparable_text(article_title);
    if title.is_empty() {
        return html.to_owned();
    }

    // The duplicate can be wrapped in a table or div, so compare the first
    // complete HTML block instead of assuming the page uses a heading tag.
    let Some(block) = split_blocks(html)
        .into_iter()
        .find(|block| !plain_text(block).trim().is_empty())
    else {
        return html.to_owned();
    };
    if comparable_text(&plain_text(&block)) != title {
        return html.to_owned();
    }

    let Some(start) = html.find(&block) else {
        return html.to_owned();
    };
    let end = start + block.len();
    format!("{}{}", &html[..start], &html[end..])
}

fn comparable_text(text: &str) -> String {
    // Ignore formatting whitespace added around the same headline.
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn readability_article_html(raw: &str, base: &str) -> Option<String> {
    let mut reader = dom_smoothie::Readability::new(raw, Some(base), None).ok()?;
    let article = reader.parse().ok()?;
    let content = article.content.trim();
    if plain_text(content).trim().len() < 80 {
        return None;
    }
    Some(content.to_owned())
}

fn decruft_article_html(raw: &str, base: &str) -> Option<String> {
    let mut options = decruft::DecruftOptions::default();
    options.url = Some(base.to_owned());
    options.markdown = false;
    options.allow_network = false;
    options.remove_small_images = true;
    let result = decruft::parse(raw, &options);
    let content = result.content.trim();
    if content.is_empty() || plain_text(content).trim().len() < 80 {
        return None;
    }
    Some(content.to_owned())
}

fn trafilatura_article_html(raw: &str, base: &str) -> Option<String> {
    let options = rs_trafilatura::Options {
        url: Some(base.to_owned()),
        include_images: true,
        include_tables: true,
        include_links: true,
        include_formatting: true,
        favor_recall: true,
        ..rs_trafilatura::Options::default()
    };
    let result = rs_trafilatura::extract_with_options(raw, &options).ok()?;
    let content = result
        .content_html
        .filter(|html| !html.trim().is_empty())
        .unwrap_or(result.content_text);
    let content = content.trim();
    if plain_text(content).trim().len() < 80 {
        return None;
    }
    Some(content.to_owned())
}

pub fn heuristic_article_html(raw: &str) -> String {
    let document = Html::parse_document(raw);
    let selectors = [
        "article",
        "main",
        "[role='main']",
        ".article-content",
        ".article_content",
        "#article-content",
        ".post-content",
        ".entry-content",
        ".post-body",
        ".article-body",
        ".story-body",
        ".TRS_Editor",
        "#zoom",
        ".pages_content",
        ".Custom_UnionStyle",
        "#mainContent",
        ".main-content",
        ".content",
        "#content",
    ];
    let mut best: Option<(usize, String)> = None;
    for selector in selectors {
        let Ok(parsed) = Selector::parse(selector) else {
            continue;
        };
        for element in document.select(&parsed) {
            let html = element.inner_html();
            let score = plain_text(&html).trim().len();
            if score < 80 {
                continue;
            }
            // Prefer denser article-like nodes over huge page shells.
            let tag = element.value().name();
            let bonus = match tag {
                "article" => 400,
                "main" => 200,
                _ => 0,
            };
            let score = score + bonus;
            if best
                .as_ref()
                .is_none_or(|(best_score, _)| score > *best_score)
            {
                best = Some((score, html));
            }
        }
    }
    if best.is_none() {
        if let Ok(parsed) = Selector::parse("p") {
            let paragraphs = document
                .select(&parsed)
                .map(|element| element.html())
                .filter(|html| plain_text(html).trim().len() >= 40)
                .take(24)
                .collect::<Vec<_>>();
            if !paragraphs.is_empty() {
                let combined = paragraphs.join("\n");
                let score = plain_text(&combined).trim().len();
                if score >= 80 {
                    best = Some((score, combined));
                }
            }
        }
    }
    best.map(|(_, html)| html)
        .unwrap_or_else(|| document.root_element().inner_html())
}
