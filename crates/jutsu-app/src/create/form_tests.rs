//! `CreateFormTests`, `CreateFormRangesTests`, `CreateFormKeyboardTests`, `CreateDraftTests`, `EstimateCacheTests`,
//! `VariantsTests` and `ModeOption` cases, ported.

use std::collections::BTreeSet;
use std::sync::Arc;

use jutsu_core::{
    AudioOption, Capabilities, ClosedRange, Cost, CostUnit, LocalImage, Mode, ModeLimits, PixelSize, Pricing,
    ProviderKind, Resolution, SpendCap, VideoModel,
};
use rust_decimal::Decimal;
use uuid::Uuid;

use super::context::{CreateContext, EstimateCache, SubmitDecision, price_key};
use super::draft::{CreateDraftStore, replacement};
use super::form::{CreateForm, DraftLength, PrimarySubmit, SourceVideo, TimeRange, TrimPart, time_range};
use super::view::{MediaState, snapshot};

pub(super) fn video_named(duration: f64, width: i64, height: i64, codec: &str, name: &str) -> SourceVideo {
    SourceVideo {
        file_url: crate::testing::file_url(format!("/tmp/{name}")),
        duration,
        pixel_size: PixelSize::new(width, height),
        codec: codec.to_owned(),
        file_size: 38_000_000,
        remote_origin: None,
    }
}

pub(super) fn video(duration: f64) -> SourceVideo {
    video_named(duration, 1920, 1080, "avc1", "clip.mp4")
}

pub(super) fn image(index: usize) -> LocalImage {
    LocalImage::new(crate::testing::file_url(format!("/tmp/ref{index}.png")))
}

pub(super) fn images(indices: impl IntoIterator<Item = usize>) -> Vec<LocalImage> {
    indices.into_iter().map(image).collect()
}

pub(super) fn form(mode: Mode, resolution: Resolution) -> CreateForm {
    CreateForm::new(mode, resolution, Resolution::P480)
}

/// Motion transfer with a 14.2 s clip and one image: submittable.
pub(super) fn ready(resolution: Resolution) -> CreateForm {
    let mut form = form(Mode::MotionTransfer, resolution);
    form.set_source(Some(video(14.2)));
    form.add_references(images([1]));
    form
}

pub(super) fn r(lower: f64, upper: f64) -> TimeRange {
    time_range(lower, upper)
}

pub(super) fn api_key() -> CreateContext {
    CreateContext {
        connection: Some(ProviderKind::ApiKey),
        pricing: Some(Arc::new(Pricing::bundled().unwrap())),
        ..CreateContext::default()
    }
}

pub(super) fn account(credits: Option<i64>) -> CreateContext {
    CreateContext {
        connection: Some(ProviderKind::Account),
        credits_left: credits,
        ..CreateContext::default()
    }
}

pub(super) fn quote(context: &mut CreateContext, spec: &jutsu_core::JobSpec, cost: Cost) {
    context.estimates.costs.insert(price_key(spec), cost);
}

pub(super) fn credits(amount: i64, is_estimate: bool) -> Cost {
    Cost::credits(Decimal::from(amount), is_estimate)
}

pub(super) fn three_ranges(resolution: Resolution) -> CreateForm {
    let mut form = ready(resolution);
    form.set_source(Some(video(47.3)));
    form.set_end(10.0);
    form.add_range();
    form.add_range();
    form
}

// MARK: Ranges

#[test]
fn a_short_clip_is_selected_whole_and_a_long_one_from_the_start() {
    let mut form = CreateForm::default();
    form.set_source(Some(video(14.2)));
    assert_eq!(form.range(), r(0.0, 14.2));
    form.set_source(Some(video(47.3)));
    assert_eq!(form.range(), r(0.0, 30.0));
    assert!(
        form.stage_pills()
            .contains(&"47.3 s clip — pick up to 30 s below".to_owned())
    );
}

#[test]
fn handles_keep_the_length_within_4_to_30_seconds() {
    let mut form = CreateForm::default();
    form.set_source(Some(video(47.5)));
    form.set_start(28.0);
    assert_eq!(form.range(), r(26.0, 30.0));
    form.set_end(100.0);
    assert_eq!(form.range(), r(26.0, 47.5));
    form.set_start(0.0);
    assert_eq!(form.range(), r(17.5, 47.5));
    form.set_end(10.0);
    assert_eq!(form.range(), r(17.5, 21.5));
}

#[test]
fn moving_keeps_the_length_inside_the_clip() {
    let mut form = CreateForm::default();
    form.set_source(Some(video(47.5)));
    form.set_end(10.0);
    form.move_range(45.0);
    assert_eq!(form.range(), r(37.5, 47.5));
    form.move_range(-3.0);
    assert_eq!(form.range(), r(0.0, 10.0));
}

#[test]
fn a_clip_shorter_than_4_seconds_stays_whole_and_blocks() {
    let mut form = ready(Resolution::P720);
    form.set_source(Some(video(3.0)));
    form.set_start(1.0);
    form.set_end(2.0);
    assert_eq!(form.range(), r(0.0, 3.0));
    assert_eq!(
        form.blocker(false, &api_key()).as_deref(),
        Some("Pick at least 4 s of video")
    );
}

#[test]
fn added_ranges_follow_the_edited_one_and_take_its_length() {
    let form = three_ranges(Resolution::P720);
    assert_eq!(form.ranges(), [r(0.0, 10.0), r(10.0, 20.0), r(20.0, 30.0)]);
    assert_eq!(form.selected_range_index(), 2);
    assert_eq!(form.range_text(2), "00:20.0 – 00:30.0");
    assert_eq!(form.range_text(7), "");
    assert_eq!(form.uncovered_ranges(), [r(30.0, 47.3)]);
    assert_eq!(form.billed_seconds(Some(ProviderKind::ApiKey)), 30);
    assert_eq!(
        form.trim_summary(Some(ProviderKind::ApiKey)),
        "10.0 s selected · 3 ranges · billed 30 s"
    );
    assert_eq!(form.source_summary(), "3 ranges · 30.0 s");
}

#[test]
fn a_new_range_fits_the_first_gap_of_at_least_4_seconds() {
    let mut form = CreateForm::default();
    form.set_source(Some(video(14.2)));
    assert!(!form.can_add_range());
    form.add_range();
    assert_eq!(form.ranges(), [r(0.0, 14.2)]);
    form.set_end(10.0);
    assert!(form.can_add_range());
    form.add_range();
    assert_eq!(form.ranges(), [r(0.0, 10.0), r(10.0, 14.2)]);
    assert!(!form.can_add_range());
    assert!(!CreateForm::default().can_add_range());
}

#[test]
fn a_range_goes_back_into_the_gap_it_left() {
    let mut form = three_ranges(Resolution::P720);
    form.remove_range(1);
    assert_eq!(form.ranges(), [r(0.0, 10.0), r(20.0, 30.0)]);
    assert_eq!(form.selected_range_index(), 1);
    form.select_range(0);
    form.add_range();
    assert_eq!(form.ranges(), [r(0.0, 10.0), r(10.0, 20.0), r(20.0, 30.0)]);
    assert_eq!(form.selected_range_index(), 1);
}

