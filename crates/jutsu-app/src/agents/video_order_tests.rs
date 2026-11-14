//! `AgentVideoOrderTests` (the order cases) and `AgentVariantsTests`, ported: an agent's video becomes the same job
//! the Create form makes, by the same rules, and is refused where the form would quietly fit it.

use std::collections::{BTreeMap, BTreeSet};

use jutsu_core::{
    AspectRatio, Capabilities, CostUnit, JobSpec, LocalImage, Mode, ModelConstraintTable, PixelSize,
    ProviderKind, Resolution, VideoModel,
};

use super::{AgentVariants, AgentVideoOrder, OrderSource, variants};
use crate::create::context::CreateContext;
use crate::create::form::{CreateForm, SourceVideo, time_range};

fn api_key() -> CreateContext {
    CreateContext {
        connection: Some(ProviderKind::ApiKey),
        ..CreateContext::default()
    }
}

fn base() -> CreateForm {
    CreateForm::new(Mode::Restyle, Resolution::P720, Resolution::P480)
}

fn video(duration: f64) -> SourceVideo {
    SourceVideo {
        file_url: crate::testing::file_url("/tmp/clip.mp4"),
        duration,
        pixel_size: PixelSize::new(1920, 1080),
        codec: "avc1".to_owned(),
        file_size: 38_000_000,
        remote_origin: None,
    }
}

fn image(index: usize) -> LocalImage {
    LocalImage::new(crate::testing::file_url(format!("/tmp/ref{index}.png")))
}

fn order_with(model: Option<&str>, start: Option<f64>, end: Option<f64>, images: usize) -> AgentVideoOrder {
    AgentVideoOrder {
        model: model.map(str::to_owned),
        prompt: "Image 1 dances".to_owned(),
        source: Some(OrderSource {
            video: video(14.2),
            start,
            end,
        }),
        images: (0..images).map(image).collect(),
        ..AgentVideoOrder::new("motion_transfer")
    }
}

fn order() -> AgentVideoOrder {
    order_with(None, Some(1.0), Some(6.0), 1)
}

fn refusal_in(order: &AgentVideoOrder, context: &CreateContext) -> Option<String> {
    order.spec(base(), context, None).err()
}

fn refusal(order: &AgentVideoOrder) -> Option<String> {
    refusal_in(order, &api_key())
}

fn says(refusal: Option<String>, text: &str) -> bool {
    refusal.is_some_and(|message| message.contains(text))
}

fn built_in() -> ModelConstraintTable {
    ModelConstraintTable::built_in()
}

#[test]
fn the_job_is_what_the_create_form_would_queue() {
    let context = api_key();
    let spec = order().spec(base(), &context, None).unwrap();

    let mut form = CreateForm::new(Mode::MotionTransfer, Resolution::P720, Resolution::P480);
    form.set_source(Some(video(14.2)));
    form.set_ranges_unclamped(vec![time_range(1.0, 6.0)]);
    form.add_references(vec![image(0)]);
    form.prompt = "Image 1 dances".to_owned();
    assert_eq!(form.specs_in(false, &context), vec![spec.clone()]);
    assert_eq!(
        spec.source.as_ref().map(|source| source.range),
        Some(time_range(1.0, 6.0))
    );
    assert_eq!(spec.model, VideoModel::GENJUTSU);
}

/// The agent's title names the job; the source's name without one.
#[test]
fn a_title_names_the_job_instead_of_the_source_file() {
    let untitled = order().spec(base(), &api_key(), None).unwrap();
    assert_eq!(untitled.title, None);
    let titled = AgentVideoOrder {
        title: Some("  Knight on the roof\n".to_owned()),
        ..order()
    };
    let spec = titled.spec(base(), &api_key(), None).unwrap();
    assert_eq!(spec.title.as_deref(), Some("Knight on the roof"));
    assert_eq!(spec.name(), "Knight on the roof");
}

