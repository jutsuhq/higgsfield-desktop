//! `elements`: the account's reference elements (characters, environments, props) by name, and the `@ImageN` tags a
//! request's images get. `create_element` makes one from the agent's images.

use crate::{AgentVideoRequest, names};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Lists the account's reference elements, in the account's order: name words, a category, a page at a time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementsCommand {
    /// Words that all appear in the name, in any case.
    pub query: Option<String>,
    pub category: Option<String>,
    pub limit: Option<i64>,
    /// `nextCursor` of the page before.
    pub cursor: Option<String>,
}

pub const CATEGORIES: [&str; 3] = ["character", "environment", "prop"];

impl ElementsCommand {
    pub const DEFAULT_LIMIT: i64 = 20;
    pub const MAX_LIMIT: i64 = 50;
    /// In Unicode scalars.
    pub const MAX_QUERY_LENGTH: usize = 200;

    pub fn problem(&self) -> Option<String> {
        if self
            .query
            .as_deref()
            .is_some_and(|query| names::scalar_count(query) > Self::MAX_QUERY_LENGTH)
        {
            return Some(format!("query is at most {} characters", Self::MAX_QUERY_LENGTH));
        }
        if self
            .category
            .as_deref()
            .is_some_and(|category| !CATEGORIES.contains(&category))
        {
            return Some(format!("category is one of {}", CATEGORIES.join(", ")));
        }
        if self
            .limit
            .is_some_and(|limit| !(1..=Self::MAX_LIMIT).contains(&limit))
        {
            return Some(format!("limit is 1 to {}", Self::MAX_LIMIT));
        }
        if self.cursor.as_deref().is_some_and(|cursor| {
            !(1..=1024).contains(&cursor.len()) || cursor.chars().any(names::is_newline)
        }) {
            return Some("cursor is a nextCursor from an earlier answer".to_owned());
        }
        None
    }
}

/// One reference element of the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentElement {
    pub id: String,
    /// What `create_video` and `create_image` take in `elements`.
    pub name: String,
    pub category: Option<String>,
    pub image_count: Option<i64>,
    /// `create_element`: completed, processing or `ip_checking`; `None` in `elements`.
    pub status: Option<String>,
}

/// Makes a reference element on the account from the agent's images, idempotent by its request ID. Free.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateElementCommand {
    pub request_id: String,
    pub fingerprint: String,
    pub name: String,
    /// One of `CATEGORIES`; `None` is `DEFAULT_CATEGORY`.
    pub category: Option<String>,
    /// The images' paths: the agent's own files, then the copies the helper handed the app.
    pub images: Vec<String>,
}

impl CreateElementCommand {
    /// Higgsfield's own limit on an element's name.
    pub const MAX_NAME_LENGTH: usize = 32;
    /// A bound of the protocol: an element is a few clear images of one subject.
    pub const MAX_IMAGES: usize = 10;
    pub const DEFAULT_CATEGORY: &str = "character";

    pub fn problem(&self) -> Option<String> {
        if !names::is_valid_request_id(&self.request_id) {
            return Some(names::REQUEST_ID_RULE.to_owned());
        }
        if !names::is_valid_fingerprint(&self.fingerprint) {
            return Some("fingerprint is 64 lowercase hex digits".to_owned());
        }
        let trimmed = names::trim_whitespace_and_newlines(&self.name);
        if trimmed.is_empty() || names::character_count(trimmed) > Self::MAX_NAME_LENGTH {
            return Some(format!("name is 1 to {} characters", Self::MAX_NAME_LENGTH));
        }
        if self
            .category
            .as_deref()
            .is_some_and(|category| !CATEGORIES.contains(&category))
        {
            return Some(format!("category is one of {}", CATEGORIES.join(", ")));
        }
        if self.images.is_empty() {
            return Some("images takes the element's images, at least one".to_owned());
        }
        if self.images.len() > Self::MAX_IMAGES {
            return Some(format!("images takes at most {} images", Self::MAX_IMAGES));
        }
        if let Some(path) = self
            .images
            .iter()
            .find(|path| !AgentVideoRequest::is_valid_path(path))
        {
            return Some(format!(
                "images has a path that isn't one: {}",
                names::prefix(path, 80)
            ));
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentElementsPage {
    pub items: Vec<AgentElement>,
    pub next_cursor: Option<String>,
}

/// One image a request sent, by the tag its prompt names it with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentReference {
    /// "@Image1", "@Image2"…
    pub tag: String,
    /// The element it is an image of, by name; `None` for the agent's own file.
    pub element: Option<String>,
    pub file: String,
}

/// The checks `elements` gets in `create_video`, `quote` and `create_image`.
pub struct AgentElementNames;

impl AgentElementNames {
    pub const MAX_NAMES: usize = 4;
    pub const MAX_NAME_LENGTH: usize = 64;

    pub fn problem(names: &[String]) -> Option<String> {
        if names.len() > Self::MAX_NAMES {
            return Some(format!("elements takes at most {} names", Self::MAX_NAMES));
        }
        if names.iter().any(|name| {
            names::trim_whitespaces(name).is_empty() || names::character_count(name) > Self::MAX_NAME_LENGTH
        }) {
            return Some(format!(
                "elements are names from the elements tool, 1 to {} characters",
                Self::MAX_NAME_LENGTH
            ));
        }
        let trimmed: HashSet<String> = names
            .iter()
            .map(|name| names::trim_whitespaces(name).to_lowercase())
            .collect();
        (trimmed.len() != names.len()).then(|| "elements names an element twice".to_owned())
    }
}
