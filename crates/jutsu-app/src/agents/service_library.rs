//! `library_search`, `library_get`, `library_mark` (`AgentService+Library.swift`) and `library_download`
//! (`AgentService+Download.swift`): the Library's own code paths (its filters, culling, star and From Higgsfield's
//! download), never anything that deletes or trashes, and never anything that spends.

use jutsu_agent::{
    AgentCulling, AgentDownload, AgentDownloadStatus, AgentDownloads, AgentError, AgentLibraryDetailsPage,
    AgentLibraryPage, AgentMarked, AgentMarkedItem, ErrorCode, LibraryDownloadCommand, LibraryGetCommand,
    LibraryMarkCommand, LibrarySearchCommand,
};
use jutsu_core::Material;
use uuid::Uuid;

use super::{AgentService, app_id, wire_id};
use crate::agents::job::mode_named;
use crate::agents::library_detail::{agent_details, details};
use crate::agents::library_search::{
    AgentLibraryDownload, AgentLibraryMarks, AgentLibraryQuery, agent_item, culling, library_kind, search,
    verdict,
};
use crate::library::CullingAction;

impl AgentService {
    // MARK: library_search

    pub(super) async fn library_search(
        &self,
        fields: &LibrarySearchCommand,
    ) -> Result<AgentLibraryPage, AgentError> {
        let query = AgentLibraryQuery {
            text: fields.query.clone().unwrap_or_default(),
            kind: library_kind(fields.kind),
            starred: fields.starred,
            verdict: fields.verdict.map(culling),
            include_rejected: fields.include_rejected.unwrap_or(false),
            model: fields.model.clone(),
            mode: fields.mode.as_deref().and_then(mode_named),
            from: fields.from.map(|from| from.0),
            to: fields.to.map(|to| to.0),
            limit: usize::try_from(fields.limit.unwrap_or(LibrarySearchCommand::DEFAULT_LIMIT)).unwrap_or(1),
            cursor: fields.cursor.clone(),
            include_higgsfield: fields.include_higgsfield.unwrap_or(false),
        };
        // From Higgsfield's own refresh: it reads the history, never spends, and does nothing without an account.
        if fields.refresh_higgsfield == Some(true) {
            self.environment.library.load_history(true).await;
        }
        let (hits, next) = search(&self.materials().await?, &query);
        Ok(AgentLibraryPage {
            items: hits.iter().map(agent_item).collect(),
            next_cursor: next,
        })
    }

    // MARK: library_get

    pub(super) async fn library_get(
        &self,
        fields: &LibraryGetCommand,
    ) -> Result<AgentLibraryDetailsPage, AgentError> {
        let ids: Vec<Uuid> = fields.ids.iter().copied().map(app_id).collect();
        let (found, missing) = details(&ids, &self.materials().await?);
        Ok(AgentLibraryDetailsPage::new(
            found.iter().map(agent_details).collect(),
            missing.into_iter().map(wire_id).collect(),
        ))
    }

    // MARK: library_mark

    pub(super) async fn library_mark(&self, fields: &LibraryMarkCommand) -> Result<AgentMarked, AgentError> {
        let ids: Vec<Uuid> = fields.ids.iter().copied().map(app_id).collect();
        let action = fields.verdict.map(|verdict| match verdict {
            AgentCulling::Keep => CullingAction::Keep,
            AgentCulling::Reject => CullingAction::Reject,
            AgentCulling::Clear => CullingAction::Clear,
        });
        let marks = AgentLibraryMarks::new(&ids, action, fields.star, &self.materials().await?);
        if let Some(missing) = marks.missing.first() {
            return Err(AgentError::new(
                ErrorCode::NOT_FOUND,
                format!(
                    "There's no Library item {} with a file on this computer; nothing was changed.",
                    wire_id(*missing)
                ),
                false,
                Some("Call library_search for the item IDs."),
            ));
        }
        if !marks.culling.is_empty()
            && let Err(error) = self.environment.store.set_culling(&marks.culling).await
        {
            tracing::error!(%error, "can't mark Library items");
            return Err(AgentError::new(
                ErrorCode::FAILED,
                format!("The marks couldn't all be saved: {error}"),
                true,
                Some("Send the same library_mark again; what was saved stays as it is."),
            ));
        }
        if let Some(star) = fields.star {
            let starred: Vec<Uuid> = marks
                .videos
                .iter()
                .map(crate::library::item::LibraryItem::id)
                .chain(marks.images.iter().copied())
                .collect();
            // The Library's own star, which likes the result on Higgsfield too; the images only once every video's
            // star was saved. A star that couldn't be saved, or a like Higgsfield refused, fails the call.
            if !starred.is_empty()
                && let Err(error) = self.environment.library.star(&starred, star).await
            {
                tracing::error!(%error, "can't mark Library items");
                return Err(AgentError::new(
                    ErrorCode::FAILED,
                    format!("The marks couldn't all be saved: {error}"),
                    true,
                    Some("Send the same library_mark again; what was saved stays as it is."),
                ));
            }
        }
        Ok(AgentMarked {
            items: marks
                .results
                .iter()
                .map(|marked| AgentMarkedItem {
                    id: wire_id(marked.id),
                    verdict: verdict(marked.culling),
                    starred: marked.is_starred,
                })
                .collect(),
            changed: i64::try_from(marks.changes()).unwrap_or(i64::MAX),
        })
    }

