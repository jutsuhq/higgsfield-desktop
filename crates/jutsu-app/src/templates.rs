//! Templates (`TemplateLibrary.swift`, `JobTemplates.swift`, the Templates menu and sheet): saving the Create form as
//! a template with copies of its images, the list, applying one, renaming and deleting.

use std::sync::Arc;

use jutsu_core::{AudioOption, JobTemplate, LocalImage, TemplateKind, VideoModel};
use jutsu_store::{FileLayout, Store};
use serde::Serialize;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::create::form::{audio_title, mode_short_title};
use crate::create::{CreateService, Notice};
use crate::images::form::kind_title;
use crate::images::service::{CreateSide, ImageService};
use crate::queue::format::resolution_title;

/// The longest name, in characters.
pub const MAXIMUM_LENGTH: usize = 60;
/// The name of a template saved without one.
pub const FALLBACK: &str = "Untitled template";

/// "Restyle · rainy neon street": `prefix` (the mode or the kind) and the start of the prompt's first line.
pub fn suggested_name(prefix: &str, prompt: &str) -> String {
    let first_line = prompt.lines().next().unwrap_or_default().trim();
    if first_line.is_empty() {
        return prefix.to_owned();
    }
    truncated(&format!("{prefix} · {first_line}"))
}

/// `proposed` trimmed and cut to length; when another template already has it (ignoring case), " 2", " 3"… is
/// appended. `existing` must not contain the name of the template being renamed.
pub fn unique_name(proposed: &str, existing: &[String]) -> String {
    let trimmed = proposed.trim();
    let base = truncated(if trimmed.is_empty() { FALLBACK } else { trimmed });
    let taken: Vec<String> = existing.iter().map(|name| name.to_lowercase()).collect();
    if !taken.contains(&base.to_lowercase()) {
        return base;
    }
    let mut number = 2;
    while taken.contains(&format!("{base} {number}").to_lowercase()) {
        number += 1;
    }
    format!("{base} {number}")
}

fn truncated(name: &str) -> String {
    if name.chars().count() <= MAXIMUM_LENGTH {
        return name.to_owned();
    }
    let head: String = name.chars().take(MAXIMUM_LENGTH - 1).collect();
    format!("{}…", head.trim())
}

/// "Motion · Seedance 2.5 · 2 images · 720p · Mute" under the template's name; Genjutsu isn't named. An image
/// template reads "Image · Soul 2 · 16:9 · 4 variants".
pub fn summary(template: &JobTemplate) -> String {
    if template.kind == TemplateKind::Image {
        let mut parts = vec![kind_title(TemplateKind::Image).to_owned()];
        parts.extend(template.image_model_title.clone());
        parts.extend(template.image_aspect_ratio.clone());
        if template.image_count > 1 {
            parts.push(format!("{} variants", template.image_count));
        } else if !template.references.is_empty() {
            parts.push(images_text(template.references.len()));
        }
        return parts.join(" · ");
    }
    let mut parts = vec![mode_short_title(template.mode).to_owned()];
    if template.model != VideoModel::GENJUTSU {
        parts.push(template.model.title());
    }
    if template.references_are_frames {
        parts.push(
            if template.references.len() > 1 {
                "Start and end frames"
            } else {
                "Start frame"
            }
            .to_owned(),
        );
    } else if !template.references.is_empty() {
        parts.push(images_text(template.references.len()));
    }
    parts.push(resolution_title(template.resolution, &template.model));
    if template.audio != AudioOption::Original {
        parts.push(audio_title(template.audio).to_owned());
    }
    parts.join(" · ")
}

/// "1 image", "2 images": the string catalog's plural variations.
fn images_text(count: usize) -> String {
    if count == 1 {
        "1 image".to_owned()
    } else {
        format!("{count} images")
    }
}

/// A failed save, for an alert.
pub fn not_saved(error: &TemplateError) -> Notice {
    Notice {
        title: "Can't save the template".to_owned(),
        detail: match error {
            TemplateError::UnreadableImage => {
                "A reference image can't be read. Add it again, then save the template.".to_owned()
            }
            TemplateError::Store(message) | TemplateError::NotFound(message) => message.clone(),
        },
    }
}

