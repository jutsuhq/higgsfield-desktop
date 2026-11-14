//! Make Video from Library images (`MakeVideo.swift`, `MakeVideoHost.swift`): the part each image plays, asked when
//! more than one fits the selected model.

use jutsu_core::{LocalImage, Mode, ModelConstraintTable, VideoControl, VideoModel};
use serde::{Deserialize, Serialize};

use super::form::{CreateForm, FrameSlot};
use crate::connection::UserMessage;

/// The part an image plays in a video made from it: a frame the clip starts or ends on, or a reference to draw from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ImageRole {
    StartFrame,
    EndFrame,
    Reference,
}

impl ImageRole {
    pub fn title(self) -> &'static str {
        match self {
            Self::StartFrame => FrameSlot::Start.title(),
            Self::EndFrame => FrameSlot::End.title(),
            Self::Reference => "Reference",
        }
    }

    /// The roles `model` takes for an image in `mode`, in this order: start frame, end frame, reference. Frames go
    /// with a model that makes a clip from them without a source, the end frame where it takes one; a reference
    /// with one whose limit takes an image.
    pub fn options(model: &VideoModel, mode: Mode, constraints: &ModelConstraintTable) -> Vec<Self> {
        let mut roles = Vec::new();
        if model.declares(VideoControl::Frames) && model.makes_clips_without_source() {
            roles.push(Self::StartFrame);
            if model.takes_end_frame() {
                roles.push(Self::EndFrame);
            }
        }
        if model.limits(mode, true, constraints).reference_images.upper() > 0 {
            roles.push(Self::Reference);
        }
        roles
    }
}

/// One way Make Video can place the chosen images in Create: the role of each image, in the images' order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MakeVideoChoice {
    pub roles: Vec<ImageRole>,
}

impl MakeVideoChoice {
    pub fn new(roles: Vec<ImageRole>) -> Self {
        Self { roles }
    }

    /// Frames replace the source video, which Create says with the Undo toast.
    pub fn uses_frames(&self) -> bool {
        self.roles.iter().any(|role| *role != ImageRole::Reference)
    }

    /// "Start frame", "Start and end frames", "3 references".
    pub fn title(&self) -> String {
        match self.roles.as_slice() {
            [ImageRole::StartFrame, ImageRole::EndFrame] => "Start and end frames".to_owned(),
            [role] => role.title().to_owned(),
            roles => format!("{} references", roles.len()),
        }
    }
}

/// A button of the role question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MakeVideoOption {
    pub choice: MakeVideoChoice,
    pub title: String,
}

/// What Make Video did with the images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "kind")]
pub enum MakeVideoOutcome {
    /// More than one role fits: ask, then send the picked choice back.
    Ask {
        title: String,
        detail: String,
        options: Vec<MakeVideoOption>,
    },
    /// Placed. `can_undo` offers the replaced draft back ("Draft replaced"); `notice` is what didn't fit.
    Placed {
        can_undo: bool,
        notice: Option<UserMessage>,
    },
    /// The selected model takes none of the roles.
    Unavailable { message: UserMessage },
}

/// The role question for `count` images on `model` (`MakeVideoHost.title`, `.detail`).
pub fn question(model: &VideoModel, count: usize, choices: &[MakeVideoChoice]) -> MakeVideoOutcome {
    let title = if count > 1 {
        format!("How should {} use these images?", model.title())
    } else {
        format!("How should {} use this image?", model.title())
    };
    let detail = if choices.iter().any(MakeVideoChoice::uses_frames) {
        "A start or end frame makes the clip from the image alone, without a source video."
    } else {
        "References show the model what to draw from."
    };
    MakeVideoOutcome::Ask {
        title,
        detail: detail.to_owned(),
        options: choices
            .iter()
            .map(|choice| MakeVideoOption {
                choice: choice.clone(),
                title: choice.title(),
            })
            .collect(),
    }
}

