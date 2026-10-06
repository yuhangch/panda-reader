//! Host-owned article parsing, extraction and sanitization.

mod extraction;
pub mod html;

pub use extraction::{
    extract_article_html, heuristic_article_html, plain_text, remove_leading_duplicate_title,
    sanitize_html,
};