    // MARK: library_download

    /// From Higgsfield's download, one result at a time: a video through the history import, an image saved into the
    /// output folder's Images. Both keep the item's ID; neither spends.
    pub(super) async fn library_download(
        &self,
        fields: &LibraryDownloadCommand,
    ) -> Result<AgentDownloads, AgentError> {
        let ids: Vec<Uuid> = fields.ids.iter().copied().map(app_id).collect();
        let materials = self.materials().await?;
        let plan = AgentLibraryDownload::plan(&ids, &materials);
        if let Some(index) = plan
            .iter()
            .position(|target| *target == AgentLibraryDownload::Missing)
        {
            return Err(AgentError::new(
                ErrorCode::NOT_FOUND,
                format!(
                    "There's no Library item {}; nothing was downloaded.",
                    fields.ids.get(index).map(ToString::to_string).unwrap_or_default()
                ),
                false,
                Some("Call library_search with include_higgsfield for the item IDs."),
            ));
        }
        let mut items = Vec::with_capacity(ids.len());
        for (id, target) in ids.iter().zip(plan) {
            let item = match target {
                AgentLibraryDownload::Local(file) => {
                    download(*id, AgentDownloadStatus::AlreadyHere, Some(file), None)
                }
                AgentLibraryDownload::Video(_) | AgentLibraryDownload::Image(_) => {
                    let is_image = matches!(target, AgentLibraryDownload::Image(_));
                    let Some(material) = materials.iter().find(|material| material.id == *id) else {
                        continue;
                    };
                    match self.environment.library.download_quietly(material).await {
                        Ok(()) => {
                            let file = self.local_file(*id).await;
                            download(*id, AgentDownloadStatus::Downloaded, file, None)
                        }
                        Err(error) => {
                            let message = if is_image {
                                "The image couldn't be saved.".to_owned()
                            } else {
                                error
                            };
                            download(*id, AgentDownloadStatus::Failed, None, Some(message))
                        }
                    }
                }
                AgentLibraryDownload::Missing => continue,
            };
            items.push(item);
        }
        tracing::info!(
            downloaded = items
                .iter()
                .filter(|item| item.status == AgentDownloadStatus::Downloaded)
                .count(),
            "an agent downloaded results"
        );
        Ok(AgentDownloads { items })
    }

    /// The file of `id` on this computer once it is downloaded.
    async fn local_file(&self, id: Uuid) -> Option<std::path::PathBuf> {
        let materials = self.environment.store.materials().await.ok()?;
        match AgentLibraryDownload::plan(&[id], &materials).pop() {
            Some(AgentLibraryDownload::Local(file)) => Some(file),
            _ => None,
        }
    }

    // MARK: Helpers

    pub(super) async fn materials(&self) -> Result<Vec<Material>, AgentError> {
        self.environment.store.materials().await.map_err(|error| {
            tracing::error!(%error, "can't read the library");
            AgentError::new(
                ErrorCode::FAILED,
                "Jutsu Studio can't read its library right now.",
                true,
                Some("Try again in a moment."),
            )
        })
    }
}

fn download(
    id: Uuid,
    status: AgentDownloadStatus,
    file: Option<std::path::PathBuf>,
    message: Option<String>,
) -> AgentDownload {
    AgentDownload {
        id: wire_id(id),
        status,
        file: file.map(|file| file.to_string_lossy().into_owned()),
        message,
    }
}