impl CreateForm {
    /// The ways the selected model can take `count` images: each role it takes for one, a start and an end frame or
    /// references for two, references for more. Empty when it takes none of them. `takes_frames` is false when the
    /// connection can't send frames (`ModelParameter::SeedanceFrames`), so they aren't offered.
    pub fn make_video_choices(&self, count: usize, takes_frames: bool) -> Vec<MakeVideoChoice> {
        let roles: Vec<ImageRole> =
            ImageRole::options(&self.selected_model(), self.mode(), &self.constraints())
                .into_iter()
                .filter(|role| takes_frames || *role == ImageRole::Reference)
                .collect();
        match count {
            1 => roles
                .into_iter()
                .map(|role| MakeVideoChoice::new(vec![role]))
                .collect(),
            2 => {
                let mut choices = Vec::new();
                if roles.contains(&ImageRole::StartFrame) && roles.contains(&ImageRole::EndFrame) {
                    choices.push(MakeVideoChoice::new(vec![
                        ImageRole::StartFrame,
                        ImageRole::EndFrame,
                    ]));
                }
                if roles.contains(&ImageRole::Reference) {
                    choices.push(MakeVideoChoice::new(vec![ImageRole::Reference; 2]));
                }
                choices
            }
            0 => Vec::new(),
            count if roles.contains(&ImageRole::Reference) => {
                vec![MakeVideoChoice::new(vec![ImageRole::Reference; count])]
            }
            _ => Vec::new(),
        }
    }

    /// Why Make Video can't place images on the selected model; shown when `make_video_choices` is empty.
    pub fn make_video_unavailable(&self) -> UserMessage {
        UserMessage::new(
            "Can't make a video from these images",
            format!(
                "{} doesn't take them. Pick another model in Create and try again.",
                self.selected_model().title()
            ),
            None,
        )
    }

    /// Places `images` as `choice` says. Frames start a clip without a source, so they take the Motion task and
    /// replace the source video; references stop the frames, which would leave them unused, and a character batch,
    /// which has none. Returns what didn't fit, as opened files do.
    pub fn make_video(&mut self, images: Vec<LocalImage>, choice: &MakeVideoChoice) -> Option<UserMessage> {
        if choice.uses_frames() {
            self.set_model(self.selected_model());
            if self.mode().is_video_operation() {
                self.set_mode(Mode::MotionTransfer);
            }
            if self.source().is_some() {
                self.set_source(None);
            }
            self.set_batch(false);
            for (image, role) in images.into_iter().zip(&choice.roles) {
                match role {
                    ImageRole::StartFrame => self.set_frame(FrameSlot::Start, Some(image)),
                    ImageRole::EndFrame => self.set_frame(FrameSlot::End, Some(image)),
                    ImageRole::Reference => {}
                }
            }
            return None;
        }
        if self.uses_frames() {
            self.set_frame(FrameSlot::Start, None);
            self.set_frame(FrameSlot::End, None);
        }
        if self.is_batch() && self.mode() != Mode::Restyle {
            self.set_batch(false);
        }
        self.add_opened_references(images)
    }

    /// The images the jobs send and their roles: the frames while they are used, else the references.
    pub fn placed_images(&self) -> Vec<(LocalImage, ImageRole)> {
        if self.uses_frames() {
            return [
                (self.frame(FrameSlot::Start), ImageRole::StartFrame),
                (self.frame(FrameSlot::End), ImageRole::EndFrame),
            ]
            .into_iter()
            .filter_map(|(image, role)| image.map(|image| (image.clone(), role)))
            .collect();
        }
        self.references()
            .iter()
            .map(|image| (image.clone(), ImageRole::Reference))
            .collect()
    }

    /// Every image the form holds, used or waiting: the frames and the references.
    fn held_images(&self) -> std::collections::BTreeSet<url::Url> {
        self.frame(FrameSlot::Start)
            .into_iter()
            .chain(self.frame(FrameSlot::End))
            .chain(self.references())
            .map(|image| image.file_url.clone())
            .collect()
    }

