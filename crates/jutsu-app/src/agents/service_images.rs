//! `create_image` (`AgentService+Image.swift`): images made as the Image side of Create makes them
//! (`AgentImageOrder`), never through its visible form, and sent through Generate's own runs behind the agents'
//! spending guard (`AgentImageQueue`). `elements` and `create_element` (`AgentService+Elements.swift`), and the
//! elements `create_video`, `quote` and `create_image` take by name.

use std::path::PathBuf;
use std::sync::Arc;

use jutsu_agent::library::MAX_INPUTS;
use jutsu_agent::{
    AgentActed, AgentCreated, AgentElementsPage, AgentError, AgentJob, AgentJobAction, AgentReference,
    CreateElementCommand, CreateImageCommand, ElementsCommand, ErrorCode, ImageModel, Style, Voice,
};
use jutsu_core::{
    AccountElement, AccountElements, AgentOrigin, ElementCategory, ImageModelInput, LocalImage, MaterialKind,
};
use uuid::Uuid;

use super::{AgentService, check_unit, file_url, not_connected, refusals, wire_id};
use crate::agents::budget::money;
use crate::agents::element_lookup::{self, ElementImages, Found};
use crate::agents::element_making::{AgentElementMaking, agent_element};
use crate::agents::image_record::{AgentImageOrder, AgentImageRecord, AgentImageState, image_job};
use crate::agents::request_gate::AgentRaiseOutcome;
use crate::images::form::{OFFERS_IMAGE_GENERATION, generation_blocker};

/// No `max_cost` raise: a request sent again with another one is refused as a conflict.
pub(super) type NoRaise =
    fn(Vec<Uuid>) -> std::future::Ready<Result<AgentRaiseOutcome<AgentCreated>, AgentError>>;

impl AgentService {
    // MARK: create_image

    pub(super) async fn create_image(
        &self,
        fields: CreateImageCommand,
        client: Option<String>,
    ) -> Result<AgentCreated, AgentError> {
        let files = fields.image.inputs.clone();
        let me = self.strong()?;
        let queued = fields.clone();
        let request_id = &fields.request_id;
        let created = self
            .requests
            .run(
                &fields.request_id,
                &fields.fingerprint,
                None,
                refusals("create_image", "never taken"),
                || self.discard(&files),
                |ids| async move {
                    let mut jobs = Vec::new();
                    for id in ids {
                        if let Some(job) = self.image_job(id).await {
                            jobs.push(job);
                        }
                    }
                    Ok((!jobs.is_empty()).then(|| AgentCreated::new(request_id, true, jobs)))
                },
                None::<NoRaise>,
                move || async move { me.queue_image(&queued, client).await },
            )
            .await;
        self.sweep_inbox().await;
        let created = created?;
        tracing::info!(known = created.known, "an agent asked for images");
        Ok(created)
    }

