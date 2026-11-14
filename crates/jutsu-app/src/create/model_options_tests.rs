//! `CreateFormModelOptionsTests`, ported: Seedance's sound, bitrate and frames, Kling's orientation, and what a
//! connection that can't send them says.

use std::collections::BTreeSet;

use jutsu_core::{
    AspectRatio, AudioOption, Capabilities, CharacterOrientation, CostUnit, LocalImage, Mode, ModelParameter,
    ProviderKind, Resolution, SeedanceBitrate, VideoModel,
};
use url::Url;
use uuid::Uuid;

use super::context::CreateContext;
use super::form::{CreateForm, FrameSlot};
use super::form_tests::{api_key, image, ready, video};

/// Every model and parameter offered, with the bundled list prices.
fn every_model_api_key() -> CreateContext {
    let mut capabilities = Capabilities::new(
        Mode::ALL.iter().copied().collect(),
        Resolution::ALL.iter().copied().collect(),
        CostUnit::Usd,
        None,
    );
    capabilities.models = VideoModel::all().into_iter().collect();
    CreateContext {
        capabilities: Some(capabilities),
        ..api_key()
    }
}

/// An account that runs every model but sends none of the parameters.
fn account() -> CreateContext {
    let mut capabilities = Capabilities::new(
        BTreeSet::from([Mode::MotionTransfer, Mode::ObjectSwap]),
        Resolution::ALL.iter().copied().collect(),
        CostUnit::Credits,
        None,
    );
    capabilities.models = VideoModel::all().into_iter().collect();
    capabilities.parameters = BTreeSet::new();
    CreateContext {
        capabilities: Some(capabilities),
        ..super::form_tests::account(Some(1000))
    }
}

fn seedance_without_source() -> CreateForm {
    let mut form = CreateForm::new(Mode::MotionTransfer, Resolution::P720, Resolution::P480);
    form.set_model(VideoModel::SEEDANCE25);
    "A fox turns to the camera".clone_into(&mut form.prompt);
    form
}

fn frame(name: &str) -> LocalImage {
    LocalImage::new(Url::from_file_path(format!("/tmp/{name}.png")).unwrap())
}

fn first_spec(form: &CreateForm) -> jutsu_core::JobSpec {
    form.make_specs(false).into_iter().next().unwrap()
}

// MARK: Audio

#[test]
fn seedance_with_a_source_can_make_its_own_sound() {
    let mut form = ready(Resolution::P720);
    assert_eq!(
        form.audio_choices(),
        [AudioOption::Original, AudioOption::Mute, AudioOption::FadeOut]
    );
    form.set_model(VideoModel::SEEDANCE25);
    assert_eq!(
        form.audio_choices(),
        [
            AudioOption::Original,
            AudioOption::Mute,
            AudioOption::FadeOut,
            AudioOption::Generated
        ]
    );
    assert_eq!(form.audio_detail(), "From the source");

    form.audio = AudioOption::Generated;
    assert_eq!(form.audio_detail(), "Made by Seedance 2.5");
    assert_eq!(first_spec(&form).audio, AudioOption::Generated);

    form.set_mode(Mode::Extend);
    assert!(!form.offers_generated_audio());
    form.set_mode(Mode::MotionTransfer);
    form.set_model(VideoModel::KLING_MOTION_CONTROL);
    assert!(!form.audio_choices().contains(&AudioOption::Generated));
    assert_eq!(form.audio_detail(), "From the source");
    assert_eq!(first_spec(&form).audio, AudioOption::Original);
}

// MARK: Bitrate and orientation

#[test]
fn seedance_has_a_bitrate_and_kling_an_orientation() {
    let mut form = ready(Resolution::P720);
    assert!(!form.shows_bitrate() && !form.shows_character_orientation());
    let spec = first_spec(&form);
    assert!(spec.bitrate.is_none() && spec.character_orientation.is_none());

    form.set_model(VideoModel::SEEDANCE25);
    assert!(form.shows_bitrate());
    assert!(first_spec(&form).bitrate.is_none());
    form.set_bitrate(SeedanceBitrate::Standard);
    let spec = first_spec(&form);
    assert!(spec.bitrate == Some(SeedanceBitrate::Standard) && spec.character_orientation.is_none());

    form.set_model(VideoModel::KLING_MOTION_CONTROL);
    assert!(form.shows_character_orientation());
    assert_eq!(form.character_orientation(), CharacterOrientation::Video);
    form.set_character_orientation(CharacterOrientation::Image);
    let spec = first_spec(&form);
    assert!(spec.character_orientation == Some(CharacterOrientation::Image) && spec.bitrate.is_none());

    let mut reopened = CreateForm::default();
    reopened.load(&spec, Some(&video(14.2)), None);
    assert_eq!(reopened.character_orientation(), CharacterOrientation::Image);
}