/// The form keeps a range inside the model's window by moving it; an agent's range is refused instead.
#[test]
fn a_range_outside_the_models_window_is_refused_not_moved() {
    let limits = VideoModel::GENJUTSU.limits(Mode::MotionTransfer, true, &built_in());
    let most = limits.duration.upper();
    let too_long = AgentVideoOrder {
        source: Some(OrderSource {
            video: video(most + 20.0),
            start: Some(0.0),
            end: Some(most + 5.0),
        }),
        ..order()
    };
    assert!(says(refusal(&too_long), "too long"));
    let too_short = order_with(None, Some(0.0), Some((limits.duration.lower() - 1.0).max(0.5)), 1);
    assert!(says(refusal(&too_short), "too short"));
    assert!(says(
        refusal(&order_with(None, Some(0.0), Some(20.0), 1)),
        "past the clip"
    ));
}

/// The form drops references over the limit; an agent's are refused.
#[test]
fn too_many_images_are_refused_not_dropped() {
    let most = VideoModel::GENJUTSU
        .limits(Mode::MotionTransfer, true, &built_in())
        .reference_images
        .upper();
    let over = order_with(None, Some(1.0), Some(6.0), most + 1);
    assert!(says(refusal(&over), &format!("at most {most}")));
    assert!(says(
        refusal(&order_with(None, Some(1.0), Some(6.0), 0)),
        "at least"
    ));
}

#[test]
fn models_that_cant_do_it_are_refused() {
    assert!(says(
        refusal(&order_with(Some("nope"), Some(1.0), Some(6.0), 1)),
        "no model nope"
    ));
    let other = VideoModel::all()
        .into_iter()
        .find(|model| !model.serves(Mode::MotionTransfer))
        .unwrap();
    let wrong = order_with(Some(other.raw_value()), Some(1.0), Some(6.0), 1);
    assert!(says(refusal(&wrong), "doesn't do motion_transfer"));
    let dance = AgentVideoOrder {
        mode: "dance".to_owned(),
        ..order()
    };
    assert!(says(refusal(&dance), "mode is one of"));
}

#[test]
fn a_draft_or_resolution_the_model_lacks_is_refused() {
    if let Some(no_draft) = VideoModel::all()
        .into_iter()
        .find(|model| !model.offers_draft() && model.serves(Mode::MotionTransfer))
    {
        let draft = AgentVideoOrder {
            is_draft: true,
            ..order_with(Some(no_draft.raw_value()), Some(1.0), Some(6.0), 1)
        };
        assert!(says(refusal(&draft), "no drafts"));
    }
    let offered = VideoModel::GENJUTSU.resolutions();
    let missing = Resolution::ALL
        .iter()
        .find(|resolution| !offered.contains(resolution))
        .unwrap();
    let unrendered = AgentVideoOrder {
        resolution: Some(missing.raw_value().to_owned()),
        ..order()
    };
    assert!(says(refusal(&unrendered), "renders at"));
    let unknown = AgentVideoOrder {
        resolution: Some("8K".to_owned()),
        ..order()
    };
    assert!(says(refusal(&unknown), "renders at"));
}

#[test]
fn a_length_outside_the_window_is_refused() {
    let model = VideoModel::all()
        .into_iter()
        .find(VideoModel::makes_clips_without_source)
        .unwrap();
    let window = model.limits(Mode::MotionTransfer, false, &built_in()).duration;
    let mut no_source = AgentVideoOrder {
        source: None,
        length: Some(window.upper() + 1.0),
        ..order_with(Some(model.raw_value()), Some(1.0), Some(6.0), 1)
    };
    assert!(says(refusal(&no_source), "length is"));
    no_source.length = Some(window.lower());
    assert_eq!(refusal(&no_source), None);
    // With a source the range sets the length.
    let with_source = AgentVideoOrder {
        length: Some(5.0),
        ..order()
    };
    assert!(says(refusal(&with_source), "only for a clip without a source"));
}