#[test]
fn handles_stop_at_the_neighbours() {
    let mut form = three_ranges(Resolution::P720);
    form.select_range(1);
    form.set_end(25.0);
    assert_eq!(form.range(), r(10.0, 20.0));
    form.set_start(5.0);
    assert_eq!(form.range(), r(10.0, 20.0));
    form.move_range(0.0);
    assert_eq!(form.range(), r(10.0, 20.0));
    form.select_range(2);
    form.move_range(40.0);
    assert_eq!(form.range(), r(37.3, 47.3));
    form.select_range(1);
    form.set_end(28.0);
    assert_eq!(form.ranges(), [r(0.0, 10.0), r(10.0, 28.0), r(37.3, 47.3)]);
}

#[test]
fn removing_keeps_at_least_one_range_and_a_valid_selection() {
    let mut form = three_ranges(Resolution::P720);
    form.remove_range(2);
    assert_eq!(form.selected_range_index(), 1);
    form.remove_range(0);
    assert_eq!(form.ranges(), [r(10.0, 20.0)]);
    assert_eq!(form.selected_range_index(), 0);
    form.remove_range(0);
    form.remove_range(5);
    assert_eq!(form.ranges(), [r(10.0, 20.0)]);
}

#[test]
fn a_loaded_job_or_a_new_source_has_one_range() {
    let mut form = three_ranges(Resolution::P720);
    let spec = form.make_specs(false).pop().unwrap();
    form.load(&spec, Some(&video(47.3)), None);
    assert_eq!(form.ranges(), [r(20.0, 30.0)]);
    assert_eq!(form.selected_range_index(), 0);
    let mut form = three_ranges(Resolution::P720);
    form.set_source(Some(video(14.2)));
    assert_eq!(form.ranges(), [r(0.0, 14.2)]);
}

// `CreateFormRangesTests`, the rest.

#[test]
fn single_range_keeps_the_one_range_texts() {
    let mut form = ready(Resolution::P720);
    form.set_source(Some(video(47.3)));
    form.set_start(6.0);
    form.set_end(18.5);
    assert!(!form.has_several_ranges());
    assert_eq!(form.ranges(), [r(6.0, 18.5)]);
    assert_eq!(form.range_text(0), "00:06.0 – 00:18.5");
    assert_eq!(form.source_summary(), "00:06.0 – 00:18.5 · 12.5 s");
    assert_eq!(
        form.trim_summary(Some(ProviderKind::ApiKey)),
        "12.5 s selected · billed 13 s"
    );
    assert_eq!(
        form.cost_line(&api_key()).unwrap().detail,
        "List price · failed runs are free"
    );
}

#[test]
fn each_range_is_one_job_with_the_same_inputs() {
    let mut form = three_ranges(Resolution::P1080);
    form.prompt = "Rain".to_owned();
    form.audio = AudioOption::FadeOut;
    let specs = form.make_specs(false);
    let ranges: Vec<TimeRange> = specs
        .iter()
        .map(|spec| spec.source.as_ref().unwrap().range)
        .collect();
    assert_eq!(ranges, [r(0.0, 10.0), r(10.0, 20.0), r(20.0, 30.0)]);
    assert!(specs.iter().all(|spec| spec.prompt == "Rain"
        && spec.resolution == Resolution::P1080
        && spec.audio == AudioOption::FadeOut
        && spec.references == specs[0].references
        && !spec.is_draft));
}

#[test]
fn draft_length_applies_to_every_range() {
    let mut form = three_ranges(Resolution::P720);
    form.draft_length = DraftLength::FirstFiveSeconds;
    let drafts = form.make_specs(true);
    let ranges: Vec<TimeRange> = drafts
        .iter()
        .map(|spec| spec.source.as_ref().unwrap().range)
        .collect();
    assert_eq!(ranges, [r(0.0, 5.0), r(10.0, 15.0), r(20.0, 25.0)]);
    assert!(
        drafts
            .iter()
            .all(|spec| spec.is_draft && spec.resolution == Resolution::P480)
    );
    assert!((form.draft_duration() - 15.0).abs() < 1e-9);
    assert_eq!(form.draft_length_detail(&api_key()), "15.0 s · $4.77");
}

#[test]
fn draft_length_shows_when_any_range_is_longer_than_the_short_draft() {
    let mut form = ready(Resolution::P720);
    form.set_source(Some(video(20.0)));
    form.set_end(4.5);
    form.add_range();
    assert_eq!(form.ranges(), [r(0.0, 4.5), r(4.5, 9.0)]);
    assert!(!form.shows_draft_length());
    form.set_end(12.0);
    assert!(form.shows_draft_length());
}

#[test]
fn each_job_is_billed_in_whole_seconds() {
    let mut form = ready(Resolution::P720);
    form.set_source(Some(video(20.0)));
    form.set_end(4.5);
    form.add_range();
    assert_eq!(form.billed_seconds(Some(ProviderKind::ApiKey)), 10);
    assert_eq!(form.cost_line(&api_key()).unwrap().text, "10 s × $0.681 = $6.81");
}

#[test]
fn cost_line_and_buttons_show_the_total() {
    let form = three_ranges(Resolution::P720);
    let line = form.cost_line(&api_key()).unwrap();
    assert_eq!(line.text, "30 s × $0.681 = $20.43");
    assert_eq!(line.detail, "List price · 3 jobs · failed runs are free");
    assert_eq!(form.price_text(false, &api_key()).as_deref(), Some("$20.43"));
    assert_eq!(form.price_text(true, &api_key()).as_deref(), Some("$9.54"));
}

#[test]
fn credits_total_needs_every_estimate() {
    let form = three_ranges(Resolution::P720);
    let specs = form.make_specs(false);
    let mut context = account(Some(100));
    quote(&mut context, &specs[0], credits(40, true));
    quote(&mut context, &specs[1], credits(40, true));
    assert_eq!(form.price_text(false, &context), None);
    assert_eq!(form.blocker(false, &context), None);
    quote(&mut context, &specs[2], credits(40, true));
    assert_eq!(form.cost_line(&context).unwrap().text, "30 s = 120 credits");
    assert_eq!(
        form.blocker(false, &context).as_deref(),
        Some("Not enough credits for Generate, try a draft")
    );
}

#[test]
fn mixed_units_have_no_total() {
    let specs = three_ranges(Resolution::P720).make_specs(false);
    let mut context = api_key();
    quote(&mut context, &specs[0], credits(40, true));
    assert_eq!(context.cost_of(&specs), None);
    assert_eq!(context.cost_of(&[]), None);
    assert_eq!(
        context.cost_of(&specs[1..]),
        Some(Cost::usd("13.62".parse().unwrap(), false))
    );
}