    /// Records the request with its run's ID, then hands it to the queue.
    async fn queue_image(
        &self,
        fields: &CreateImageCommand,
        client: Option<String>,
    ) -> Result<AgentCreated, AgentError> {
        let images = &self.environment.images;
        let (Some(kind), Some((provider, upload)), Some(queue)) =
            (self.kind(), images.agent_provider(), images.agent_images())
        else {
            return Err(not_connected());
        };
        if let Some(blocker) = generation_blocker(kind, OFFERS_IMAGE_GENERATION) {
            return Err(AgentError::new(
                ErrorCode::NOT_APPLICABLE,
                blocker,
                true,
                Some("Try again in a moment."),
            ));
        }
        images.read_models_if_needed().await;
        if let Some(note) = images.generator().form().agent_models_note() {
            return Err(AgentError::new(
                ErrorCode::NOT_APPLICABLE,
                "The image models of this connection aren't read yet.",
                true,
                Some(&note),
            ));
        }
        check_unit(&fields.max_cost, kind)?;
        let mut files = Vec::new();
        for path in &fields.image.inputs {
            files.push(LocalImage::new(file_url(&self.inbox_file(path)?)?));
        }
        let elements = self.element_images(&fields.image.elements).await?;
        let image = &fields.image;
        let order = AgentImageOrder {
            model: image.model.clone(),
            prompt: image.prompt.clone(),
            aspect_ratio: image.aspect_ratio.clone(),
            count: image.count,
            inputs: elements
                .iter()
                .flat_map(|element| element.images.iter().cloned())
                .chain(files.iter().cloned())
                .collect(),
            title: image.title.clone(),
        };
        let id = Uuid::new_v4();
        self.record_ids(&fields.request_id, &fields.fingerprint, None, vec![id])?;
        let origin = AgentOrigin::new(client, fields.request_id.clone(), money(fields.max_cost));
        queue
            .create(&order, id, origin, provider, upload)
            .map_err(|refusal| {
                if refusal.is_not_saved {
                    AgentError::new(
                        ErrorCode::FAILED,
                        refusal.message,
                        true,
                        Some("Send the request again."),
                    )
                } else {
                    AgentError::invalid_request(refusal.message)
                }
            })?;
        let jobs = self.image_job(id).await.into_iter().collect();
        let mut created = AgentCreated::new(&fields.request_id, false, jobs);
        created.references = references(&elements, &files);
        Ok(created)
    }

    /// An agent's image request as `jobs` lists it; `None` when there is no such request.
    pub(super) async fn image_job(&self, id: Uuid) -> Option<AgentJob> {
        let record = self.environment.images.agent_images()?.record(id)?;
        let files = self.image_files(&record).await;
        Some(image_job(&record, &files))
    }

    /// Whether `jobs`' wait is over for an image request: it ended with its images saved, or waits for the person,
    /// or failed or was cancelled. One the app doesn't know counts as over.
    pub(super) fn image_is_settled(&self, id: Uuid) -> bool {
        let Some(queue) = self.environment.images.agent_images() else {
            return true;
        };
        let Some(record) = queue.record(id) else {
            return true;
        };
        (record.is_over() && !queue.files(id).is_empty())
            || matches!(
                record.state,
                AgentImageState::Held | AgentImageState::Failed | AgentImageState::Canceled
            )
    }

    /// The agents' image requests for a `jobs` page: when each was made, and its ID.
    pub(super) fn image_entries(&self) -> Vec<(time::OffsetDateTime, Uuid)> {
        self.environment
            .images
            .agent_images()
            .and_then(|queue| queue.records().ok())
            .unwrap_or_default()
            .iter()
            .map(|record| (record.created_at, record.id))
            .collect()
    }

    /// The images a finished request saved in Library › Images: this session's results, else the catalog's images
    /// of the request (`variant_group_id`).
    async fn image_files(&self, record: &AgentImageRecord) -> Vec<PathBuf> {
        let session = self
            .environment
            .images
            .agent_images()
            .map(|queue| queue.files(record.id))
            .unwrap_or_default();
        if !session.is_empty() || record.state != AgentImageState::Finished {
            return session;
        }
        let materials = self.environment.store.materials().await.unwrap_or_default();
        materials
            .iter()
            .filter(|material| {
                material.kind == MaterialKind::Image
                    && material.variant_group_id == Some(record.id)
                    && !material.is_hidden
            })
            .filter_map(|material| material.current.file_url.as_ref()?.to_file_path().ok())
            .collect()
    }

    /// `job_action` on an image request: cancel only. A request runs as soon as it is taken, and a retry would be a
    /// new paid request, which only `create_image` with a new `request_id` makes.
    pub(super) async fn act_on_image(
        &self,
        action: AgentJobAction,
        id: Uuid,
    ) -> Result<AgentActed, AgentError> {
        match action {
            AgentJobAction::Cancel => {
                let canceled = self
                    .environment
                    .images
                    .agent_images()
                    .is_some_and(|queue| queue.cancel(id));
                if !canceled {
                    return Err(AgentError::new(
                        ErrorCode::NOT_APPLICABLE,
                        "The image request is already over.",
                        false,
                        Some("Call jobs to see how it ended."),
                    ));
                }
                Ok(AgentActed {
                    action,
                    job: self.image_job(id).await,
                })
            }
            AgentJobAction::Retry => Err(AgentError::new(
                ErrorCode::NOT_APPLICABLE,
                "An image request isn't retried.",
                false,
                Some("Send create_image again with a new request_id."),
            )),
            AgentJobAction::RunNext => Err(AgentError::new(
                ErrorCode::NOT_APPLICABLE,
                "Image requests don't wait in the queue.",
                false,
                Some("Call jobs to follow it."),
            )),
            AgentJobAction::PauseQueue | AgentJobAction::ResumeQueue => Err(AgentError::bad_request(
                format!("{} takes no job", action.as_str()),
            )),
        }
    }