/// Until picked, the picker shows what each server does with no bitrate: Standard on the account, High with an API
/// key; nothing is sent, so the default stays the server's.
#[test]
fn the_bitrate_defaults_to_the_connections_own() {
    let mut form = seedance_without_source();
    assert_eq!(
        form.bitrate(Some(ProviderKind::Account)),
        SeedanceBitrate::Standard
    );
    assert_eq!(form.bitrate(Some(ProviderKind::ApiKey)), SeedanceBitrate::High);
    assert!(first_spec(&form).bitrate.is_none());

    form.set_bitrate(SeedanceBitrate::High);
    assert_eq!(form.bitrate(Some(ProviderKind::Account)), SeedanceBitrate::High);
    assert_eq!(first_spec(&form).bitrate, Some(SeedanceBitrate::High));
}

/// A connection that can't send a parameter has Create disable its control with the reason and a short hint; an API
/// key sends all of them. The account never sends Kling's sound or orientation: its model has neither.
#[test]
fn unsent_parameters_say_why_on_the_account() {
    for parameter in ModelParameter::ALL.iter().copied() {
        assert_eq!(
            every_model_api_key().parameter_reason(parameter),
            None,
            "{parameter:?}"
        );
        assert_eq!(
            every_model_api_key().parameter_hint(parameter),
            None,
            "{parameter:?}"
        );
        assert!(account().parameter_reason(parameter).is_some(), "{parameter:?}");
    }
    assert_eq!(
        account()
            .parameter_hint(ModelParameter::KlingOrientation)
            .as_deref(),
        Some("Only with an API key")
    );
    assert_eq!(
        account()
            .parameter_hint(ModelParameter::KlingVideoBackground)
            .as_deref(),
        Some("Not available for this account")
    );
    assert_eq!(api_key().parameter_reason(ModelParameter::SeedanceFrames), None);
    assert_eq!(
        account()
            .parameter_reason(ModelParameter::KlingOrientation)
            .as_deref(),
        Some("Kling's character orientation is set only with an API key, not on the account")
    );
    assert_eq!(
        account()
            .parameter_reason(ModelParameter::KlingOriginalSound)
            .as_deref(),
        Some("Kling keeps the source's sound on the server only with an API key, not on the account")
    );
}

// MARK: Frames

#[test]
fn frames_run_on_image_to_video_and_set_references_aside() {
    let context = every_model_api_key();
    let mut form = seedance_without_source();
    form.add_references(vec![image(1)]);
    form.aspect_ratio = AspectRatio::Portrait;
    assert!(form.shows_frames() && !form.uses_frames());
    assert_eq!(form.references_disabled_reason(), None);

    form.set_frame(FrameSlot::Start, Some(frame("start")));
    form.set_frame(FrameSlot::End, Some(frame("end")));
    assert!(form.uses_frames());
    assert!(form.references_disabled_reason().is_some());
    assert_eq!(form.blocker(false, &context), None);
    let spec = first_spec(&form);
    assert_eq!(spec.references, [frame("start"), frame("end")]);
    assert!(spec.references_are_frames && spec.uses_frames());
    assert_eq!(spec.aspect_ratio, None);
    assert_eq!(spec.endpoint_id(), "bytedance/seedance-2.5/image-to-video");
    // 5 s at 720p, priced like a clip without video input.
    assert_eq!(form.price_text(false, &context).as_deref(), Some("$2.31"));
    assert_eq!(form.rate_line(&context).as_deref(), Some("$0.462 / s"));

    let mut reopened = CreateForm::default();
    reopened.load(&spec, None, None);
    assert_eq!(reopened.frame(FrameSlot::Start), Some(&frame("start")));
    assert_eq!(reopened.frame(FrameSlot::End), Some(&frame("end")));
    assert_eq!(reopened.references(), Vec::<LocalImage>::new());
    assert_eq!(reopened.make_specs(false), [spec]);

    form.set_frame(FrameSlot::Start, None);
    form.set_frame(FrameSlot::End, None);
    let with_references = first_spec(&form);
    assert!(!with_references.references_are_frames && with_references.references.len() == 1);
    assert_eq!(with_references.aspect_ratio, Some(AspectRatio::Portrait));
}