#[test]
fn spend_cap_checks_the_sum_and_asks_once() {
    let mut context = api_key();
    context.committed_today = Decimal::from(60);
    context.cap = Some(SpendCap::usd(Decimal::from(75)));
    let mut single = three_ranges(Resolution::P720);
    single.remove_range(0);
    single.remove_range(0);
    assert_eq!(
        single.submit_decision(false, &context),
        SubmitDecision::Start(single.make_specs(false))
    );
    let form = three_ranges(Resolution::P720);
    assert!(form.cost_line(&context).unwrap().is_warning);
    let SubmitDecision::Confirm(specs, confirmation) = form.submit_decision(false, &context) else {
        panic!("expected one confirmation for all ranges");
    };
    assert_eq!(specs, form.make_specs(false));
    assert_eq!(specs.len(), 3);
    assert_eq!(
        confirmation.message,
        "These 3 jobs add $20.43, bringing today to $80.43 against your $75.00 cap."
    );
}

#[test]
fn clamp_cuts_references_and_brings_ranges_inside_the_limits() {
    let mut form = CreateForm::default();
    form.set_source(Some(video(60.0)));
    form.add_references(images(1..=4));
    form.set_end(20.0);
    form.add_range();
    form.set_start(25.0);
    form.set_end(27.0);
    assert_eq!(form.ranges(), [r(0.0, 20.0), r(25.0, 29.0)]);
    let mut limits = ModeLimits::new(ClosedRange::new(1, 2), false);
    limits.duration = ClosedRange::new(6.0, 12.0);
    form.clamp_to(limits);
    assert_eq!(form.references(), images([1, 2]));
    assert_eq!(form.ranges(), [r(0.0, 12.0), r(25.0, 31.0)]);
}

// MARK: Keyboard

// `CreateFormKeyboardTests`.

/// 0–30 s of a 47.3 s clip.
fn long_clip() -> CreateForm {
    let mut form = ready(Resolution::P720);
    form.set_source(Some(video(47.3)));
    form
}

#[test]
fn handles_move_by_a_tenth_of_a_second() {
    let mut form = long_clip();
    form.nudge(TrimPart::Start, 1);
    assert_eq!(form.range(), r(0.1, 30.0));
    form.nudge(TrimPart::End, -2);
    assert_eq!(form.range(), r(0.1, 29.8));
    assert!((form.time_of(TrimPart::Start) - 0.1).abs() < f64::EPSILON);
    assert!((form.time_of(TrimPart::End) - 29.8).abs() < f64::EPSILON);
}

#[test]
fn selection_moves_whole() {
    let mut form = long_clip();
    form.set_end(10.0);
    form.nudge(TrimPart::Selection, 3);
    assert!((form.range().lower() - 0.3).abs() < f64::EPSILON);
    assert!((form.range().upper() - 10.3).abs() < 1e-9);
    assert!((form.time_of(TrimPart::Selection) - 0.3).abs() < f64::EPSILON);
    form.nudge(TrimPart::Selection, -5);
    assert!(form.range().lower().abs() < f64::EPSILON);
    assert!((form.selected_duration() - 10.0).abs() < 1e-9);
}

#[test]
fn nudges_are_clamped_like_a_drag() {
    let mut form = long_clip();
    form.nudge(TrimPart::Start, -1);
    assert_eq!(form.range(), r(0.0, 30.0));
    // The 30 s window: the end can't move out while the start stays.
    form.nudge(TrimPart::End, 1);
    assert_eq!(form.range(), r(0.0, 30.0));
    // Past the clip's end.
    form.move_range(40.0);
    form.nudge(TrimPart::End, 1);
    assert!((form.range().upper() - 47.3).abs() < f64::EPSILON);
    form.nudge(TrimPart::Selection, 1);
    assert!((form.range().upper() - 47.3).abs() < f64::EPSILON);
}

#[test]
fn steps_land_on_the_grid() {
    let mut form = long_clip();
    form.set_end(10.0);
    for _ in 0..7 {
        form.nudge(TrimPart::End, 1);
    }
    for _ in 0..7 {
        form.nudge(TrimPart::End, -1);
    }
    assert_eq!(form.range(), r(0.0, 10.0));
    assert_eq!(form.billed_seconds(Some(ProviderKind::ApiKey)), 10);
    // An edge left between steps by a drag snaps to the grid in the direction of the step.
    form.set_end(20.0);
    form.set_start(6.04);
    form.nudge(TrimPart::Start, 1);
    assert!((form.range().lower() - 6.1).abs() < f64::EPSILON);
    form.set_start(6.06);
    form.nudge(TrimPart::Start, -1);
    assert!((form.range().lower() - 6.0).abs() < f64::EPSILON);
}

#[test]
fn nothing_moves_without_a_source() {
    let mut form = CreateForm::default();
    form.nudge(TrimPart::End, 5);
    assert_eq!(form.range(), r(0.0, 0.0));
}

#[test]
fn primary_submit_is_generate_outside_a_batch() {
    let form = ready(Resolution::P720);
    assert_eq!(
        form.primary_submit(&api_key(), false),
        PrimarySubmit {
            title: "Generate".to_owned(),
            is_draft: false,
            is_enabled: true
        }
    );
    assert!(!form.primary_submit(&api_key(), true).is_enabled);
}

#[test]
fn primary_submit_is_disabled_while_blocked() {
    let mut form = ready(Resolution::P720);
    form.set_source(None);
    assert!(!form.primary_submit(&api_key(), false).is_enabled);
    let disconnected = CreateContext {
        pricing: Some(Arc::new(Pricing::bundled().unwrap())),
        ..CreateContext::default()
    };
    assert!(
        !ready(Resolution::P720)
            .primary_submit(&disconnected, false)
            .is_enabled
    );
}

#[test]
fn primary_submit_is_the_draft_in_a_batch() {
    let mut form = ready(Resolution::P720);
    form.set_batch(true);
    let submit = form.primary_submit(&api_key(), false);
    assert!(submit.is_draft);
    assert_eq!(submit.title, form.draft_button_title());
}

// MARK: References

#[test]
fn reference_limits_follow_the_mode() {
    let mut form = form(Mode::MotionTransfer, Resolution::P720);
    assert_eq!(form.add_references(images(1..=10)), 8);
    assert!(!form.can_add_references());
    assert_eq!(form.reference_count_text(), "8 / 8");
    form.set_mode(Mode::Restyle);
    assert_eq!(form.references().len(), 5);
    assert_eq!(form.reference_count_text(), "5 / 5");
}

/// `CreateForm.addOpenedReferences`, as a linked or opened image reaches it (`CreateViewLinks.swift`).
#[test]
fn opened_references_beyond_the_limit_say_what_the_mode_takes() {
    let mut form = form(Mode::Restyle, Resolution::P720);
    assert_eq!(form.add_opened_references(images(1..=4)), None);
    // Already added: skipped silently.
    assert_eq!(form.add_opened_references(images([1])), None);
    assert_eq!(form.add_opened_references(images([5])), None);
    let alert = form.add_opened_references(images([6])).unwrap();
    assert_eq!(alert.title, "1 image wasn't added");
    assert_eq!(alert.message, "Restyle takes up to 5 reference images.");
    let alert = form.add_opened_references(images([7, 8])).unwrap();
    assert_eq!(alert.title, "2 images weren't added");
}