    /// Runs `change` (another model, tab or task), then checks the images' roles again. Frames the selected model no
    /// longer takes become references, first, where there is room; a reference over its limit is cut as before.
    /// Returns what changed in a sentence, `None` when nothing did (`CreateForm.recheckingRoles`).
    pub fn rechecking_roles(&mut self, change: impl FnOnce(&mut Self)) -> Option<String> {
        let before = self.placed_images();
        let held = self.held_images();
        let had_frames = self.uses_frames();
        change(self);
        let limit = self.limits().reference_images.upper();
        let merged = |first: Vec<LocalImage>, rest: &[LocalImage]| -> Vec<LocalImage> {
            first
                .iter()
                .cloned()
                .chain(
                    rest.iter()
                        .filter(|image| !first.iter().any(|moved| moved.file_url == image.file_url))
                        .cloned(),
                )
                .take(limit)
                .collect()
        };
        if had_frames && !self.shows_frames() && !self.selected_model().declares(VideoControl::Frames) {
            let moved: Vec<LocalImage> = [FrameSlot::Start, FrameSlot::End]
                .into_iter()
                .filter_map(|slot| self.frame(slot).cloned())
                .collect();
            self.set_frame(FrameSlot::Start, None);
            self.set_frame(FrameSlot::End, None);
            let references = merged(moved, self.references());
            self.set_references_unclamped(references);
        }
        // A model with a start frame alone (Kling 4.0 Flash) keeps the end frame as the first reference.
        let moved_end = (self.shows_frames() && !self.selected_model().takes_end_frame())
            .then(|| self.frame(FrameSlot::End).cloned())
            .flatten();
        if let Some(end) = &moved_end {
            self.set_frame(FrameSlot::End, None);
            let references = merged(vec![end.clone()], self.references());
            self.set_references_unclamped(references);
        }
        let after = self.placed_images();
        let removed = held.difference(&self.held_images()).count();
        let became_references = before.iter().any(|(image, role)| {
            *role != ImageRole::Reference
                && after
                    .iter()
                    .find(|(placed, _)| placed.file_url == image.file_url)
                    .is_some_and(|(_, role)| *role == ImageRole::Reference)
        });
        let model = self.selected_model().title();
        let mut lines = Vec::new();
        if moved_end.is_some() {
            lines.push(format!(
                "{model} takes only a start frame, so the end frame is a reference now."
            ));
        } else if became_references {
            lines.push(format!(
                "{model} doesn't take start and end frames, so they are references now."
            ));
        }
        if removed > 0 {
            let images = if limit == 1 {
                "1 reference image".to_owned()
            } else {
                format!("{limit} reference images")
            };
            let removed_images = if removed == 1 {
                "1 image was removed".to_owned()
            } else {
                format!("{removed} images were removed")
            };
            lines.push(format!("{model} takes up to {images}: {removed_images}."));
        }
        (!lines.is_empty()).then(|| lines.join(" "))
    }
}

#[cfg(test)]
mod tests {
    //! `MakeVideoTests`: the role options, the choices and the placing.

    use jutsu_core::{MaterialReference, PixelSize};
    use url::Url;
    use uuid::Uuid;

    use super::*;
    use crate::create::form::SourceVideo;

    fn image(name: &str) -> LocalImage {
        LocalImage::new(Url::from_file_path(format!("/tmp/{name}.png")).unwrap())
    }

    fn version() -> MaterialReference {
        MaterialReference::new(Uuid::from_u128(1), Uuid::from_u128(2))
    }

    fn start() -> LocalImage {
        let mut image = image("start");
        image.material = Some(version());
        image
    }

    fn seedance() -> CreateForm {
        let mut form = CreateForm::new(
            Mode::MotionTransfer,
            jutsu_core::Resolution::P720,
            jutsu_core::Resolution::P480,
        );
        form.set_model(VideoModel::SEEDANCE25);
        form
    }

    fn clip() -> SourceVideo {
        SourceVideo {
            file_url: Url::from_file_path("/tmp/clip.mp4").unwrap(),
            duration: 14.2,
            pixel_size: PixelSize::new(1920, 1080),
            codec: "avc1".to_owned(),
            file_size: 38_000_000,
            remote_origin: None,
        }
    }

    fn roles(choices: &[MakeVideoChoice]) -> Vec<Vec<ImageRole>> {
        choices.iter().map(|choice| choice.roles.clone()).collect()
    }

    use ImageRole::{EndFrame, Reference, StartFrame};

    #[test]
    fn roles_follow_the_models_declared_inputs() {
        let table = ModelConstraintTable::default();
        assert_eq!(
            ImageRole::options(&VideoModel::SEEDANCE25, Mode::MotionTransfer, &table),
            [StartFrame, EndFrame, Reference]
        );
        assert_eq!(
            ImageRole::options(&VideoModel::GENJUTSU, Mode::Restyle, &table),
            [Reference]
        );
        assert_eq!(
            ImageRole::options(&VideoModel::KLING_MOTION_CONTROL, Mode::MotionTransfer, &table),
            [Reference]
        );
        // Kling 4.0 Flash starts from a frame but has no end frame.
        let flash = VideoModel::from_raw("kling40Flash").unwrap();
        assert_eq!(
            ImageRole::options(&flash, Mode::MotionTransfer, &table),
            [StartFrame, Reference]
        );
        // Every model in models.json: frames only where it declares them, a reference where its limit takes one.
        for model in VideoModel::all() {
            let mode = model.modes()[0];
            let roles = ImageRole::options(&model, mode, &table);
            assert_eq!(
                roles.contains(&StartFrame),
                model.declares(VideoControl::Frames),
                "{model:?}"
            );
            assert_eq!(
                roles.contains(&Reference),
                model.limits(mode, true, &table).reference_images.upper() > 0,
                "{model:?}"
            );
        }
    }