/// Why a template couldn't be saved, changed or opened.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("A reference image can't be read. Add it again, then save the template.")]
    UnreadableImage,
    #[error("{0}")]
    Store(String),
    #[error("{0}")]
    NotFound(String),
}

/// One row of the Templates menu and sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateRow {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub side: CreateSide,
}

/// The stored templates and the copies of their images.
pub trait TemplateStore: Send + Sync {
    fn names(&self, excluding: Option<Uuid>) -> futures::future::BoxFuture<'_, Result<Vec<String>, String>>;
    fn add<'a>(&'a self, template: &'a JobTemplate) -> futures::future::BoxFuture<'a, Result<(), String>>;
    fn rename<'a>(&'a self, id: Uuid, name: &'a str) -> futures::future::BoxFuture<'a, Result<(), String>>;
    fn delete(&self, id: Uuid) -> futures::future::BoxFuture<'_, Result<(), String>>;
    /// Copies the files of `references` into the template's own folder; nothing is left behind when one fails.
    fn copy_images(&self, references: &[LocalImage], id: Uuid) -> Result<Vec<url::Url>, String>;
    /// Deletes the template's copies; a failure is only logged.
    fn remove_images(&self, id: Uuid);
}

/// Saving, renaming and deleting templates, with the copies of their reference images kept in step.
pub struct TemplateLibrary<'a> {
    pub store: &'a dyn TemplateStore,
}

impl TemplateLibrary<'_> {
    /// Saves `draft` under a unique name, with its reference images copied into the template's own folder. When the
    /// template can't be stored, the copies are removed again.
    pub async fn save(&self, draft: &JobTemplate) -> Result<JobTemplate, TemplateError> {
        let mut template = draft.clone();
        let names = self.store.names(None).await.unwrap_or_default();
        template.name = unique_name(&draft.name, &names);
        let copies = self
            .store
            .copy_images(&draft.references, template.id)
            .map_err(|_| TemplateError::UnreadableImage)?;
        // A generated image keeps its prompt.
        template.references = copies
            .into_iter()
            .zip(&draft.references)
            .map(|(copy, original)| {
                let mut image = LocalImage::new(copy);
                image.prompt.clone_from(&original.prompt);
                image
            })
            .collect();
        if let Err(error) = self.store.add(&template).await {
            self.store.remove_images(template.id);
            return Err(TemplateError::Store(error));
        }
        Ok(template)
    }

    /// Renames to a unique name; returns the name it got.
    pub async fn rename(&self, id: Uuid, name: &str) -> Result<String, TemplateError> {
        let names = self.store.names(Some(id)).await.unwrap_or_default();
        let unique = unique_name(name, &names);
        self.store
            .rename(id, &unique)
            .await
            .map_err(TemplateError::Store)?;
        Ok(unique)
    }

    /// Deletes the template, then its copied images; they stay when the template can't be deleted.
    pub async fn delete(&self, id: Uuid) -> Result<(), TemplateError> {
        self.store.delete(id).await.map_err(TemplateError::Store)?;
        self.store.remove_images(id);
        Ok(())
    }
}

/// The app's store and file layout.
pub struct StoredTemplates {
    pub store: Arc<Store>,
    pub layout: FileLayout,
}

impl TemplateStore for StoredTemplates {
    fn names(&self, excluding: Option<Uuid>) -> futures::future::BoxFuture<'_, Result<Vec<String>, String>> {
        Box::pin(async move {
            self.store
                .template_names(excluding)
                .await
                .map_err(|error| error.to_string())
        })
    }
    fn add<'a>(&'a self, template: &'a JobTemplate) -> futures::future::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.store
                .add_template(template)
                .await
                .map_err(|error| error.to_string())
        })
    }
    fn rename<'a>(&'a self, id: Uuid, name: &'a str) -> futures::future::BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.store
                .rename_template(id, name)
                .await
                .map_err(|error| error.to_string())
        })
    }
    fn delete(&self, id: Uuid) -> futures::future::BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.store
                .delete_template(id)
                .await
                .map_err(|error| error.to_string())
        })
    }
    fn copy_images(&self, references: &[LocalImage], id: Uuid) -> Result<Vec<url::Url>, String> {
        let sources: Vec<url::Url> = references.iter().map(|image| image.file_url.clone()).collect();
        self.layout
            .copy_template_images(&sources, id)
            .map_err(|error| error.to_string())
    }
    fn remove_images(&self, id: Uuid) {
        if let Err(error) = self.layout.remove_template_images(id) {
            tracing::warn!(%error, "can't delete a template's images");
        }
    }
}