#[test]
fn duplicates_are_skipped_and_the_list_reorders() {
    let mut form = CreateForm::default();
    form.add_references(images([1, 1, 2, 3]));
    assert_eq!(form.references(), images([1, 2, 3]));
    form.move_reference(0, 2);
    assert_eq!(form.references(), images([2, 3, 1]));
    form.move_reference(2, 0);
    form.remove_reference(1);
    form.remove_reference(9);
    assert_eq!(form.references(), images([1, 3]));
}

// MARK: Blockers

#[test]
fn connection_and_source_come_first() {
    let disconnected = CreateContext::default();
    assert_eq!(
        ready(Resolution::P720).blocker(false, &disconnected).as_deref(),
        Some("Connect Higgsfield in Settings")
    );
    assert_eq!(
        CreateForm::default().blocker(false, &api_key()).as_deref(),
        Some("Add a source video")
    );
    assert_eq!(
        CreateForm::default().submit_decision(false, &api_key()),
        SubmitDecision::Blocked("Add a source video".to_owned())
    );
}

#[test]
fn a_mode_the_connection_cant_run_says_why() {
    let mut context = api_key();
    context.capabilities = Some(Capabilities::new(
        BTreeSet::from([Mode::Restyle]),
        Resolution::ALL.iter().copied().collect(),
        CostUnit::Usd,
        None,
    ));
    assert_eq!(
        ready(Resolution::P720).blocker(false, &context).as_deref(),
        Some("Not available with this API key")
    );
    assert_eq!(
        context.mode_reason(Mode::MotionTransfer).as_deref(),
        Some("Not available with this API key")
    );
    assert_eq!(
        CreateContext::default().mode_reason(Mode::Restyle).as_deref(),
        Some("Connect Higgsfield first")
    );
    assert_eq!(
        account(None).mode_reason(Mode::Restyle).as_deref(),
        Some("Checking what this connection can run…")
    );
}

#[test]
fn the_account_offers_genjutsu_until_its_models_are_known() {
    let mut context = account(None);
    assert_eq!(
        ready(Resolution::P720).model_options(&context),
        [VideoModel::GENJUTSU]
    );
    let mut capabilities = Capabilities::new(
        BTreeSet::from([Mode::MotionTransfer, Mode::ObjectSwap]),
        Resolution::ALL.iter().copied().collect(),
        CostUnit::Credits,
        None,
    );
    capabilities.models = VideoModel::all().into_iter().collect();
    context.capabilities = Some(capabilities);
    let expected: Vec<VideoModel> = VideoModel::all()
        .into_iter()
        .filter(|model| model.serves(Mode::MotionTransfer))
        .collect();
    assert_eq!(ready(Resolution::P720).model_options(&context), expected);
}

#[test]
fn validation_issues_read_as_actions() {
    let mut form = CreateForm::default();
    form.set_source(Some(video(14.2)));
    assert_eq!(
        form.blocker(false, &api_key()).as_deref(),
        Some("Add at least one image")
    );
    form.set_mode(Mode::Restyle);
    assert_eq!(form.blocker(false, &api_key()).as_deref(), Some("Pick a style"));
    form.set_preset_id(Some(Uuid::new_v4()));
    assert_eq!(form.blocker(false, &api_key()), None);
    form.prompt = "a".repeat(10_001);
    assert_eq!(form.prompt_counter_text(), "10001 / 10 000");
    assert_eq!(
        form.blocker(false, &api_key()).as_deref(),
        Some("Shorten the prompt to 10 000 characters")
    );
}

#[test]
fn a_small_swap_frame_needs_the_upscale() {
    let mut form = form(Mode::ObjectSwap, Resolution::P720);
    form.set_source(Some(video_named(11.4, 720, 480, "avc1", "clip.mp4")));
    form.add_references(images([1]));
    assert_eq!(
        form.blocker(false, &api_key()).as_deref(),
        Some("Object swap needs 640×640 px, allow the upscale")
    );
    let notice = form.upscale_notice().unwrap();
    assert_eq!(
        notice.title,
        "720×480 is below the 640×640 minimum for object swap."
    );
    assert!(notice.detail.starts_with("Upscale to 784×524 before upload?"));
    form.allows_upscale = true;
    assert_eq!(form.blocker(false, &api_key()), None);
    assert!(form.stage_pills().contains(&"Will upscale to 784×524".to_owned()));
    assert!(form.make_specs(false)[0].source.as_ref().unwrap().allows_upscale);
    form.set_mode(Mode::MotionTransfer);
    assert!(!form.allows_upscale);
    assert_eq!(form.upscale_notice(), None);
}

#[test]
fn credits_are_checked_against_each_action_s_own_cost() {
    let form = ready(Resolution::P720);
    let draft = form.make_specs(true).remove(0);
    let full = form.make_specs(false).remove(0);
    let mut context = account(Some(50));
    quote(&mut context, &draft, credits(60, true));
    assert_eq!(
        form.blocker(true, &context).as_deref(),
        Some("Not enough credits on your plan")
    );
    quote(&mut context, &draft, credits(40, true));
    quote(&mut context, &full, credits(96, true));
    assert_eq!(form.blocker(true, &context), None);
    let reason = "Not enough credits for Generate, try a draft";
    assert_eq!(form.blocker(false, &context).as_deref(), Some(reason));
    assert_eq!(
        form.submit_decision(false, &context),
        SubmitDecision::Blocked(reason.to_owned())
    );
    assert_eq!(
        form.submit_decision(true, &context),
        SubmitDecision::Start(vec![draft])
    );
}

// MARK: Prices

#[test]
fn the_usd_cost_line_uses_the_list_price() {
    let mut form = ready(Resolution::P720);
    form.set_source(Some(video(47.3)));
    form.set_end(12.4);
    let line = form.cost_line(&api_key()).unwrap();
    assert_eq!(line.text, "13 s × $0.681 = $8.85");
    assert_eq!(line.detail, "List price · failed runs are free");
    assert!(!line.is_warning);
    assert_eq!(form.price_text(false, &api_key()).as_deref(), Some("$8.85"));
    assert_eq!(form.rate_line(&api_key()).as_deref(), Some("$0.681 / s"));
}

#[test]
fn over_the_cap_warns() {
    let mut context = api_key();
    context.committed_today = Decimal::from(70);
    context.cap = Some(SpendCap::usd(Decimal::from(75)));
    let line = ready(Resolution::P720).cost_line(&context).unwrap();
    assert_eq!(line.detail, "Over today's cap, you'll be asked to confirm");
    assert!(line.is_warning);
}

