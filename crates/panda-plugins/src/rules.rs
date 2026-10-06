//! Selector operations run in the host so plugins never ship an HTML parser.

use anyhow::{Context as _, bail};
use ego_tree::NodeId;
use html5ever::{Attribute, QualName, namespace_url};
use scraper::{ElementRef, Html, Node, Selector};
use serde::Deserialize;

const MAX_RULES: usize = 256;
const MAX_MATCHES_PER_RULE: usize = 10_000;
const MAX_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOCUMENT_NODES: usize = 100_000;

#[derive(Clone, Debug, Deserialize)]
pub struct RuleSet {
    #[serde(default)]
    pub rules: Vec<RuleAction>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RuleAction {
    SelectBody {
        selector: String,
    },
    Remove {
        selector: String,
    },
    RemoveText {
        selector: String,
        text: String,
    },
    RemoveTextContains {
        selector: String,
        text: String,
    },
    SetAttribute {
        selector: String,
        name: String,
        value: String,
    },
    RemoveAttribute {
        selector: String,
        name: String,
    },
    LazyImages {
        attributes: Vec<String>,
    },
}

/// Mutable host DOM with invocation-local, invalidatable handles.
pub struct ArticleDocument {
    html: Html,
    handles: Vec<NodeId>,
    invalid: Vec<NodeId>,
    query_budget: usize,
    modified: bool,
}

impl ArticleDocument {
    pub fn parse(source: &str) -> anyhow::Result<Self> {
        if source.len() > MAX_DOCUMENT_BYTES {
            bail!("document exceeds the 8 MiB processing limit");
        }
        let html = Html::parse_document(source);
        if html.tree.nodes().count() > MAX_DOCUMENT_NODES {
            bail!("document exceeds the 100,000 node limit");
        }
        Ok(Self {
            html,
            handles: Vec::new(),
            invalid: Vec::new(),
            query_budget: MAX_MATCHES_PER_RULE,
            modified: false,
        })
    }

    pub fn query(&mut self, selector: &str) -> anyhow::Result<Vec<u32>> {
        let selector = Selector::parse(selector)
            .map_err(|error| anyhow::anyhow!("invalid CSS selector `{selector}`: {error:?}"))?;
        let matches = self
            .html
            .select(&selector)
            .map(|element| element.id())
            .filter(|id| !self.invalid.contains(id))
            .take(self.query_budget + 1)
            .collect::<Vec<_>>();
        if matches.len() > self.query_budget {
            bail!("selector matched more than the per-operation node limit");
        }
        self.query_budget = self.query_budget.saturating_sub(matches.len());
        let mut handles = Vec::with_capacity(matches.len());
        for node in matches {
            self.handles.push(node);
            handles.push((self.handles.len() - 1) as u32);
        }
        Ok(handles)
    }

    pub fn text(&self, handle: u32) -> anyhow::Result<String> {
        let node = self.node_id(handle)?;
        let node = self.html.tree.get(node).context("unknown node handle")?;
        let mut text = String::new();
        for edge in node.traverse() {
            if let ego_tree::iter::Edge::Open(child) = edge
                && let Node::Text(value) = child.value()
            {
                text.push_str(&value.text);
            }
        }
        Ok(text)
    }

    pub fn html(&self, handle: u32) -> anyhow::Result<String> {
        let node = self.node_id(handle)?;
        let node = self.html.tree.get(node).context("unknown node handle")?;
        let Node::Element(_) = node.value() else {
            bail!("node handle is not an element");
        };
        Ok(ElementRef::wrap(node)
            .context("node handle is not an element")?
            .html())
    }

    pub fn attribute(&self, handle: u32, name: &str) -> anyhow::Result<Option<String>> {
        let node = self.node_id(handle)?;
        let node = self.html.tree.get(node).context("unknown node handle")?;
        let Node::Element(element) = node.value() else {
            bail!("node handle is not an element");
        };
        Ok(element
            .attrs
            .iter()
            .find(|(key, _)| key.local.as_ref().eq_ignore_ascii_case(name))
            .map(|(_, value)| value.to_string()))
    }

    pub fn remove(&mut self, handle: u32) -> anyhow::Result<()> {
        let id = self.node_id(handle)?;
        let descendants = self
            .html
            .tree
            .get(id)
            .context("unknown node handle")?
            .traverse()
            .filter_map(|edge| match edge {
                ego_tree::iter::Edge::Open(node) => Some(node.id()),
                ego_tree::iter::Edge::Close(_) => None,
            })
            .collect::<Vec<_>>();
        self.invalid.extend(descendants);
        self.html
            .tree
            .get_mut(id)
            .context("unknown node handle")?
            .detach();
        self.modified = true;
        Ok(())
    }