#[test]
fn an_end_frame_alone_cant_run() {
    let mut form = seedance_without_source();
    form.set_frame(FrameSlot::End, Some(frame("end")));
    assert_eq!(
        form.blocker(false, &every_model_api_key()).as_deref(),
        Some("Add a start frame, or clear the end frame")
    );
}

/// A connection whose capabilities leave the frames out blocks them with the reason.
#[test]
fn frames_need_the_connection_to_send_them() {
    let mut form = seedance_without_source();
    form.set_frame(FrameSlot::Start, Some(frame("start")));
    assert_eq!(
        form.blocker(false, &account()),
        account().parameter_reason(ModelParameter::SeedanceFrames)
    );
}

/// Frames are for a clip made without a source on Seedance, outside a batch.
#[test]
fn frames_need_seedance_without_a_source() {
    let mut form = seedance_without_source();
    form.set_frame(FrameSlot::Start, Some(frame("start")));
    form.set_source(Some(video(14.2)));
    assert!(!form.shows_frames() && !form.uses_frames());
    assert!(!first_spec(&form).references_are_frames);

    form.set_source(None);
    form.set_model(VideoModel::GENJUTSU);
    assert!(!form.shows_frames());
    form.set_model(VideoModel::SEEDANCE25);
    form.set_batch(true);
    assert!(!form.shows_frames());
}

// MARK: Templates and drafts

#[test]
fn templates_carry_the_model_parameters() {
    let mut form = seedance_without_source();
    form.set_bitrate(SeedanceBitrate::Standard);
    form.set_frame(FrameSlot::Start, Some(frame("start")));
    let template = form.make_template("Frames", Uuid::new_v4(), time::OffsetDateTime::UNIX_EPOCH);
    assert!(template.references_are_frames && template.references == [frame("start")]);
    assert_eq!(template.bitrate, Some(SeedanceBitrate::Standard));
    assert_eq!(
        crate::templates::summary(&template),
        "Motion · Seedance 2.5 · Start frame · 720p"
    );

    let every_model: BTreeSet<VideoModel> = VideoModel::all().into_iter().collect();
    let mut applied = CreateForm::default();
    applied.apply_template(&template, &every_model);
    assert_eq!(applied.frame(FrameSlot::Start), Some(&frame("start")));
    assert_eq!(applied.references(), Vec::<LocalImage>::new());
    assert_eq!(
        applied.bitrate(Some(ProviderKind::ApiKey)),
        SeedanceBitrate::Standard
    );
    assert_eq!(applied.make_specs(false), form.make_specs(false));

    let mut kling = ready(Resolution::P720);
    kling.set_model(VideoModel::KLING_MOTION_CONTROL);
    kling.set_character_orientation(CharacterOrientation::Image);
    let mut applied_kling = CreateForm::default();
    applied_kling.apply_template(
        &kling.make_template("Kling", Uuid::new_v4(), time::OffsetDateTime::UNIX_EPOCH),
        &every_model,
    );
    assert_eq!(applied_kling.character_orientation(), CharacterOrientation::Image);
}

#[test]
fn a_draft_saved_before_the_model_parameters_still_decodes() {
    let mut form = seedance_without_source();
    form.set_bitrate(SeedanceBitrate::Standard);
    form.set_frame(FrameSlot::Start, Some(frame("start")));
    let mut object = serde_json::to_value(&form).unwrap();
    let fields = object.as_object_mut().unwrap();
    for key in ["seedanceBitrate", "startFrame"] {
        assert!(fields.remove(key).is_some(), "{key}");
    }
    let old: CreateForm = serde_json::from_value(object).unwrap();
    assert_eq!(old.bitrate(Some(ProviderKind::ApiKey)), SeedanceBitrate::High);
    assert_eq!(old.frame(FrameSlot::Start), None);
    let again: CreateForm = serde_json::from_value(serde_json::to_value(&form).unwrap()).unwrap();
    assert_eq!(again, form);
}