#[test]
fn the_credits_cost_line_uses_the_quote() {
    let form = ready(Resolution::P720);
    let spec = form.make_specs(false).remove(0);
    let mut context = account(Some(1240));
    assert_eq!(form.cost_line(&context).unwrap().text, "15 s · estimating…");
    assert_eq!(form.price_text(false, &context), None);
    assert_eq!(form.rate_line(&context), None);
    quote(&mut context, &spec, credits(120, true));
    let line = form.cost_line(&context).unwrap();
    assert_eq!(line.text, "15 s = 120 credits");
    assert_eq!(
        line.detail,
        "Estimate, Higgsfield quotes it after upload · 1240 cr left on your plan · failed runs are free"
    );
    assert_eq!(form.price_text(false, &context).as_deref(), Some("120 cr"));
    quote(&mut context, &spec, credits(105, false));
    assert_eq!(
        form.cost_line(&context).unwrap().detail,
        "Price from Higgsfield · 1240 cr left on your plan · failed runs are free"
    );
}

#[test]
fn an_unknown_price_says_so() {
    let form = ready(Resolution::P720);
    let spec = form.make_specs(false).remove(0);
    let mut context = account(None);
    quote(&mut context, &spec, Cost::UNKNOWN_CREDITS);
    let line = form.cost_line(&context).unwrap();
    assert_eq!(line.text, "15 s · Price unknown");
    assert_eq!(
        line.detail,
        "Higgsfield quotes it once the files are uploaded · failed runs are free"
    );
}

#[test]
fn the_last_known_price_shows_while_a_changed_form_is_quoted() {
    let mut form = ready(Resolution::P720);
    let spec = form.make_specs(false).remove(0);
    let mut cache = EstimateCache::default();
    cache.record(
        vec![(price_key(&spec), credits(120, true))],
        std::slice::from_ref(&spec),
        &[],
    );
    let mut context = account(None);
    context.estimates = cache;
    form.prompt = "Typing reuses the price".to_owned();
    assert_eq!(form.price_text(false, &context).as_deref(), Some("120 cr"));
    form.set_end(10.0);
    assert_eq!(context.estimates.missing(&form.make_specs(false)).len(), 1);
    assert_eq!(form.price_text(false, &context).as_deref(), Some("120 cr"));
    assert_eq!(
        form.cost_line(&context).unwrap().detail,
        "Updating the price · failed runs are free"
    );
}

#[test]
fn the_draft_is_480p_and_can_be_shortened() {
    let mut form = ready(Resolution::P1080);
    assert!(form.shows_draft_length());
    assert_eq!(form.price_text(true, &api_key()).as_deref(), Some("$4.77"));
    form.draft_length = DraftLength::FirstFiveSeconds;
    assert!((form.draft_duration() - 5.0).abs() < 1e-9);
    assert_eq!(form.draft_length_detail(&api_key()), "5.0 s · $1.59");
    let draft = form.make_specs(true).remove(0);
    assert_eq!(draft.resolution, Resolution::P480);
    assert!(draft.is_draft);
    assert_eq!(draft.source.unwrap().range, r(0.0, 5.0));
    assert_eq!(
        form.make_specs(false)[0].source.as_ref().unwrap().range,
        r(0.0, 14.2)
    );
    assert_eq!(form.draft_title(), "Draft 480p");
}

#[test]
fn the_draft_length_hides_for_short_ranges() {
    let mut form = ready(Resolution::P720);
    form.set_source(Some(video(4.5)));
    form.draft_length = DraftLength::FirstFiveSeconds;
    assert!(!form.shows_draft_length());
    assert!((form.draft_duration() - 4.5).abs() < 1e-9);
}

// MARK: Specs and loading

#[test]
fn a_spec_carries_the_form_and_the_preset_only_for_restyle() {
    let mut form = form(Mode::Restyle, Resolution::P720);
    form.set_source(Some(video(47.3)));
    form.set_start(6.0);
    form.add_references(images([1]));
    let preset = Uuid::new_v4();
    form.set_preset_id(Some(preset));
    form.prompt = "Rain".to_owned();
    form.audio = AudioOption::Mute;
    let spec = form.make_specs(false).remove(0);
    assert_eq!(spec.mode, Mode::Restyle);
    assert_eq!(spec.source.as_ref().unwrap().range, r(6.0, 30.0));
    assert_eq!(spec.references, images([1]));
    assert_eq!(spec.preset_id, Some(preset));
    assert_eq!(spec.resolution, Resolution::P720);
    assert_eq!(spec.audio, AudioOption::Mute);
    let mut motion = ready(Resolution::P720);
    motion.set_preset_id(Some(Uuid::new_v4()));
    assert_eq!(motion.make_specs(false)[0].preset_id, None);
    assert_eq!(CreateForm::default().make_specs(false), Vec::new());
}

#[test]
fn loading_a_spec_restores_the_form() {
    let mut original = form(Mode::ObjectSwap, Resolution::P1080);
    let small = video_named(47.3, 720, 480, "avc1", "clip.mp4");
    original.set_source(Some(small.clone()));
    original.set_start(6.0);
    original.set_end(18.5);
    original.add_references(images([1, 2]));
    original.prompt = "Green sneaker".to_owned();
    original.allows_upscale = true;
    original.audio = AudioOption::FadeOut;
    let spec = original.make_specs(false).remove(0);

    let mut form = ready(Resolution::P720);
    form.load(&spec, Some(&small), None);
    assert_eq!(form.make_specs(false)[0], spec);

    form.load(&spec, None, None);
    assert!(form.source().is_none());
    assert_eq!(form.prompt, "Green sneaker");
    assert_eq!(form.references().len(), 2);
}

#[test]
fn a_loaded_item_stays_the_parent_until_the_source_changes() {
    let spec = ready(Resolution::P720).make_specs(false).remove(0);
    let parent = Uuid::new_v4();
    let mut form = ready(Resolution::P720);
    form.load(&spec, Some(&video(14.2)), Some(parent));
    assert_eq!(form.parent_id(), Some(parent));
    form.set_source(Some(video(12.0)));
    assert_eq!(form.parent_id(), None);
}

#[test]
fn a_model_the_connection_doesnt_offer_falls_back_to_genjutsu() {
    let mut form = ready(Resolution::P720);
    form.set_model(VideoModel::SEEDANCE25);
    let notice = form
        .fit_model(Some(&BTreeSet::from([VideoModel::GENJUTSU])))
        .unwrap();
    assert_eq!(notice.title, "Running on Genjutsu");
    assert_eq!(form.model(), &VideoModel::GENJUTSU);
    assert_eq!(form.fit_model(None), None);
}

#[test]
fn conversion_pill_and_file_text() {
    let mut form = CreateForm::default();
    form.set_source(Some(video_named(14.2, 1920, 1080, "hvc1", "IMG.MOV")));
    assert_eq!(form.stage_pills(), ["Will convert to MP4 · H.264"]);
    assert_eq!(form.source().unwrap().file_text(), "38 MB · HEVC");
    assert!(!form.source().unwrap().plays_in_webview());
    form.set_source(Some(video(14.2)));
    assert_eq!(form.stage_pills(), Vec::<String>::new());
}

// MARK: Submit decision