    #[test]
    fn a_reference_already_added_is_pinned_to_the_version_it_comes_with() {
        let mut form = CreateForm::new(
            Mode::Restyle,
            jutsu_core::Resolution::P720,
            jutsu_core::Resolution::P480,
        );
        form.set_model(VideoModel::GENJUTSU);
        form.add_references(vec![image("start")]);
        assert_eq!(
            form.make_video(vec![start()], &MakeVideoChoice::new(vec![Reference])),
            None
        );
        assert_eq!(form.references().len(), 1);
        assert_eq!(form.references()[0].material, Some(version()));
    }

    #[test]
    fn choices_ask_only_when_more_than_one_fits() {
        let form = seedance();
        assert_eq!(
            roles(&form.make_video_choices(1, true)),
            [vec![StartFrame], vec![EndFrame], vec![Reference]]
        );
        assert_eq!(
            roles(&form.make_video_choices(2, true)),
            [vec![StartFrame, EndFrame], vec![Reference, Reference]]
        );
        assert_eq!(
            roles(&form.make_video_choices(3, true)),
            [vec![Reference, Reference, Reference]]
        );
        // The connection can't send frames, so they aren't offered.
        assert_eq!(roles(&form.make_video_choices(1, false)), [vec![Reference]]);
        let genjutsu = CreateForm::new(
            Mode::MotionTransfer,
            jutsu_core::Resolution::P720,
            jutsu_core::Resolution::P480,
        );
        assert_eq!(
            roles(&genjutsu.make_video_choices(2, true)),
            [vec![Reference, Reference]]
        );
        assert_eq!(form.make_video_choices(0, true), Vec::<MakeVideoChoice>::new());
        assert_eq!(
            MakeVideoChoice::new(vec![StartFrame, EndFrame]).title(),
            "Start and end frames"
        );
        assert_eq!(MakeVideoChoice::new(vec![EndFrame]).title(), "End frame");
    }

    #[test]
    fn frames_replace_the_source_and_pin_the_version() {
        let mut form = seedance();
        form.set_mode(Mode::Edit);
        form.set_source(Some(clip()));
        form.add_references(vec![image("third")]);
        let choice = MakeVideoChoice::new(vec![StartFrame, EndFrame]);
        assert_eq!(form.make_video(vec![start(), image("end")], &choice), None);
        assert_eq!(form.mode(), Mode::MotionTransfer);
        assert!(form.source().is_none());
        assert_eq!(form.frame(FrameSlot::Start), Some(&start()));
        assert_eq!(form.frame(FrameSlot::End), Some(&image("end")));
        let spec = form.make_specs(false).into_iter().next().unwrap();
        assert!(spec.uses_frames());
        let materials: Vec<_> = spec
            .references
            .iter()
            .map(|reference| reference.material)
            .collect();
        assert_eq!(materials, [Some(version()), None]);
    }

    #[test]
    fn one_image_fills_the_role_asked() {
        let mut form = seedance();
        let _ = form.make_video(vec![image("end")], &MakeVideoChoice::new(vec![EndFrame]));
        assert_eq!(form.frame(FrameSlot::End), Some(&image("end")));
        assert_eq!(form.frame(FrameSlot::Start), None);
        assert_eq!(form.references(), Vec::<LocalImage>::new());
    }

    #[test]
    fn references_stop_the_frames_that_would_leave_them_unused() {
        let mut form = seedance();
        form.set_frame(FrameSlot::Start, Some(start()));
        let _ = form.make_video(vec![image("third")], &MakeVideoChoice::new(vec![Reference]));
        assert!(!form.uses_frames());
        assert_eq!(form.references(), [image("third")]);
    }

    // MARK: Switching the model

