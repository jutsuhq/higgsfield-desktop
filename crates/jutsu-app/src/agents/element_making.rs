//! `create_element` (`AgentElementMaking.swift`): publishes an agent's images as one reference element of the
//! account, as Save to Higgsfield… does, once for its request ID. Higgsfield names the element only in its answer, so
//! the request is recorded before anything is sent under an ID of its own (`SENT_KIND`), and again under the
//! element's ID once Higgsfield answered (`KIND`): a request whose answer never came is never sent a second time.

use std::sync::Arc;

use futures::future::BoxFuture;
use jutsu_agent::{AgentCreated, AgentElement, AgentError, CreateElementCommand, ErrorCode};
use jutsu_core::{AccountElement, AccountElements, ElementCategory, LocalImage, PortError, RemoteAsset};
use time::OffsetDateTime;
use uuid::Uuid;

use super::request_gate::{AgentRequestGate, Refusals};
use crate::create::elements::SavedElement;

/// The `kind` of a request recorded with the element it made.
pub const KIND: &str = "element";
/// The `kind` of a request sent to Higgsfield whose answer isn't recorded.
pub const SENT_KIND: &str = "element_sent";

/// Uploads an image to the account as Save to Higgsfield… does.
pub type ElementUpload =
    Arc<dyn Fn(LocalImage) -> BoxFuture<'static, Result<RemoteAsset, PortError>> + Send + Sync>;

type Gate = AgentRequestGate<AgentCreated, AgentError>;
type NoRaise =
    fn(
        Vec<Uuid>,
    ) -> std::future::Ready<Result<super::request_gate::AgentRaiseOutcome<AgentCreated>, AgentError>>;

/// Makes the elements agents ask for.
pub struct AgentElementMaking {
    pub requests: Arc<Gate>,
    pub source: Arc<dyn AccountElements>,
    pub upload: ElementUpload,
    pub refusals: Refusals<AgentError>,
}

impl AgentElementMaking {
    /// The element the request makes, or made when it is known. `images` is read only for a new request: one already
    /// known sends none. `discard` is called once the request's files aren't needed, made or not.
    pub async fn create(
        &self,
        fields: &CreateElementCommand,
        images: impl FnOnce() -> Result<Vec<LocalImage>, AgentError> + Send + 'static,
        discard: impl Fn(),
    ) -> Result<AgentCreated, AgentError> {
        let making = Self {
            requests: self.requests.clone(),
            source: self.source.clone(),
            upload: self.upload.clone(),
            refusals: self.refusals.clone(),
        };
        let queued = fields.clone();
        let created = self
            .requests
            .run(
                &fields.request_id,
                &fields.fingerprint,
                None,
                self.refusals.clone(),
                &discard,
                |ids: Vec<Uuid>| async move {
                    match ids.first() {
                        Some(id) => self.made(*id, fields).await,
                        None => Ok(None),
                    }
                },
                None::<NoRaise>,
                move || async move { making.make(&queued, images()?).await },
            )
            .await;
        // Higgsfield keeps the images now.
        discard();
        created
    }

    async fn make(
        &self,
        fields: &CreateElementCommand,
        images: Vec<LocalImage>,
    ) -> Result<AgentCreated, AgentError> {
        let category = fields
            .category
            .as_deref()
            .and_then(ElementCategory::from_raw)
            .unwrap_or(ElementCategory::Character);
        self.record(fields, Uuid::new_v4(), SENT_KIND)?;
        let element = match self.publish(fields, category, images).await {
            Ok(element) => element,
            Err(error) => {
                tracing::error!(%error, "an agent's element wasn't made");
                return Err(AgentError::new(
                    ErrorCode::FAILED,
                    format!("Higgsfield didn't make the element: {error}"),
                    true,
                    Some(&format!(
                        "Call elements with query {} to check it wasn't made after all, then send the same \
                         create_element again.",
                        fields.name
                    )),
                ));
            }
        };
        if let Ok(id) = Uuid::parse_str(&element.id)
            && let Err(error) = self.record(fields, id, KIND)
        {
            tracing::error!(message = %error.message, "an agent's element can't be recorded");
        }
        let mut created = AgentCreated::new(&fields.request_id, false, Vec::new());
        created.element = Some(agent_element(&element));
        Ok(created)
    }

    async fn publish(
        &self,
        fields: &CreateElementCommand,
        category: ElementCategory,
        images: Vec<LocalImage>,
    ) -> Result<AccountElement, PortError> {
        let mut assets = Vec::with_capacity(images.len());
        for image in images {
            assets.push((self.upload)(image).await?);
        }
        self.source
            .create_element(SavedElement::name(&fields.name), category, assets)
            .await
    }

    /// The element a known request made, as the account has it now; `None` when the log names none.
    async fn made(
        &self,
        id: Uuid,
        fields: &CreateElementCommand,
    ) -> Result<Option<AgentCreated>, AgentError> {
        match self.requests.kind_of(id).as_deref() {
            Some(KIND) => {
                let asked = AccountElement {
                    id: id.to_string(),
                    name: fields.name.clone(),
                    category: None,
                    thumbnail_url: None,
                    images: Vec::new(),
                    status: None,
                };
                let element = self.source.element_images(asked).await.map_err(|error| {
                    AgentError::new(
                        ErrorCode::FAILED,
                        format!("Higgsfield didn't return the element: {error}"),
                        true,
                        Some("Try again in a moment."),
                    )
                })?;
                let mut created = AgentCreated::new(&fields.request_id, true, Vec::new());
                created.element = Some(agent_element(&element));
                Ok(Some(created))
            }
            Some(SENT_KIND) => Err(AgentError::new(
                ErrorCode::FAILED,
                "This request was sent to Higgsfield, but whether it made the element isn't known: its answer never \
                 came.",
                false,
                Some(&format!(
                    "Call elements with query {} in a minute; if it isn't there, send create_element with a new \
                     request_id.",
                    fields.name
                )),
            )),
            _ => Ok(None),
        }
    }

    fn record(&self, fields: &CreateElementCommand, id: Uuid, kind: &str) -> Result<(), AgentError> {
        self.requests
            .record(
                &fields.request_id,
                &fields.fingerprint,
                None,
                vec![id],
                OffsetDateTime::now_utc(),
                Some(kind),
            )
            .map_err(|_| {
                AgentError::new(
                    ErrorCode::FAILED,
                    "Jutsu Studio can't record the request, so nothing is sent.",
                    true,
                    Some("Ask the person to check the computer has room, then send the request again."),
                )
            })
    }
}

/// An element as `elements` and `create_element` answer it.
pub fn agent_element(element: &AccountElement) -> AgentElement {
    AgentElement {
        id: element.id.clone(),
        name: element.name.clone(),
        category: element.category.map(|category| category.raw_value().to_owned()),
        image_count: (!element.images.is_empty())
            .then(|| i64::try_from(element.images.len()).unwrap_or(i64::MAX)),
        status: element.status.clone(),
    }
}

#[cfg(test)]
#[path = "element_making_tests.rs"]
mod tests;
