//! `show_reference_elements` as the catalog snapshot of 2026-10-15 states it: `action` `list` (`size` 1–100,
//! `cursor` from the last page's `next_cursor`) or `get` (`element_id`), both answering `{items, next_cursor}`;
//! `create` takes `name` (at most 32 characters), `category` and `medias: [{id, url, type}]` of confirmed uploads,
//! and answers with the new element under `element`. An element is used by its image IDs, never by its own ID.

use serde_json::{Value, json};
use url::Url;

use super::{HiggsfieldAccount, ToolRole, UploadedMedia, failure, insert_cursor, object};
use crate::core::HiggsfieldError;
use crate::mcp::json::{self, Object};

/// What a reference element shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceElementCategory {
    Character,
    Environment,
    Prop,
}

impl ReferenceElementCategory {
    pub const ALL: [Self; 3] = [Self::Character, Self::Environment, Self::Prop];

    #[must_use]
    pub fn raw_value(self) -> &'static str {
        match self {
            Self::Character => "character",
            Self::Environment => "environment",
            Self::Prop => "prop",
        }
    }

    fn from_raw_value(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.raw_value() == text)
    }
}

/// One image of a reference element.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceElementImage {
    pub url: Url,
    /// The `media_id` or image job ID generations take; `None` when the answer names none.
    pub media_id: Option<String>,
}

/// A named character, environment or prop the account keeps.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferenceElement {
    pub id: String,
    pub name: String,
    /// `None` when the answer names none.
    pub category: Option<ReferenceElementCategory>,
    pub thumbnail_url: Option<Url>,
    /// An element's images: only the first one in a list, all of them from `get`.
    pub images: Vec<ReferenceElementImage>,
    /// The state as the server names it: `completed`, `processing`, `ip_checking`, `failed`, `nsfw` or
    /// `kling_failed`.
    pub status: Option<String>,
}

/// One page of elements, in the server's order.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceElementPage {
    pub items: Vec<ReferenceElement>,
    /// Opaque: sent back unchanged for the next page. `None` on the last page.
    pub next_cursor: Option<Value>,
}

impl ReferenceElementPage {
    /// `None` when the answer has no `items`. Items without an ID are left out.
    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let items = json::array(value, "items")?
            .iter()
            .filter_map(ReferenceElement::from_value)
            .collect();
        Some(Self {
            items,
            next_cursor: json::cursor(value, "next_cursor"),
        })
    }
}

impl ReferenceElement {
    /// Previews an answer may name, best first; else the first image.
    const THUMBNAIL_KEYS: [&'static str; 3] = ["thumbnail_url", "preview_url", "url"];
    /// The suffix of a category the server verified for protected content (`character_ip_verified`).
    const VERIFIED_SUFFIX: &'static str = "_ip_verified";

    pub(crate) fn from_value(value: &Value) -> Option<Self> {
        let id = json::string(value, "id").filter(|id| !id.is_empty())?.to_owned();
        let images: Vec<ReferenceElementImage> = json::array(value, "medias")
            .into_iter()
            .flatten()
            .filter_map(|media| {
                Some(ReferenceElementImage {
                    url: json::media_url(media, "url")?,
                    media_id: json::string(media, "id").map(str::to_owned),
                })
            })
            .collect();
        let thumbnail = Self::THUMBNAIL_KEYS
            .iter()
            .find_map(|key| json::media_url(value, key));
        let category = json::string(value, "category")
            .map(|name| name.strip_suffix(Self::VERIFIED_SUFFIX).unwrap_or(name))
            .and_then(ReferenceElementCategory::from_raw_value);
        Some(Self {
            id,
            name: json::string(value, "name").unwrap_or("").to_owned(),
            category,
            thumbnail_url: thumbnail.or_else(|| images.first().map(|image| image.url.clone())),
            images,
            status: json::string(value, "status").map(str::to_owned),
        })
    }
}

impl HiggsfieldAccount {
    /// `create`'s `name`: "max 32 chars".
    pub const ELEMENT_NAME_LIMIT: usize = 32;

    /// A page of the account's reference elements of the selected workspace, newest first. `size` is 1–100.
    pub async fn reference_elements(
        &self,
        cursor: Option<&Value>,
        size: i64,
    ) -> Result<ReferenceElementPage, HiggsfieldError> {
        let mut arguments = object(json!({ "action": "list", "size": size.clamp(1, 100) }));
        insert_cursor(&mut arguments, cursor);
        let value = self.call(ToolRole::ReferenceElements, arguments).await?;
        ReferenceElementPage::from_value(&value).ok_or_else(|| {
            HiggsfieldError::validation(
                json::string(&value, "error")
                    .unwrap_or("Unexpected result from the show_reference_elements tool"),
            )
        })
    }

    /// One element with all its images (`get`).
    pub async fn reference_element(&self, id: &str) -> Result<ReferenceElement, HiggsfieldError> {
        let value = self
            .call(
                ToolRole::ReferenceElements,
                object(json!({ "action": "get", "element_id": id })),
            )
            .await?;
        ReferenceElementPage::from_value(&value)
            .and_then(|page| page.items.into_iter().next())
            .ok_or_else(|| {
                HiggsfieldError::validation(
                    json::string(&value, "error")
                        .unwrap_or("Unexpected result from the show_reference_elements tool"),
                )
            })
    }

    /// `create`: a new element from confirmed uploads, each sent as `media_input`. Not idempotent: sent again, it
    /// makes a second element. `name` is cut to `ELEMENT_NAME_LIMIT` characters; `None` or empty lets the server
    /// derive one from the images. `category` `None` lets the server classify the element (`auto`).
    pub async fn create_reference_element(
        &self,
        name: Option<&str>,
        category: Option<ReferenceElementCategory>,
        images: &[UploadedMedia],
    ) -> Result<ReferenceElement, HiggsfieldError> {
        // `list` and `get` work without `create`'s arguments, so the role alone doesn't say the tool creates.
        let creates = self
            .look_up(ToolRole::ReferenceElements, std::time::Duration::ZERO)
            .await?
            .is_some_and(|tool| tool.argument_names().iter().any(|name| name == "medias"));
        if !creates {
            return Err(HiggsfieldError::ToolUnavailable(
                ToolRole::ReferenceElements.raw_value().to_owned(),
            ));
        }
        let medias: Vec<Value> = images
            .iter()
            .map(|image| json!({ "id": image.id, "url": image.url.as_str(), "type": "media_input" }))
            .collect();
        let mut arguments: Object = object(json!({
            "action": "create",
            "category": category.map_or("auto", ReferenceElementCategory::raw_value),
            "medias": medias,
        }));
        let name = name.map(|name| {
            name.trim()
                .chars()
                .take(Self::ELEMENT_NAME_LIMIT)
                .collect::<String>()
                .trim()
                .to_owned()
        });
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            arguments.insert("name".into(), name.into());
        }
        let value = self.call(ToolRole::ReferenceElements, arguments).await?;
        json::get(&value, "element")
            .and_then(ReferenceElement::from_value)
            .ok_or_else(|| failure(&value, ToolRole::ReferenceElements))
    }
}