/// The Templates menu and sheet: the list, Save as Template, Use, Rename and Delete.
pub struct TemplateService {
    stored: StoredTemplates,
    create: Arc<CreateService>,
    images: Arc<ImageService>,
}

impl std::fmt::Debug for TemplateService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("TemplateService").finish_non_exhaustive()
    }
}

impl TemplateService {
    pub fn new(
        store: Arc<Store>,
        layout: FileLayout,
        create: Arc<CreateService>,
        images: Arc<ImageService>,
    ) -> Self {
        Self {
            stored: StoredTemplates { store, layout },
            create,
            images,
        }
    }

    fn library(&self) -> TemplateLibrary<'_> {
        TemplateLibrary { store: &self.stored }
    }

    /// The templates, those of `side` first.
    pub async fn list(&self, side: CreateSide) -> Result<Vec<TemplateRow>, TemplateError> {
        let templates = self
            .stored
            .store
            .templates()
            .await
            .map_err(|error| TemplateError::Store(error.to_string()))?;
        let rows = templates.iter().map(|template| TemplateRow {
            id: template.id.to_string(),
            name: template.name.clone(),
            summary: summary(template),
            side: template.kind.into(),
        });
        let (mut first, rest): (Vec<TemplateRow>, Vec<TemplateRow>) = rows.partition(|row| row.side == side);
        first.extend(rest);
        Ok(first)
    }

    /// What the name field of Save as Template starts with.
    pub fn suggested_name(&self, side: CreateSide) -> String {
        match side {
            CreateSide::Video => {
                let form = self.create.form();
                suggested_name(mode_short_title(form.mode()), &form.prompt)
            }
            CreateSide::Image => self.images.suggested_template_name(),
        }
    }

    /// Saves the form of `side` as a template named `name` (made unique); returns the name it got.
    pub async fn save(&self, side: CreateSide, name: &str) -> Result<String, TemplateError> {
        let draft = match side {
            CreateSide::Video => {
                self.create
                    .form()
                    .make_template(name, Uuid::new_v4(), OffsetDateTime::now_utc())
            }
            CreateSide::Image => self.images.make_template(name),
        };
        Ok(self.library().save(&draft).await?.name)
    }

    /// What the name field of a result's Save as Template starts with (`TemplateNaming.suggestedName`).
    pub fn suggested_name_for(spec: &jutsu_core::JobSpec) -> String {
        suggested_name(mode_short_title(spec.mode), &spec.prompt)
    }

    /// A result's Save as Template…: its inputs as a template named `name` (made unique); returns the name it got.
    pub async fn save_spec(&self, spec: &jutsu_core::JobSpec, name: &str) -> Result<String, TemplateError> {
        let draft = JobTemplate::from_spec(Uuid::new_v4(), name, OffsetDateTime::now_utc(), spec);
        Ok(self.library().save(&draft).await?.name)
    }

    pub async fn rename(&self, id: Uuid, name: &str) -> Result<String, TemplateError> {
        self.library().rename(id, name).await
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), TemplateError> {
        self.library().delete(id).await
    }

    /// Fills the Create form of the template's kind from it, and shows that side. A model the connection doesn't
    /// offer is said in the notice.
    pub async fn apply(&self, id: Uuid) -> Result<Option<Notice>, TemplateError> {
        let template = self
            .stored
            .store
            .template(id)
            .await
            .map_err(|error| TemplateError::Store(error.to_string()))?
            .ok_or_else(|| TemplateError::NotFound("This template no longer exists.".to_owned()))?;
        Ok(match template.kind {
            TemplateKind::Image => self.images.apply_template(&template),
            TemplateKind::Video => {
                self.images.set_side(CreateSide::Video);
                self.create.apply_template(&template).await
            }
        })
    }
}

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