    pub fn set_attribute(
        &mut self,
        handle: u32,
        name: &str,
        value: Option<&str>,
    ) -> anyhow::Result<()> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            bail!("invalid HTML attribute name");
        }
        let id = self.node_id(handle)?;
        let node = self.html.tree.get(id).context("unknown node handle")?;
        let Node::Element(element) = node.value() else {
            bail!("node handle is not an element");
        };
        let previous = element
            .attrs
            .iter()
            .find(|(key, _)| key.local.as_ref().eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_ref());
        if previous == value {
            return Ok(());
        }
        let mut attributes = element
            .attrs
            .iter()
            .filter(|(key, _)| !key.local.as_ref().eq_ignore_ascii_case(name))
            .map(|(name, value)| Attribute {
                name: name.clone(),
                value: value.clone(),
            })
            .collect::<Vec<_>>();
        if let Some(value) = value {
            let local = html5ever::LocalName::from(name);
            let qual_name = QualName::new(None, namespace_url!(""), local);
            attributes.push(Attribute {
                name: qual_name,
                value: value.into(),
            });
        }
        let replacement = Node::Element(scraper::node::Element::new(
            element.name.clone(),
            attributes,
        ));
        *self
            .html
            .tree
            .get_mut(id)
            .context("unknown node handle")?
            .value() = replacement;
        self.modified = true;
        Ok(())
    }

    pub fn set_body_html(&mut self, handle: u32, html: &str) -> anyhow::Result<()> {
        if html.len() > 2 * 1024 * 1024 {
            bail!("replacement body exceeds the 2 MiB limit");
        }
        let id = self.node_id(handle)?;
        let node = self.html.tree.get(id).context("unknown node handle")?;
        if !matches!(node.value(), Node::Element(_)) {
            bail!("node handle is not an element");
        }
        let replacement = Html::parse_fragment(html);
        let contents = replacement.root_element().inner_html();
        let old_html = self.html.html();
        let outer = ElementRef::wrap(self.html.tree.get(id).expect("validated handle"))
            .context("node handle is not an element")?
            .html();
        let element_name = match self.html.tree.get(id).expect("validated handle").value() {
            Node::Element(element) => element.name().to_owned(),
            _ => unreachable!(),
        };
        let replacement = replace_element_contents(&outer, &element_name, &contents)?;
        let start = old_html
            .find(&outer)
            .context("could not locate body node")?;
        let end = start + outer.len();
        let updated = format!("{}{}{}", &old_html[..start], replacement, &old_html[end..]);
        self.html = Html::parse_document(&updated);
        self.handles.clear();
        self.invalid.clear();
        self.modified = true;
        Ok(())
    }

    pub fn select_body(&mut self, handle: u32) -> anyhow::Result<()> {
        let body = self.html(handle)?;
        let element = Html::parse_fragment(&body);
        self.html = Html::parse_document(&element.root_element().inner_html());
        self.handles.clear();
        self.invalid.clear();
        self.modified = true;
        Ok(())
    }

    pub fn serialize(&self) -> String {
        self.html.root_element().inner_html()
    }

    fn node_id(&self, handle: u32) -> anyhow::Result<NodeId> {
        let id = *self
            .handles
            .get(handle as usize)
            .context("unknown node handle")?;
        if self.invalid.contains(&id) {
            bail!("node handle was invalidated by a document edit");
        }
        Ok(id)
    }
}

impl RuleSet {
    pub fn selects_body(&self) -> bool {
        self.rules
            .iter()
            .any(|rule| matches!(rule, RuleAction::SelectBody { .. }))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.rules.len() > MAX_RULES {
            bail!("plugin has more than 256 rules");
        }
        for rule in &self.rules {
            let selector = match rule {
                RuleAction::SelectBody { selector }
                | RuleAction::Remove { selector }
                | RuleAction::RemoveText { selector, .. }
                | RuleAction::RemoveTextContains { selector, .. }
                | RuleAction::SetAttribute { selector, .. }
                | RuleAction::RemoveAttribute { selector, .. } => Some(selector.as_str()),
                RuleAction::LazyImages { .. } => None,
            };
            if let Some(selector) = selector {
                Selector::parse(selector).map_err(|error| {
                    anyhow::anyhow!("invalid CSS selector `{selector}`: {error:?}")
                })?;
            }
            let attribute = match rule {
                RuleAction::SetAttribute { name, .. }
                | RuleAction::RemoveAttribute { name, .. } => Some(name.as_str()),
                RuleAction::LazyImages { attributes } => {
                    for name in attributes {
                        validate_attribute_name(name)?;
                    }
                    None
                }
                RuleAction::SelectBody { .. }
                | RuleAction::Remove { .. }
                | RuleAction::RemoveText { .. }
                | RuleAction::RemoveTextContains { .. } => None,
            };
            if let Some(attribute) = attribute {
                validate_attribute_name(attribute)?;
            }
        }
        Ok(())
    }
}

