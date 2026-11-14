//! Enhance on a result's page (`EnhanceSheet`): the sheet of one operation with its price, and queueing it as a new
//! job whose result joins the Library linked to this one.

use jutsu_core::{PixelSize, PostActionKind};
use jutsu_engine::EnqueueOptions;
use uuid::Uuid;

use super::{LibraryError, LibraryService};
use crate::create::SubmitOutcome;
use crate::library::enhance::{EnhanceChoice, EnhanceSheet, VoiceRow, operation_spec};
use crate::queue::QueueConfirmation;
use crate::queue::service::{budgeted, queued_notice, user_message};

/// The confirm button when the operation takes today over the cap.
const START_ANYWAY: &str = "Start Anyway";
const CANT_QUEUE: &str = "Can't queue this";
const NO_VOICES: &str = "Higgsfield lists no voices for this account.";
const VOICES_FAILED: &str = "Can't load the voices. Close this and try again.";

impl LibraryService {
    /// The result file's frame size and length, when ffmpeg can read them.
    async fn probed(&self, item: &crate::library::item::LibraryItem) -> (Option<PixelSize>, Option<f64>) {
        let (Some(tools), Some(path)) = (self.tools.as_ref(), item.output_file()) else {
            return (None, None);
        };
        match jutsu_media::probe::probe(tools, &path).await {
            Ok(info) => (
                Some(info.pixel_size).filter(jutsu_core::PixelSize::is_valid),
                Some(info.duration),
            ),
            Err(error) => {
                tracing::warn!(%error, "can't read the result's size");
                (None, None)
            }
        }
    }

    /// The sheet of `choice` on the result `id`, priced by the provider.
    pub async fn enhance_sheet(&self, id: Uuid, choice: EnhanceChoice) -> Result<EnhanceSheet, LibraryError> {
        let item = self.video_item(id).await?;
        let choice = choice.normalized();
        let (size, duration) = self.probed(&item).await;
        let spec = choice
            .action()
            .and_then(|action| operation_spec(&item.job, action, self.owner().as_deref(), duration, size));
        let price = match &spec {
            Some(spec) => self.estimate(spec).await,
            None => None,
        };
        let connected = self.connection.state().kind().is_some();
        Ok(EnhanceSheet::new(
            &item,
            choice,
            price.as_ref(),
            size.or_else(|| item.job.output_pixel_size()),
            duration,
            connected,
        ))
    }

    /// Queues `choice` on the result `id`. Over today's cap it asks first; `approved` is the answer. An unknown price
    /// must have been agreed to in the sheet.
    pub async fn enhance(&self, id: Uuid, choice: EnhanceChoice, approved: bool) -> SubmitOutcome {
        let item = match self.video_item(id).await {
            Ok(item) => item,
            Err(error) => {
                return SubmitOutcome::Blocked {
                    reason: error.to_string(),
                };
            }
        };
        let choice = choice.normalized();
        let (size, duration) = self.probed(&item).await;
        let Some(spec) = choice
            .action()
            .and_then(|action| operation_spec(&item.job, action, self.owner().as_deref(), duration, size))
        else {
            return SubmitOutcome::Blocked {
                reason: if choice.kind == PostActionKind::ChangeVoice {
                    "Pick a voice to see the price.".to_owned()
                } else {
                    CANT_QUEUE.to_owned()
                },
            };
        };
        let Some(price) = self.estimate(&spec).await else {
            return SubmitOutcome::Blocked {
                reason: "Higgsfield gave no price for this, so it can't be queued.".to_owned(),
            };
        };
        if !approved {
            let (committed, cap) = match self.cap_and_spend().await {
                Ok(spend) => spend,
                Err(reason) => return SubmitOutcome::Blocked { reason },
            };
            if let Some(cap) = cap
                && let Some(confirmation) = budgeted(1, Some(&price), committed, &cap)
            {
                return SubmitOutcome::Confirm {
                    confirmation: QueueConfirmation {
                        confirm_title: START_ANYWAY.to_owned(),
                        ..confirmation
                    },
                };
            }
        }
        let kind = self.connection.state().kind();
        let options = EnqueueOptions {
            parent_id: Some(item.id()),
            over_cap_approved: approved,
            ..EnqueueOptions::default()
        };
        match self.engine.enqueue(spec, options).await {
            Ok(job) => queued_notice(std::slice::from_ref(&job))
                .map_or(SubmitOutcome::Failed, |notice| SubmitOutcome::Queued { notice }),
            Err(error) => {
                if let Some(message) = user_message(&error, kind) {
                    self.connection.show_alert(message);
                }
                SubmitOutcome::Failed
            }
        }
    }

    /// The voices Change Voice offers, or the notice that stands in for them.
    pub async fn voices(&self) -> Result<Vec<VoiceRow>, String> {
        let Some(library) = self.locked().account.voices.clone() else {
            return Err(VOICES_FAILED.to_owned());
        };
        match library.voices().await {
            Ok(voices) if voices.is_empty() => Err(NO_VOICES.to_owned()),
            Ok(voices) => Ok(voices.iter().map(VoiceRow::new).collect()),
            Err(error) => {
                tracing::warn!(%error, "can't load the voices");
                Err(VOICES_FAILED.to_owned())
            }
        }
    }
}
