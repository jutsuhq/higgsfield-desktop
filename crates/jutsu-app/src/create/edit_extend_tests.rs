//! `CreateFormEditExtendTests`, ported: Seedance's Edit and Extend tasks in the Motion tab, and the follow-ups of a
//! finished result (`ResultFollowUp.swift`).

use jutsu_core::{Cost, Job, JobSpec, Mode, Pricing, ProviderKind, Resolution, VideoModel};
use rust_decimal::Decimal;
use url::Url;
use uuid::Uuid;

use super::follow_up::follow_up;
use super::form::CreateForm;
use super::form_tests::{account, api_key, credits, quote, r, video, video_named};

/// Seedance with a 14.2 s source and a prompt.
fn seedance(mode: Mode) -> CreateForm {
    let mut form = CreateForm::new(Mode::MotionTransfer, Resolution::P720, Resolution::P480);
    form.set_model(VideoModel::SEEDANCE25);
    form.set_source(Some(video(14.2)));
    "make it night".clone_into(&mut form.prompt);
    form.set_mode(mode);
    form
}

#[test]
fn edit_and_extend_are_seedances_tasks_in_the_motion_tab() {
    let mut form = CreateForm::new(Mode::MotionTransfer, Resolution::P720, Resolution::P480);
    assert_eq!(form.task_choices(&api_key()), [Mode::MotionTransfer]);
    form.set_model(VideoModel::SEEDANCE25);
    assert_eq!(
        form.task_choices(&api_key()),
        [Mode::MotionTransfer, Mode::Edit, Mode::Extend]
    );

    form.pick_task(Mode::Edit);
    assert_eq!(form.tab(), Mode::MotionTransfer);
    assert!(form.model_options(&api_key()).contains(&VideoModel::GENJUTSU));
    form.set_source(None);
    assert!(!form.is_without_source());
    assert_eq!(form.make_specs(false), Vec::<JobSpec>::new());

    // A model without Edit takes the form back to motion transfer; one that only edits takes it to Edit.
    form.set_model(VideoModel::GENJUTSU);
    assert_eq!(form.mode(), Mode::MotionTransfer);
    form.set_model(VideoModel::from_raw("klingO3VideoEdit").unwrap());
    assert_eq!(form.mode(), Mode::Edit);

    form.set_tab(Mode::Restyle);
    assert_eq!(form.task_choices(&api_key()), Vec::<Mode>::new());
    form.set_tab(Mode::MotionTransfer);
    assert_eq!(form.mode(), Mode::Edit);
}

#[test]
fn edit_runs_on_seedance_and_needs_a_prompt() {
    let mut form = seedance(Mode::Edit);
    assert_eq!(form.selected_model(), VideoModel::SEEDANCE25);
    assert_eq!(form.prompt_title(), "Prompt");
    assert!(!form.shows_added_length());
    assert!(!form.offers_batch());

    let spec = form.make_specs(false).remove(0);
    assert_eq!(spec.mode, Mode::Edit);
    assert_eq!(spec.model, VideoModel::SEEDANCE25);
    assert_eq!(spec.length, None);
    assert_eq!(spec.endpoint_id(), "bytedance/seedance-2.5/video-edit");

    form.prompt = String::new();
    assert_eq!(
        form.blocker(false, &api_key()).as_deref(),
        Some("Say what should change")
    );
}

#[test]
fn extend_adds_its_length_and_keeps_the_whole_range_in_a_draft() {
    let mut form = seedance(Mode::Extend);
    form.length = 40.0;
    assert!(form.shows_added_length());
    assert_eq!(form.added_length_title(), "Add 4–30 s");
    assert!(form.generates_audio());
    assert!(!form.shows_draft_length());
    form.draft_length = super::form::DraftLength::FirstFiveSeconds;

    let draft = form.make_specs(true).remove(0);
    assert_eq!(draft.length, Some(30.0));
    assert!((draft.duration() - 30.0).abs() < f64::EPSILON);
    assert_eq!(draft.source.unwrap().range, r(0.0, 14.2));
    // With an API key the source selection and the added part are billed together.
    assert_eq!(form.billed_seconds(Some(ProviderKind::ApiKey)), 45);
    assert_eq!(form.cost_line(&api_key()).unwrap().text, "45 s = $12.48");
}

/// The account bills only the part Extend adds: a 4 s extension of a 4 s clip cost 12 credits, 4 s at 3 a second, in
/// a live run on 2026-10-15.
#[test]
fn extend_on_the_account_bills_only_the_added_part() {
    let mut form = seedance(Mode::Extend);
    form.length = 4.0;
    let spec = form.make_specs(false).remove(0);
    let mut context = account(Some(100));
    quote(&mut context, &spec, credits(12, false));

    assert_eq!(form.billed_seconds(Some(ProviderKind::Account)), 4);
    assert_eq!(
        form.trim_summary(Some(ProviderKind::Account)),
        "14.2 s selected · billed 4 s"
    );
    assert!(form.cost_line(&context).unwrap().text.starts_with("4 s"));
    assert_eq!(Pricing::billed_seconds_on(&spec, Some(ProviderKind::Account)), 4);
    assert_eq!(Pricing::billed_seconds_on(&spec, Some(ProviderKind::ApiKey)), 19);
}

#[test]
fn switching_to_edit_or_extend_ends_a_batch() {
    let mut form = seedance(Mode::MotionTransfer);
    form.set_batch(true);
    form.set_mode(Mode::Extend);
    assert!(!form.is_batch());
    assert_eq!(form.make_specs(false).len(), 1);
}

/// Opening an Extend of a result selects up to its last 30 s and reaches the end of the file as probed, even when
/// the result runs a little longer than asked for.
#[test]
fn loading_an_extend_of_a_result_ends_at_the_end_of_the_file() {
    let mut spec = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        None,
        Resolution::P720,
    );
    "a fox".clone_into(&mut spec.prompt);
    spec.length = Some(8.0);
    let job = Job::new(
        Uuid::new_v4(),
        spec,
        ProviderKind::ApiKey,
        Cost::usd(Decimal::ONE, true),
        Uuid::new_v4(),
        time::OffsetDateTime::UNIX_EPOCH,
    );
    let mut finished = job.clone();
    finished.output_file = Some(Url::from_file_path("/tmp/result.mp4").unwrap());
    let spec = follow_up(&finished, Mode::Extend).unwrap();
    assert_eq!(spec.mode, Mode::Extend);
    assert_eq!(spec.model, VideoModel::SEEDANCE25);
    assert_eq!(spec.source.as_ref().unwrap().range, r(0.0, 8.0));
    assert_eq!(spec.length, Some(CreateForm::DEFAULT_LENGTH));
    assert_eq!(follow_up(&job, Mode::Extend), None);
    assert_eq!(follow_up(&finished, Mode::MotionTransfer), None);

    let result = video_named(8.04, 1920, 1080, "avc1", "result.mp4");
    let mut form = CreateForm::new(Mode::MotionTransfer, Resolution::P480, Resolution::P480);
    form.load(&spec, Some(&result), Some(job.id));
    assert_eq!(form.mode(), Mode::Extend);
    assert_eq!(form.range(), r(0.0, 8.04));
    assert_eq!(form.parent_id(), Some(job.id));

    let edit = follow_up(&finished, Mode::Edit).unwrap();
    form.load(&edit, Some(&result), Some(job.id));
    assert_eq!(form.range(), r(0.0, 8.0));
}