#[test]
fn under_the_cap_it_starts_and_over_it_asks() {
    let form = ready(Resolution::P720);
    let mut context = api_key();
    context.cap = Some(SpendCap::usd(Decimal::from(75)));
    assert_eq!(
        form.submit_decision(false, &context),
        SubmitDecision::Start(form.make_specs(false))
    );
    context.committed_today = Decimal::from(70);
    let SubmitDecision::Confirm(specs, confirmation) = form.submit_decision(false, &context) else {
        panic!("expected a confirmation");
    };
    assert_eq!(specs, form.make_specs(false));
    assert_eq!(confirmation.title, "Over today's spend cap");
    assert_eq!(
        confirmation.message,
        "This job adds $10.22, bringing today to $80.22 against your $75.00 cap."
    );
    assert_eq!(confirmation.confirm_title, "Start anyway");
    assert_eq!(
        form.submit_decision(true, &context),
        SubmitDecision::Start(form.make_specs(true))
    );
}

#[test]
fn an_unknown_price_under_a_cap_asks() {
    let form = ready(Resolution::P720);
    let mut context = account(None);
    context.cap = Some(SpendCap::credits(1_000));
    let SubmitDecision::Confirm(_, confirmation) = form.submit_decision(false, &context) else {
        panic!("expected a confirmation");
    };
    assert_eq!(confirmation.title, "Price not known yet");
}

#[test]
fn a_credit_cap_asks_for_an_account() {
    let form = ready(Resolution::P720);
    let spec = form.make_specs(false).remove(0);
    let mut context = account(Some(5_000));
    quote(&mut context, &spec, credits(120, true));
    context.committed_today = Decimal::from(900);
    context.cap = Some(SpendCap::credits(1_000));
    assert!(form.cost_line(&context).unwrap().is_warning);
    let SubmitDecision::Confirm(_, confirmation) = form.submit_decision(false, &context) else {
        panic!("expected a confirmation");
    };
    assert_eq!(
        confirmation.message,
        "This job adds 120 credits, bringing today to 1020 credits against your 1000 credits cap."
    );
}

// MARK: Variants

#[test]
fn variants_follow_the_connection() {
    let mut form = ready(Resolution::P720);
    form.set_variants(9);
    assert_eq!(form.variants(), 4);
    let mut context = api_key();
    assert_eq!(form.variants_in(&context), 1);
    let mut capabilities = Capabilities::new(
        BTreeSet::from([Mode::MotionTransfer, Mode::Restyle]),
        Resolution::ALL.iter().copied().collect(),
        CostUnit::Usd,
        None,
    );
    capabilities.max_variants = 1;
    context.capabilities = Some(capabilities.clone());
    assert_eq!(
        form.variants_unavailable_reason(&context).as_deref(),
        Some("This connection makes one video per job")
    );
    capabilities.max_variants = 2;
    context.capabilities = Some(capabilities);
    assert_eq!(form.variants_in(&context), 2);
    assert!(
        form.specs_in(false, &context)
            .iter()
            .all(|spec| spec.variants == 2)
    );
    form.set_mode(Mode::Restyle);
    assert_eq!(
        form.variants_unavailable_reason(&context).as_deref(),
        Some("A style makes one video per run")
    );
}

// MARK: Draft persistence