#[test]
fn frames_are_inputs_of_the_draft() {
    let mut form = CreateForm::default();
    assert!(!form.has_inputs());
    form.set_frame(FrameSlot::Start, Some(frame("start")));
    assert!(form.has_inputs());
    assert!(CreateForm::default().drops_inputs(&form));
    assert_eq!(form.reopening_files(&|_| false).frame(FrameSlot::Start), None);
    assert_eq!(
        form.reopening_files(&|_| true).frame(FrameSlot::Start),
        Some(&frame("start"))
    );
}

// MARK: The panel's view (`CreatePanel`, `ModelOptionControls`, `SourceSection`)

fn view(form: &CreateForm, context: &CreateContext) -> super::view::CreateSnapshot {
    super::view::snapshot(form, context, &super::view::MediaState::default())
}

/// `audioSection`: a model that makes the sound offers Generate or Silent, and Silent reaches the provider as a muted
/// job (which sends `generate_audio: false`).
#[test]
fn a_model_that_makes_the_sound_offers_generate_or_silent() {
    let context = every_model_api_key();
    let mut form = seedance_without_source();
    assert!(form.generates_audio());
    let audio = view(&form, &context).audio;
    let titles: Vec<&str> = audio.options.iter().map(|option| option.title.as_str()).collect();
    assert_eq!(titles, ["Generate", "Silent"]);
    assert_eq!(audio.selected, "original");
    assert_eq!(audio.detail.as_deref(), Some("Made by Seedance 2.5"));

    form.audio = AudioOption::Mute;
    assert_eq!(view(&form, &context).audio.selected, "mute");
    assert_eq!(first_spec(&form).audio, AudioOption::Mute);

    let from_source = view(&ready(Resolution::P720), &context).audio;
    assert_eq!(from_source.detail.as_deref(), Some("From the source"));
    let titles: Vec<&str> = from_source
        .options
        .iter()
        .map(|option| option.title.as_str())
        .collect();
    assert_eq!(titles, ["Original", "Mute", "Fade out"]);
}

/// `CinemaOptions`: one Direction row, on Auto until a control is set.
#[test]
fn cinema_controls_sit_in_a_direction_row_on_auto() {
    let mut form = seedance_without_source();
    form.set_model(VideoModel::from_raw("cinemaStudio4").unwrap());
    let cinema = view(&form, &every_model_api_key()).cinema.unwrap();
    assert_eq!((cinema.summary.as_str(), cinema.is_set), ("Auto", false));
    assert_eq!(cinema.hint, "Camera, lens, genre and more");
    assert!(
        cinema
            .controls
            .iter()
            .all(|control| control.options[0].title == "Auto" && control.selected.is_empty())
    );

    let control = jutsu_core::CinemaControl::ALL[0];
    form.set_cinema(control, Some(control.options()[0].to_owned()));
    let cinema = view(&form, &every_model_api_key()).cinema.unwrap();
    assert_eq!((cinema.summary.as_str(), cinema.is_set), ("1 set", true));
    let cinema = view(&form, &account()).cinema.unwrap();
    assert_eq!(
        Some(cinema.hint),
        account().parameter_hint(ModelParameter::CinemaControls)
    );
}

/// `FramesSection`'s help, `clipSection`'s length and `SourceSection`'s empty state.
#[test]
fn frames_length_and_source_explain_themselves() {
    let context = every_model_api_key();
    let form = seedance_without_source();
    let snapshot = view(&form, &context);
    let expected = if form.frame_slots().len() > 1 {
        "First and last frame"
    } else {
        "First frame"
    };
    assert_eq!(snapshot.frames.unwrap().hint, expected);
    let length = snapshot.clip_length.unwrap();
    assert_eq!(length.detail, crate::queue::format::limit(form.clip_length()));
    assert_eq!(snapshot.source_hint, "Or describe the clip in the prompt");
    assert_eq!(
        view(&ready(Resolution::P720), &context).source_hint,
        "Any format · trimmed to 4–30 s"
    );

    // A model that voices a speech runs as long as the speech: no Length.
    let mut voiced = seedance_without_source();
    voiced.set_model(VideoModel::from_raw("intone").unwrap());
    assert!(voiced.offers_speech());
    assert!(view(&voiced, &context).clip_length.is_none());
}