#[test]
fn other_fields_are_checked_by_name() {
    let loud = AgentVideoOrder {
        audio: Some("loud".to_owned()),
        ..order()
    };
    assert!(says(refusal(&loud), "audio is one of"));
    let framed = AgentVideoOrder {
        aspect_ratio: Some("16:9".to_owned()),
        ..order()
    };
    assert!(says(refusal(&framed), "without a source"));
    let odd = AgentVideoOrder {
        aspect_ratio: Some("2:1".to_owned()),
        ..order()
    };
    assert!(says(refusal(&odd), "aspect_ratio is one of"));
    // The connection's capabilities aren't known here, so it makes one video per job.
    let three = AgentVideoOrder {
        variants: Some(3),
        ..order()
    };
    assert!(refusal(&three).is_some());
    assert!(says(
        refusal_in(&order(), &CreateContext::default()),
        "isn't connected"
    ));
}

#[test]
fn a_quote_stands_in_for_the_files() {
    let quote = AgentVideoOrder {
        source: Some(AgentVideoOrder::quote_source(None, 8.0)),
        images: AgentVideoOrder::quote_images(2),
        ..AgentVideoOrder::new("motion_transfer")
    };
    let spec = quote.spec(base(), &api_key(), None).unwrap();
    assert!((spec.duration() - 8.0).abs() < f64::EPSILON);
    assert_eq!(spec.references.len(), 2);
    assert!(
        spec.source
            .as_ref()
            .unwrap()
            .file_url
            .path()
            .starts_with("/quote/")
    );
    // A result from a source keeps its orientation; one without says the frame it asks for.
    assert_eq!(AgentVideoOrder::frame(&spec).as_deref(), Some("source"));
    let mut prompted = JobSpec::new(
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        None,
        Resolution::P720,
    );
    prompted.prompt = "a fox".to_owned();
    prompted.aspect_ratio = Some(AspectRatio::Portrait);
    assert_eq!(AgentVideoOrder::frame(&prompted).as_deref(), Some("9:16"));
}

fn capabilities(max_variants: i64, unlimited: BTreeMap<VideoModel, BTreeSet<Mode>>) -> Capabilities {
    let mut capabilities = Capabilities::new(
        Mode::ALL.iter().copied().collect(),
        BTreeSet::from([Resolution::P720]),
        CostUnit::Credits,
        None,
    );
    capabilities.models = VideoModel::all().into_iter().collect();
    capabilities.unlimited = unlimited;
    capabilities.max_variants = max_variants;
    capabilities
}

/// What capabilities and quote say about variants before `create_video` refuses them.
#[test]
fn each_reason_for_one_video_is_said() {
    let account = capabilities(
        4,
        BTreeMap::from([(VideoModel::SEEDANCE25, BTreeSet::from([Mode::MotionTransfer]))]),
    );
    let open = variants(
        &VideoModel::SEEDANCE25,
        Mode::MotionTransfer,
        Some(&account),
        false,
    );
    assert_eq!(
        open,
        AgentVariants {
            maximum: 4,
            reason: None
        }
    );
    let unlimited = variants(
        &VideoModel::SEEDANCE25,
        Mode::MotionTransfer,
        Some(&account),
        true,
    );
    assert_eq!(unlimited.maximum, 1);
    assert!(says(unlimited.reason, "Use unlimited generations"));
    // The allowance covers another mode: variants stay.
    assert_eq!(
        variants(&VideoModel::SEEDANCE25, Mode::ObjectSwap, Some(&account), true).maximum,
        4
    );
    let style = variants(&VideoModel::GENJUTSU, Mode::Restyle, Some(&account), false);
    assert!(style.maximum == 1 && says(style.reason, "style"));
    let one = capabilities(1, BTreeMap::new());
    let api_key = variants(&VideoModel::SEEDANCE25, Mode::MotionTransfer, Some(&one), false);
    assert!(api_key.maximum == 1 && says(api_key.reason, "one video per job"));
    let unknown = variants(&VideoModel::SEEDANCE25, Mode::MotionTransfer, None, false);
    assert!(unknown.maximum == 1 && unknown.reason.is_some());
}