#[test]
fn the_draft_round_trips_and_drops_files_that_are_gone() {
    let folder = tempfile::tempdir().unwrap();
    let clip = folder.path().join("clip.mp4");
    let picture = folder.path().join("ref.png");
    std::fs::write(&clip, b"video").unwrap();
    std::fs::write(&picture, b"image").unwrap();
    let mut form = form(Mode::MotionTransfer, Resolution::P720);
    let mut source = video(14.2);
    source.file_url = crate::testing::file_url(&clip);
    form.set_source(Some(source));
    form.add_references(vec![
        LocalImage::new(crate::testing::file_url(&picture)),
        image(9),
    ]);
    form.prompt = "Rain".to_owned();
    let store = CreateDraftStore::new(folder.path().join("create-draft.json"));
    store.save(Some(&form)).unwrap();

    assert_eq!(store.load().unwrap(), form);
    let restored = store.restore().unwrap();
    assert!(restored.source().is_some());
    assert_eq!(restored.references().len(), 1);

    std::fs::remove_file(&clip).unwrap();
    let restored = store.restore().unwrap();
    assert!(restored.source().is_none());
    assert_eq!(restored.ranges(), [r(0.0, 0.0)]);

    store.save(Some(&CreateForm::default())).unwrap();
    assert_eq!(store.restore(), None);
    std::fs::write(store.path(), r#"{"version":7}"#).unwrap();
    assert_eq!(store.load(), None);
}

#[test]
fn a_replacement_is_kept_only_when_inputs_are_lost_and_the_draft_was_edited() {
    let mut previous = ready(Resolution::P720);
    previous.prompt = "Old".to_owned();
    let mut replaced = previous.clone();
    replaced.prompt = "New".to_owned();
    assert_eq!(replacement(&previous, &replaced, None), Some(previous.clone()));
    assert_eq!(replacement(&previous, &replaced, Some(&previous)), None);
    let mut filled = CreateForm::default();
    filled.prompt = "First".to_owned();
    assert_eq!(replacement(&CreateForm::default(), &filled, None), None);
    assert!(previous.has_inputs());
    assert!(!CreateForm::default().has_inputs());
}

// MARK: Snapshot

// MARK: Templates (`CreateFormTemplateTests`)

fn template(name: &str, mode: Mode, model: VideoModel, resolution: Resolution) -> jutsu_core::JobTemplate {
    jutsu_core::JobTemplate::new(
        Uuid::new_v4(),
        name,
        time::OffsetDateTime::UNIX_EPOCH,
        mode,
        model,
        resolution,
    )
}

fn all_models() -> BTreeSet<VideoModel> {
    VideoModel::all().into_iter().collect()
}

#[test]
fn template_keeps_the_inputs_without_the_source() {
    let mut form = ready(Resolution::P1080);
    form.set_mode(Mode::Restyle);
    let preset = Uuid::new_v4();
    form.set_preset_id(Some(preset));
    form.prompt = "rainy neon".to_owned();
    form.audio = AudioOption::FadeOut;
    let template = form.make_template("Neon", Uuid::new_v4(), time::OffsetDateTime::UNIX_EPOCH);
    assert_eq!(template.name, "Neon");
    assert_eq!(template.mode, Mode::Restyle);
    assert_eq!(template.references, form.references());
    assert_eq!(template.preset_id, Some(preset));
    assert_eq!(template.prompt, "rainy neon");
    assert_eq!(template.resolution, Resolution::P1080);
    assert_eq!(template.audio, AudioOption::FadeOut);
}

#[test]
fn apply_fills_the_inputs_and_keeps_source_and_ranges() {
    let mut form = CreateForm::default();
    form.set_source(Some(video(40.0)));
    form.add_range();
    form.set_end(20.0);
    let ranges = form.ranges().to_vec();
    let source = form.source().cloned();
    let preset = Uuid::new_v4();
    let mut clay = template("Clay", Mode::Restyle, VideoModel::GENJUTSU, Resolution::P720);
    clay.references = images([1]);
    clay.preset_id = Some(preset);
    clay.prompt = "clay".to_owned();
    clay.audio = AudioOption::Mute;

    form.apply_template(&clay, &all_models());

    assert_eq!(form.source().cloned(), source);
    assert_eq!(form.ranges(), ranges.as_slice());
    assert_eq!(form.mode(), Mode::Restyle);
    assert_eq!(form.references(), clay.references.as_slice());
    assert_eq!(form.preset_id(), Some(preset));
    assert_eq!(form.prompt, "clay");
    assert_eq!(form.resolution(), Resolution::P720);
    assert_eq!(form.audio, AudioOption::Mute);
}

#[test]
fn apply_without_a_source_leaves_only_the_video_to_add() {
    let mut form = CreateForm::default();
    let mut dance = template(
        "Dance",
        Mode::MotionTransfer,
        VideoModel::GENJUTSU,
        Resolution::P720,
    );
    dance.references = images([1]);
    form.apply_template(&dance, &all_models());
    assert!(form.source().is_none());
    assert_eq!(
        form.blocker(false, &api_key()).as_deref(),
        Some("Add a source video")
    );
}

#[test]
fn apply_replaces_references_and_keeps_a_preset_a_non_restyle_template_has_none() {
    let mut form = ready(Resolution::P720);
    form.add_references(images([2]));
    let preset = Uuid::new_v4();
    form.set_preset_id(Some(preset));
    let mut swap = template("Swap", Mode::ObjectSwap, VideoModel::GENJUTSU, Resolution::P480);
    swap.references = images([7]);
    form.apply_template(&swap, &all_models());
    assert_eq!(form.references(), images([7]).as_slice());
    assert_eq!(form.preset_id(), Some(preset));
}

#[test]
fn apply_cuts_references_to_the_mode_limit() {
    let mut form = CreateForm::default();
    let mut many = template("Many", Mode::Restyle, VideoModel::GENJUTSU, Resolution::P720);
    many.references = images(1..=7);
    form.apply_template(&many, &all_models());
    assert_eq!(form.references().len(), 5);
}

#[test]
fn template_keeps_the_selected_model() {
    let mut form = ready(Resolution::P720);
    form.set_model(VideoModel::KLING_MOTION_CONTROL);
    let now = time::OffsetDateTime::UNIX_EPOCH;
    assert_eq!(
        form.make_template("Dance", Uuid::new_v4(), now).model,
        VideoModel::KLING_MOTION_CONTROL
    );
    form.set_mode(Mode::ObjectSwap);
    assert_eq!(
        form.make_template("Swap", Uuid::new_v4(), now).model,
        VideoModel::GENJUTSU
    );
}

#[test]
fn apply_uses_the_template_model_when_the_connection_offers_it() {
    let mut form = ready(Resolution::P720);
    let dance = template(
        "Dance",
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        Resolution::P720,
    );
    assert_eq!(form.apply_template(&dance, &all_models()), None);
    assert_eq!(form.selected_model(), VideoModel::SEEDANCE25);
    let models: Vec<VideoModel> = form
        .make_specs(false)
        .into_iter()
        .map(|spec| spec.model)
        .collect();
    assert_eq!(models, vec![VideoModel::SEEDANCE25]);
}

#[test]
fn apply_falls_back_to_genjutsu_with_a_notice() {
    let mut form = ready(Resolution::P720);
    form.set_model(VideoModel::SEEDANCE25);
    let mut dance = template(
        "Dance",
        Mode::MotionTransfer,
        VideoModel::KLING_MOTION_CONTROL,
        Resolution::P1080,
    );
    dance.references = images([3]);
    dance.prompt = "spin".to_owned();

    let notice = form.apply_template(&dance, &BTreeSet::from([VideoModel::GENJUTSU]));

    assert_eq!(form.model(), &VideoModel::GENJUTSU);
    let notice = notice.unwrap();
    assert_eq!(notice.title, "Running on Genjutsu");
    assert_eq!(
        notice.detail,
        "\"Dance\" uses Kling 3.0, which this connection doesn't offer."
    );
    assert_eq!(form.prompt, "spin");
    assert_eq!(form.references(), dance.references.as_slice());
    assert_eq!(form.resolution(), Resolution::P1080);
}

// MARK: Submit bar (`CreatePanel.SubmitBar`)

#[test]
fn the_submit_bar_shows_the_first_blocker_instead_of_the_cost_line() {
    let view = snapshot(&CreateForm::default(), &api_key(), &MediaState::default());
    assert_eq!(view.submit.blocker.as_deref(), Some("Add a source video"));
    assert!(view.submit.cost_line.is_none());
    let view = snapshot(&ready(Resolution::P720), &api_key(), &MediaState::default());
    assert_eq!(view.submit.blocker, None);
    assert!(view.submit.cost_line.is_some());
}

// MARK: Batch (`BatchPlanningTests`, `BatchCapTests`, `BatchPriceTests`)

fn preset_ids() -> Vec<Uuid> {
    (0..13).map(|index| Uuid::from_u128(index + 1)).collect()
}

/// Restyle on a 13 s clip, so every job bills 13 s.
fn restyle_batch(styles: usize) -> CreateForm {
    let mut form = form(Mode::Restyle, Resolution::P720);
    form.set_source(Some(video(13.0)));
    form.set_batch(true);
    for id in preset_ids().into_iter().take(styles) {
        form.toggle_batch_preset(id);
    }
    form
}

#[test]
fn restyle_batch_queues_the_range_once_per_style_as_drafts() {
    let form = restyle_batch(3);
    let specs = form.make_specs(true);
    let presets: Vec<Option<Uuid>> = specs.iter().map(|spec| spec.preset_id).collect();
    assert_eq!(
        presets,
        preset_ids().into_iter().take(3).map(Some).collect::<Vec<_>>()
    );
    assert!(specs.iter().all(|spec| {
        spec.source
            .as_ref()
            .is_some_and(|clip| clip.range == ClosedRange::new(0.0, 13.0))
            && spec.resolution == Resolution::P480
            && spec.is_draft
    }));
    assert_eq!(form.draft_button_title(), "Draft 480p × 3");
    assert_eq!(form.generate_button_title(), "Generate × 3");
    assert_eq!(form.batch_count_text(), "3 / 12");
}

#[test]
fn character_batch_queues_one_job_per_set() {
    let mut form = ready(Resolution::P720);
    form.set_batch(true);
    assert_eq!(form.character_sets(), &[images([1])]);
    assert_eq!(form.add_character_set(&images(2..4)), None);
    let specs = form.make_specs(false);
    let references: Vec<Vec<LocalImage>> = specs.iter().map(|spec| spec.references.clone()).collect();
    assert_eq!(references, vec![images([1]), images(2..4)]);
    assert!(
        specs
            .iter()
            .all(|spec| spec.resolution == Resolution::P720 && !spec.is_draft)
    );
    assert_eq!(form.blocker(true, &api_key()), None);
}

#[test]
fn turning_the_batch_on_starts_from_the_chosen_style() {
    let mut form = form(Mode::Restyle, Resolution::P720);
    form.set_preset_id(Some(preset_ids()[0]));
    form.set_batch(true);
    assert_eq!(form.batch_preset_ids(), &[preset_ids()[0]]);
    form.toggle_batch_preset(preset_ids()[0]);
    assert_eq!(form.batch_preset_ids().len(), 0);
}

#[test]
fn a_character_set_keeps_eight_distinct_images() {
    let mut form = ready(Resolution::P720);
    form.set_batch(true);
    let mut picked = images(10..20);
    picked.extend(images(10..11));
    let notice = form.add_character_set(&picked);
    assert_eq!(form.character_sets().last(), Some(&images(10..18)));
    assert_eq!(notice.unwrap().title, "2 images weren't added");
    assert_eq!(form.add_character_set(&[]), None);
    form.remove_character_set(0);
    assert_eq!(form.character_sets(), &[images(10..18)]);
}

#[test]
fn loading_a_job_leaves_the_batch() {
    let mut form = restyle_batch(2);
    let spec = form.make_specs(true).remove(1);
    form.load(&spec, Some(&video(13.0)), None);
    assert!(!form.is_batch());
    let presets: Vec<Option<Uuid>> = form.make_specs(false).iter().map(|spec| spec.preset_id).collect();
    assert_eq!(presets, vec![Some(preset_ids()[1])]);
}

#[test]
fn batch_blockers() {
    let context = api_key();
    assert_eq!(
        restyle_batch(1).blocker(true, &context).as_deref(),
        Some("Pick at least 2 styles")
    );
    let mut characters = ready(Resolution::P720);
    characters.set_batch(true);
    assert_eq!(
        characters.blocker(true, &context).as_deref(),
        Some("Add at least 2 characters")
    );

    let mut ranges = restyle_batch(2);
    ranges.set_end(5.0);
    ranges.add_range();
    assert_eq!(
        ranges.blocker(true, &context).as_deref(),
        Some("A batch uses one range, remove the others")
    );

    let mut swap = form(Mode::ObjectSwap, Resolution::P720);
    swap.set_source(Some(video_named(14.2, 480, 480, "avc1", "clip.mp4")));
    swap.set_batch(true);
    swap.add_character_set(&images([1]));
    swap.add_character_set(&images([2]));
    assert_eq!(
        swap.blocker(true, &context).as_deref(),
        Some("Character 1: Object swap needs 640×640 px, allow the upscale")
    );
}

#[test]
fn a_mode_without_a_batch_turns_it_off_and_a_batch_makes_one_variant() {
    let mut form = ready(Resolution::P720);
    form.set_batch(true);
    let mut context = api_key();
    context.capabilities = Some(Capabilities::new(
        BTreeSet::from([Mode::MotionTransfer, Mode::Edit]),
        Resolution::ALL.iter().copied().collect(),
        CostUnit::Usd,
        None,
    ));
    if let Some(capabilities) = context.capabilities.as_mut() {
        capabilities.max_variants = 4;
    }
    assert_eq!(
        form.variants_unavailable_reason(&context).as_deref(),
        Some("A batch makes one video per style or character")
    );
    form.set_mode(Mode::Edit);
    assert!(!form.is_batch());
}

#[test]
fn twelve_styles_is_the_limit() {
    let mut form = restyle_batch(12);
    assert_eq!(
        form.batch_cap_notice().as_deref(),
        Some("A batch takes up to 12 styles. Remove one to add another.")
    );
    assert!(!form.toggle_batch_preset(preset_ids()[12]));
    assert_eq!(form.batch_count(), 12);
    assert!(form.toggle_batch_preset(preset_ids()[0]));
    assert_eq!(form.batch_cap_notice(), None);
    assert!(form.toggle_batch_preset(preset_ids()[12]));
    assert_eq!(form.batch_preset_ids().last(), Some(&preset_ids()[12]));
}

#[test]
fn twelve_characters_is_the_limit() {
    let mut form = ready(Resolution::P720);
    form.set_batch(true);
    for index in 1..12 {
        form.add_character_set(&images([index * 10]));
    }
    assert_eq!(form.batch_count(), 12);
    let notice = form.add_character_set(&images([200]));
    assert_eq!(notice.unwrap().title, "A batch takes up to 12 characters");
    assert_eq!(form.batch_count(), 12);
    assert!(!form.can_add_batch_variant());
}

#[test]
fn a_batch_total_adds_unrounded_job_prices_then_rounds() {
    // 13 s × $0.318 = $4.134 a job; seven of them are $28.938, not 7 × $4.13.
    let form = restyle_batch(7);
    let context = api_key();
    assert_eq!(form.price_text(true, &context).as_deref(), Some("$28.94"));
    let line = form.cost_line(&context).unwrap();
    assert_eq!(line.text, "91 s × $0.318 = $28.94");
    assert_eq!(line.detail, "List price · 7 jobs · failed runs are free");
    assert_eq!(form.price_text(false, &context).as_deref(), Some("$61.97"));
}

#[test]
fn over_the_cap_asks_once_for_the_whole_batch() {
    let form = restyle_batch(7);
    let mut context = api_key();
    context.cap = Some(SpendCap::usd(Decimal::from(20)));
    let SubmitDecision::Confirm(specs, confirmation) = form.submit_decision(true, &context) else {
        panic!("expected a confirmation");
    };
    assert_eq!(specs.len(), 7);
    assert_eq!(
        confirmation.message,
        "These 7 jobs add $28.94, bringing today to $28.94 against your $20.00 cap."
    );
    context.cap = Some(SpendCap::usd(Decimal::from(30)));
    assert_eq!(
        form.submit_decision(true, &context),
        SubmitDecision::Start(form.make_specs(true))
    );
    assert!(!form.cost_line(&context).unwrap().is_warning);
}

#[test]
fn a_run_that_fails_partway_says_how_many_were_queued() {
    let failure = crate::connection::UserMessage::new(
        "Can't reach Higgsfield",
        "Check your internet connection and try again.",
        None,
    );
    let message = super::service::partly_queued(Some(failure), 2, 3);
    assert_eq!(message.title, "2 of 3 jobs queued");
    assert_eq!(message.message, "Check your internet connection and try again.");
    let silent = super::service::partly_queued(None, 2, 3);
    assert_eq!(silent.title, "2 of 3 jobs queued");
    assert_eq!(silent.message, "The rest weren't queued.");
}

/// The string catalog's singular: one image left out of a batch character says "1 image wasn't added".
#[test]
fn one_image_left_out_of_a_character_reads_in_the_singular() {
    let mut form = form(Mode::MotionTransfer, Resolution::P720);
    let limit = form.limits().reference_images.upper();
    let notice = form.add_character_set(&images(1..=limit + 1)).unwrap();
    assert_eq!(notice.title, "1 image wasn't added");
}
