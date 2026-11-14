//! Make Video… from the Library (`MakeVideoHost.take`, `.place`): the images go into the video form in the role the
//! selected model takes, asked first when more than one fits.

use std::path::PathBuf;
use std::sync::Arc;

use jutsu_core::{LocalImage, ModelParameter};

use super::{CreateError, CreateService, file_url};
use crate::create::draft::replacement;
use crate::create::make_video::{MakeVideoChoice, MakeVideoOutcome, question};

impl CreateService {
    /// Places `files` as `choice` says. Without a choice, it is placed at once when only one role fits, else the
    /// outcome asks which; a choice the selected model no longer offers asks again. Like Edit Inputs, a draft that
    /// loses inputs comes back with `undo_replacement`.
    pub async fn make_video(
        self: &Arc<Self>,
        files: &[PathBuf],
        choice: Option<MakeVideoChoice>,
    ) -> Result<MakeVideoOutcome, CreateError> {
        let images: Vec<LocalImage> = files
            .iter()
            .map(|path| file_url(path).map(LocalImage::new))
            .collect::<Result<_, _>>()?;
        let takes_frames = self
            .context()
            .await
            .parameter_reason(ModelParameter::SeedanceFrames)
            .is_none();
        let outcome = {
            let mut state = self.locked();
            let offered = state.form.make_video_choices(images.len(), takes_frames);
            let choice = match (choice, offered.as_slice()) {
                (_, []) => {
                    return Ok(MakeVideoOutcome::Unavailable {
                        message: state.form.make_video_unavailable(),
                    });
                }
                (Some(choice), _) if offered.contains(&choice) => choice,
                (None, [only]) => only.clone(),
                _ => return Ok(question(&state.form.selected_model(), images.len(), &offered)),
            };
            let previous = state.form.clone();
            let mut form = previous.clone();
            let notice = form.make_video(images.clone(), &choice);
            let lost = replacement(&previous, &form, state.untouched.as_ref());
            let can_undo = lost.is_some();
            if lost.is_some() {
                state.replaced = lost;
            }
            state.untouched = Some(form.clone());
            state.form = form;
            MakeVideoOutcome::Placed { can_undo, notice }
        };
        self.edited();
        self.make_thumbnails(images);
        Ok(outcome)
    }
}