fn validate_attribute_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("invalid HTML attribute name `{name}`");
    }
    Ok(())
}

fn replace_element_contents(outer: &str, name: &str, body: &str) -> anyhow::Result<String> {
    let opening_end = outer.find('>').context("invalid selected element")? + 1;
    let closing = format!("</{name}>");
    let closing_start = outer
        .rfind(&closing)
        .context("selected element has no closing tag")?;
    Ok(format!(
        "{}{}{}",
        &outer[..opening_end],
        body,
        &outer[closing_start..]
    ))
}

pub fn apply_rules(document: &mut ArticleDocument, rules: &RuleSet) -> anyhow::Result<()> {
    rules.validate()?;
    for action in &rules.rules {
        match action {
            RuleAction::SelectBody { selector } => {
                let matches = document.query(selector)?;
                if let Some(handle) = matches.first() {
                    document.select_body(*handle)?;
                }
            }
            RuleAction::Remove { selector } => {
                let handles = document.query(selector)?;
                for handle in handles {
                    document.remove(handle)?;
                }
            }
            RuleAction::RemoveText { selector, text } => {
                let expected = normalize_text(text);
                let mut matches = Vec::new();
                for handle in document.query(selector)? {
                    if normalize_text(&document.text(handle)?) == expected {
                        matches.push(handle);
                    }
                }
                for handle in matches {
                    document.remove(handle)?;
                }
            }
            RuleAction::RemoveTextContains { selector, text } => {
                let expected = normalize_text(text);
                if expected.is_empty() {
                    bail!("remove_text_contains requires non-empty text");
                }
                let mut matches = Vec::new();
                for handle in document.query(selector)? {
                    if normalize_text(&document.text(handle)?).contains(&expected) {
                        matches.push(handle);
                    }
                }
                for handle in matches {
                    document.remove(handle)?;
                }
            }
            RuleAction::SetAttribute {
                selector,
                name,
                value,
            } => {
                for handle in document.query(selector)? {
                    document.set_attribute(handle, name, Some(value))?;
                }
            }
            RuleAction::RemoveAttribute { selector, name } => {
                for handle in document.query(selector)? {
                    document.set_attribute(handle, name, None)?;
                }
            }
            RuleAction::LazyImages { attributes } => {
                for handle in document.query("img")? {
                    if document.attribute(handle, "src")?.is_some() {
                        continue;
                    }
                    for attribute in attributes {
                        if let Some(value) = document.attribute(handle, attribute)? {
                            document.set_attribute(handle, "src", Some(&value))?;
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Run one plugin against an isolated copy and commit only a valid result.
pub fn apply_rules_isolated(source: &str, rules: &RuleSet) -> anyhow::Result<Option<String>> {
    let mut document = ArticleDocument::parse(source)?;
    apply_rules(&mut document, rules)?;
    if !document.modified {
        return Ok(None);
    }
    let result = document.serialize();
    if result.trim().is_empty() {
        bail!("plugin produced an empty document");
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_actions_stay_in_the_host_document_and_invalidate_removed_handles() {
        let mut document = ArticleDocument::parse(
            "<article><p class='ad'>remove</p><p data-lazy='https://img.test/a.jpg'>keep</p></article>",
        )
        .unwrap();
        let removed = document.query(".ad").unwrap()[0];
        document.remove(removed).unwrap();
        assert!(document.text(removed).is_err());
        let image = document.query("p[data-lazy]").unwrap()[0];
        document
            .set_attribute(
                image,
                "src",
                document.attribute(image, "data-lazy").unwrap().as_deref(),
            )
            .unwrap();
        assert!(
            document
                .serialize()
                .contains("src=\"https://img.test/a.jpg\"")
        );
        assert!(!document.serialize().contains("remove"));
    }
}