    // MARK: elements

    pub(super) async fn elements(&self, fields: &ElementsCommand) -> Result<AgentElementsPage, AgentError> {
        let source = self.element_source()?;
        let category = fields.category.as_deref().and_then(ElementCategory::from_raw);
        let limit = fields.limit.unwrap_or(ElementsCommand::DEFAULT_LIMIT);
        let (items, next) = reading(
            element_lookup::page(
                source.as_ref(),
                fields.query.as_deref().unwrap_or_default(),
                category,
                usize::try_from(limit).unwrap_or(1),
                fields.cursor.as_deref(),
            )
            .await,
        )?;
        Ok(AgentElementsPage {
            items: items
                .iter()
                .map(|element| {
                    let mut listed = agent_element(element);
                    listed.status = None;
                    listed
                })
                .collect(),
            next_cursor: next,
        })
    }

    /// The elements named, each with all its images, in the order named. Every page is read first, up to
    /// `MAX_PAGES`, so a name two elements share is refused; an account with more pages than that can't be checked,
    /// and a name only once in what was read is refused too.
    pub(super) async fn named_elements(&self, names: &[String]) -> Result<Vec<AccountElement>, AgentError> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let source = self.element_source()?;
        let (listed, is_complete) = reading(element_lookup::all(source.as_ref()).await)?;
        let mut found = Vec::new();
        for name in names {
            match element_lookup::find(name, &listed, is_complete) {
                Found::Element(element) if element.images.is_empty() => {
                    found.push(reading(source.element_images(*element).await)?);
                }
                Found::Element(element) => found.push(*element),
                Found::Ambiguous(count) => {
                    return Err(AgentError::invalid_request(format!(
                        "{count} elements are named {name}; ask the person to rename one, or name another element."
                    )));
                }
                Found::Unchecked => {
                    return Err(AgentError::invalid_request(format!(
                        "The account has too many elements to check which one is named {name}; ask the person to \
                         remove elements they no longer use, or send the element's images as files instead."
                    )));
                }
                Found::Missing => {
                    return Err(AgentError::new(
                        ErrorCode::NOT_FOUND,
                        format!("The account has no element named {name}."),
                        false,
                        Some("Call elements for the names."),
                    ));
                }
            }
        }
        Ok(found)
    }

    /// The images of the elements named, downloaded once each as From Higgsfield adds them.
    pub(super) async fn element_images(&self, names: &[String]) -> Result<Vec<ElementImages>, AgentError> {
        let mut result = Vec::new();
        for element in self.named_elements(names).await? {
            if element.images.is_empty() {
                return Err(AgentError::invalid_request(format!(
                    "The element {} has no images to use.",
                    element.name
                )));
            }
            let mut images = Vec::new();
            for index in 0..element.images.len() {
                let Some(image) = self.environment.create.agent_element_image(&element, index).await else {
                    return Err(account_failure("the element's images can't be downloaded"));
                };
                images.push(image);
            }
            result.push(ElementImages {
                name: element.name.clone(),
                images,
            });
        }
        Ok(result)
    }

    // MARK: create_element

    pub(super) async fn create_element(
        &self,
        fields: &CreateElementCommand,
        client: Option<String>,
    ) -> Result<AgentCreated, AgentError> {
        let source = self.element_source()?;
        let Some(upload) = self.environment.create.agent_element_uploader() else {
            return Err(not_connected());
        };
        let making = AgentElementMaking {
            requests: self.requests.clone(),
            source,
            upload,
            refusals: refusals("create_element", "its element was never made"),
        };
        let me = self.strong()?;
        let paths = fields.images.clone();
        let created = making
            .create(
                fields,
                move || {
                    let mut images = Vec::new();
                    for path in &paths {
                        images.push(LocalImage::new(file_url(&me.inbox_file(path)?)?));
                    }
                    Ok(images)
                },
                || self.discard(&fields.images),
            )
            .await?;
        tracing::info!(
            client = client.as_deref().unwrap_or("unnamed"),
            known = created.known,
            "an agent made an element"
        );
        Ok(created)
    }

    fn element_source(&self) -> Result<Arc<dyn AccountElements>, AgentError> {
        self.environment.create.agent_elements().ok_or_else(|| {
            AgentError::new(
                ErrorCode::NOT_APPLICABLE,
                "Reference elements need a Higgsfield account connection.",
                false,
                Some("Ask the person to connect a Higgsfield account in Settings."),
            )
        })
    }
}

