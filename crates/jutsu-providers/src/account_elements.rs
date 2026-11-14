//! Maps the account's completed reference elements to reusable image inputs.

use async_trait::async_trait;
use higgsfield::core::is_https;
use higgsfield::mcp::account::{ReferenceElement, ReferenceElementCategory, UploadedMedia};
use jutsu_core::{
    AccountElement, AccountElementPage, AccountElements, ElementCategory, ElementImage, HistoryCursor,
    PortError, RemoteAsset, ServerMedia,
};

use crate::history_mapping;
use crate::mapping::{port, validation};
use crate::mcp_provider::{HiggsfieldMcpProvider, page_cursor, unexpected_upload};

pub(crate) const ELEMENT_PAGE_SIZE: i64 = 30;

/// Whether the element can be used: only a `completed` one ("skip processing / ip_checking / failed / kling_failed
/// / nsfw", says the tool). No state reads as usable.
pub(crate) fn is_usable(element: &ReferenceElement) -> bool {
    element.status.as_ref().is_none_or(|status| status == "completed")
}

fn category(category: ReferenceElementCategory) -> Option<ElementCategory> {
    ElementCategory::from_raw(category.raw_value())
}

fn reference_category(category: ElementCategory) -> ReferenceElementCategory {
    match category {
        ElementCategory::Character => ReferenceElementCategory::Character,
        ElementCategory::Environment => ReferenceElementCategory::Environment,
        ElementCategory::Prop => ReferenceElementCategory::Prop,
    }
}

/// `listed`: from a list page, which names only an element's first image: it is kept as the thumbnail and the
/// images are left out, so picking the element asks `get` for all of them.
pub(crate) fn element(element: &ReferenceElement, owner: Option<&str>, listed: bool) -> AccountElement {
    AccountElement {
        id: element.id.clone(),
        name: element.name.clone(),
        category: element.category.and_then(category),
        thumbnail_url: element.thumbnail_url.clone(),
        images: if listed {
            Vec::new()
        } else {
            element
                .images
                .iter()
                .map(|image| ElementImage {
                    url: image.url.clone(),
                    media: image.media_id.as_ref().map(|id| ServerMedia {
                        id: id.clone(),
                        url: image.url.clone(),
                        owner: owner.map(str::to_owned),
                    }),
                })
                .collect()
        },
        status: element.status.clone(),
    }
}

impl HiggsfieldMcpProvider {
    /// Reuses an element's media ID or a character sheet's job ID only for the account that listed it.
    pub(crate) fn reused_asset(&self, media: &ServerMedia) -> Option<RemoteAsset> {
        (media.owner == self.owner).then(|| RemoteAsset {
            url: media.url.clone(),
            id: Some(media.id.clone()),
        })
    }
}

#[async_trait]
impl AccountElements for HiggsfieldMcpProvider {
    fn owner(&self) -> Option<String> {
        self.owner.clone()
    }

    async fn elements(&self, after: Option<HistoryCursor>) -> Result<AccountElementPage, PortError> {
        let cursor = after.as_ref().and_then(history_mapping::cursor_value);
        let page = self
            .account
            .reference_elements(cursor.as_ref(), ELEMENT_PAGE_SIZE)
            .await
            .map_err(port)?;
        Ok(AccountElementPage {
            items: page
                .items
                .iter()
                .filter(|element| is_usable(element))
                .map(|listed| element(listed, self.owner.as_deref(), true))
                .collect(),
            next_cursor: page_cursor(page.items.is_empty(), page.next_cursor.as_ref()),
        })
    }

    /// `get` of an element, for all its images.
    async fn element_images(&self, listed: AccountElement) -> Result<AccountElement, PortError> {
        let full = self.account.reference_element(&listed.id).await.map_err(port)?;
        Ok(element(&full, self.owner.as_deref(), false))
    }

    /// `show_reference_elements` `create` with the uploads as `media_input`. An upload without a media ID, or not on
    /// https (the tool rejects other URLs), is refused before anything is sent.
    async fn create_element(
        &self,
        name: String,
        category: ElementCategory,
        images: Vec<RemoteAsset>,
    ) -> Result<AccountElement, PortError> {
        let uploads = images
            .into_iter()
            .map(|asset| match asset.id {
                Some(id) if is_https(&asset.url) => Ok(UploadedMedia { id, url: asset.url }),
                _ => Err(unexpected_upload()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if uploads.is_empty() {
            return Err(validation(None, "An element needs at least one image."));
        }
        let created = self
            .account
            .create_reference_element(Some(&name), Some(reference_category(category)), &uploads)
            .await
            .map_err(port)?;
        Ok(element(&created, self.owner.as_deref(), false))
    }
}