    fn start_and_end() -> CreateForm {
        let mut form = seedance();
        let _ = form.make_video(
            vec![start(), image("end")],
            &MakeVideoChoice::new(vec![StartFrame, EndFrame]),
        );
        form
    }

    fn placed_roles(form: &CreateForm) -> Vec<ImageRole> {
        form.placed_images().into_iter().map(|(_, role)| role).collect()
    }

    #[test]
    fn a_model_without_frames_turns_them_into_references() {
        let mut form = start_and_end();
        let note = form
            .rechecking_roles(|form| form.set_model(VideoModel::GENJUTSU))
            .unwrap();
        assert_eq!(
            note,
            "Genjutsu doesn't take start and end frames, so they are references now."
        );
        assert_eq!(form.references(), [start(), image("end")]);
        assert_eq!(placed_roles(&form), [Reference, Reference]);
        assert_eq!(form.references()[0].material, Some(version()));
    }

    #[test]
    fn a_smaller_limit_says_what_was_removed() {
        let mut form = start_and_end();
        let note = form
            .rechecking_roles(|form| form.set_model(VideoModel::KLING_MOTION_CONTROL))
            .unwrap();
        let kling = VideoModel::KLING_MOTION_CONTROL.title();
        assert!(note.starts_with(&format!("{kling} doesn't take start and end frames")));
        assert!(note.ends_with(&format!(
            "{kling} takes up to 1 reference image: 1 image was removed."
        )));
        // The start frame goes first, so the frame the clip started on is the one kept.
        assert_eq!(form.references(), [start()]);
    }

    /// A model with a start frame alone keeps the start frame and makes the end frame the first reference.
    #[test]
    fn a_model_without_an_end_frame_makes_it_a_reference() {
        let mut form = seedance();
        form.add_references(vec![image("third")]);
        let _ = form.make_video(
            vec![start(), image("end")],
            &MakeVideoChoice::new(vec![StartFrame, EndFrame]),
        );
        let flash = VideoModel::from_raw("kling40Flash").unwrap();
        let note = form.rechecking_roles(|form| form.set_model(flash)).unwrap();
        assert_eq!(
            note,
            "Kling 4.0 Flash takes only a start frame, so the end frame is a reference now."
        );
        assert_eq!(form.frame_slots(), [FrameSlot::Start]);
        assert_eq!(form.frame(FrameSlot::Start), Some(&start()));
        assert_eq!(form.frame(FrameSlot::End), None);
        assert_eq!(form.references(), [image("end"), image("third")]);
        assert_eq!(placed_roles(&form), [StartFrame]);
        assert_eq!(
            roles(&form.make_video_choices(2, true)),
            [vec![Reference, Reference]]
        );
    }

    #[test]
    fn a_switch_that_keeps_every_role_says_nothing() {
        let mut form = CreateForm::new(
            Mode::MotionTransfer,
            jutsu_core::Resolution::P720,
            jutsu_core::Resolution::P480,
        );
        form.set_source(Some(clip()));
        form.add_references(vec![image("1")]);
        assert_eq!(
            form.rechecking_roles(|form| form.set_model(VideoModel::SEEDANCE25)),
            None
        );
        let mut frames = seedance();
        frames.set_frame(FrameSlot::Start, Some(start()));
        assert_eq!(frames.rechecking_roles(|form| form.length = 8.0), None);
        assert_eq!(frames.frame(FrameSlot::Start), Some(&start()));
    }

    #[test]
    fn a_tab_that_falls_back_to_genjutsu_is_checked_too() {
        let mut form = seedance();
        form.set_frame(FrameSlot::Start, Some(start()));
        let note = form.rechecking_roles(|form| form.set_tab(Mode::Restyle));
        assert!(note.is_some_and(|note| note.contains("Genjutsu")));
        assert_eq!(form.references(), [start()]);
    }

    #[test]
    fn references_over_the_limit_are_said() {
        let mut form = CreateForm::new(
            Mode::MotionTransfer,
            jutsu_core::Resolution::P720,
            jutsu_core::Resolution::P480,
        );
        form.set_source(Some(clip()));
        form.add_references(vec![image("1")]);
        form.set_model(VideoModel::KLING_MOTION_CONTROL);
        let message = form.make_video(
            vec![start(), image("end")],
            &MakeVideoChoice::new(vec![Reference, Reference]),
        );
        assert_eq!(
            message.map(|message| message.title).as_deref(),
            Some("2 images weren't added")
        );
    }
}