/// The references answer of a request: every image in the order sent, with its tag.
pub(super) fn references(elements: &[ElementImages], files: &[LocalImage]) -> Option<Vec<AgentReference>> {
    let references = element_lookup::references(elements, files);
    (!references.is_empty()).then(|| {
        references
            .into_iter()
            .map(|reference| AgentReference {
                tag: reference.tag,
                element: reference.element,
                file: reference
                    .image
                    .file_url
                    .to_file_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
            })
            .collect()
    })
}

/// A call to the account, its failure as an agent error.
fn reading<T>(result: Result<T, jutsu_core::PortError>) -> Result<T, AgentError> {
    result.map_err(|error| {
        tracing::error!(%error, "can't read the account's elements");
        account_failure(&error.to_string())
    })
}

fn account_failure(reason: &str) -> AgentError {
    AgentError::new(
        ErrorCode::FAILED,
        format!("Higgsfield didn't return the account's elements: {reason}"),
        true,
        Some("Try again in a moment."),
    )
}

impl AgentService {
    // MARK: capabilities image_models, styles, voices

    /// The connection's image models as `create_image` takes them, read first while its own list isn't in, and what
    /// to do while it still isn't.
    pub(super) async fn image_models(&self) -> (Vec<ImageModel>, Option<String>) {
        let images = &self.environment.images;
        images.read_models_if_needed().await;
        let form = images.generator().form();
        (
            form.models().iter().map(image_model).collect(),
            form.agent_models_note(),
        )
    }

    /// Restyle's styles from the catalog Create last read, never a network call.
    pub(super) fn styles(&self) -> Vec<Style> {
        crate::create::presets::cached_presets(&self.environment.layout.preset_cache_folder())
            .unwrap_or_default()
            .into_iter()
            .map(|preset| Style {
                id: wire_id(preset.id),
                name: preset.name,
            })
            .collect()
    }

    /// The voices Change Voice offers on an account; none when they can't be read.
    pub(super) async fn voices(&self) -> Vec<Voice> {
        self.environment
            .library
            .voices()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|row| Voice {
                id: row.choice.id,
                name: row.choice.name,
            })
            .collect()
    }
}

/// An image model of the connection as `create_image` takes it, its inputs within the protocol's bound
/// (`AgentCapabilities.ImageModel.init`).
pub(super) fn image_model(model: &jutsu_core::ImageModel) -> ImageModel {
    let input = match model.input {
        ImageModelInput::None => "none",
        ImageModelInput::Optional => "optional",
        ImageModelInput::Required => "required",
    };
    ImageModel {
        id: model.id.clone(),
        title: model.title.clone(),
        counts: model.variant_counts.clone(),
        aspect_ratios: model.aspect_ratios.clone(),
        input: input.to_owned(),
        max_inputs: model.input_limit().min(MAX_INPUTS),
        max_inputs_note: model.input_limit_is_bound().then(ImageModel::bound_note),
    }
}
