//! `EditingExportTests.swift`.

use std::collections::HashSet;

use jutsu_core::*;
use time::macros::datetime;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::common::*;

mod export_preset {
    use jutsu_core::*;

    #[test]
    fn nle_presets_are_pro_res_in_quick_time_with_pcm_and_editing_h264_is_mp4_with_aac() {
        for (preset, video, audio, extension) in [
            (
                ExportPreset::Premiere,
                VideoCodec::ProRes422,
                AudioCodec::LinearPcm,
                "mov",
            ),
            (
                ExportPreset::DavinciResolve,
                VideoCodec::ProRes422,
                AudioCodec::LinearPcm,
                "mov",
            ),
            (
                ExportPreset::FinalCutPro,
                VideoCodec::ProRes422,
                AudioCodec::LinearPcm,
                "mov",
            ),
            (
                ExportPreset::EditingH264,
                VideoCodec::H264,
                AudioCodec::Aac,
                "mp4",
            ),
        ] {
            assert_eq!(preset.video_codec(), video);
            assert_eq!(preset.audio_codec(), audio);
            assert_eq!(preset.file_extension(), extension);
        }
    }

    #[test]
    fn frame_duration_matches_the_nominal_rate() {
        for (rate, value, timescale) in [
            (23.976, 1001, 24000),
            (23.98, 1001, 24000),
            (29.97, 1001, 30000),
            (59.94, 1001, 60000),
            (24.0, 1, 24),
            (25.0, 1, 25),
            (30.02, 1, 30),
            (60.0, 1, 60),
            // An unusable rate falls back to 30.
            (0.0, 1, 30),
            (-5.0, 1, 30),
            (f64::NAN, 1, 30),
            (f64::INFINITY, 1, 30),
            (1000.0, 1, 30),
        ] {
            assert_eq!(
                FrameDuration::constant(rate),
                FrameDuration { value, timescale },
                "{rate}"
            );
        }
    }
}

struct Spec {
    mode: Mode,
    source: &'static str,
    range: (f64, f64),
    references: Vec<&'static str>,
    preset_id: Option<Uuid>,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            mode: Mode::MotionTransfer,
            source: "/clips/rooftop.mov",
            range: (4.0, 10.0),
            references: vec!["/refs/hero.png"],
            preset_id: None,
        }
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "taken whole, like the Swift helper's labelled arguments"
)]
fn spec(args: Spec) -> JobSpec {
    let clip = SourceClip::new(
        file_url(args.source),
        600.0,
        PixelSize::new(1920, 1080),
        ClosedRange::new(args.range.0, args.range.1),
    );
    let mut spec = JobSpec::new(args.mode, VideoModel::genjutsu(), Some(clip), Resolution::P720);
    spec.references = args
        .references
        .iter()
        .map(|path| LocalImage::new(file_url(path)))
        .collect();
    spec.preset_id = args.preset_id;
    spec
}

#[test]
fn motion_transfer_names_the_reference() {
    let names = ExportNaming::names(&spec(Spec::default()), None, ExportPreset::Premiere, |_| false);
    assert_eq!(names.video, "rooftop_00m04.0s-00m10.0s_motion_hero_v1.mov");
    assert_eq!(names.sidecar, "rooftop_00m04.0s-00m10.0s_motion_hero_v1.json");
}

#[test]
fn extra_references_are_counted() {
    let spec = spec(Spec {
        mode: Mode::ObjectSwap,
        references: vec!["/r/hero.png", "/r/b.jpg", "/r/c.jpg"],
        ..Spec::default()
    });
    assert_eq!(
        ExportNaming::base_name(&spec, None),
        "rooftop_00m04.0s-00m10.0s_swap_hero+2"
    );
}

#[test]
fn restyle_names_the_preset() {
    let spec = spec(Spec {
        mode: Mode::Restyle,
        references: vec!["/r/ignored.png"],
        preset_id: Some(Uuid::new_v4()),
        ..Spec::default()
    });
    assert_eq!(
        ExportNaming::base_name(&spec, Some("Film Noir")),
        "rooftop_00m04.0s-00m10.0s_restyle_Film Noir"
    );
    assert_eq!(
        ExportNaming::base_name(&spec, None),
        "rooftop_00m04.0s-00m10.0s_restyle_preset"
    );
}

#[test]
fn missing_style_leaves_no_empty_field() {
    let spec = spec(Spec {
        references: Vec::new(),
        ..Spec::default()
    });
    assert_eq!(
        ExportNaming::base_name(&spec, None),
        "rooftop_00m04.0s-00m10.0s_motion"
    );
}

#[test]
fn range_is_zero_padded_tenths() {
    for (lower, upper, expected) in [
        (0.0, 4.04, "00m00.0s-00m04.0s"),
        (59.96, 61.25, "01m00.0s-01m01.3s"),
        (754.5, 780.0, "12m34.5s-13m00.0s"),
    ] {
        assert_eq!(ExportNaming::range_text(ClosedRange::new(lower, upper)), expected);
    }
}

#[test]
fn unsafe_characters_are_replaced() {
    let spec = spec(Spec {
        source: "/clips/.a:b*c?.mov",
        references: vec!["/r/x<y>|z\".png"],
        ..Spec::default()
    });
    assert_eq!(
        ExportNaming::base_name(&spec, None),
        "a-b-c-_00m04.0s-00m10.0s_motion_x-y--z-"
    );
}

#[test]
fn empty_source_name_becomes_clip() {
    let spec = spec(Spec {
        source: "/clips/ .mov",
        references: Vec::new(),
        ..Spec::default()
    });
    assert!(ExportNaming::base_name(&spec, None).starts_with("clip_00m04.0s"));
}

#[test]
fn long_fields_are_cut_by_bytes() {
    let source: &'static str = Box::leak(format!("/c/{}.mov", "é".repeat(200)).into_boxed_str());
    let spec = spec(Spec {
        mode: Mode::Restyle,
        source,
        preset_id: Some(Uuid::new_v4()),
        ..Spec::default()
    });
    let name = ExportNaming::names(&spec, Some(&"x".repeat(100)), ExportPreset::FinalCutPro, |_| {
        false
    })
    .video;
    assert!(name.len() <= 255);
    assert!(name.ends_with(&format!("_restyle_{}_v1.mov", "x".repeat(40))));
}

#[test]
fn version_bumps_past_any_taken_take() {
    let base = "rooftop_00m04.0s-00m10.0s_motion_hero";
    let taken: HashSet<String> = [
        format!("{base}_v1.mov"),
        format!("{base}_v2.mp4"),
        format!("{base}_v3.json"),
    ]
    .into();
    let names = ExportNaming::names(&spec(Spec::default()), None, ExportPreset::EditingH264, |name| {
        taken.contains(name)
    });
    assert_eq!(names.video, format!("{base}_v4.mp4"));
    assert_eq!(names.sidecar, format!("{base}_v4.json"));
}

#[test]
fn a_taken_subtitles_name_bumps_the_version_for_all_three() {
    let base = "rooftop_00m04.0s-00m10.0s_motion_hero";
    let taken = format!("{base}_v1.srt");
    let names = ExportNaming::names(&spec(Spec::default()), None, ExportPreset::EditingH264, |name| {
        name == taken
    });
    assert_eq!(names.video, format!("{base}_v2.mp4"));
    assert_eq!(names.sidecar, format!("{base}_v2.json"));
    assert_eq!(names.subtitles, format!("{base}_v2.srt"));
}

fn sidecar_job() -> Job {
    let id = Uuid::parse_str("1A2B3C4D-0000-0000-0000-000000000000").unwrap();
    let created_at = datetime!(2026-03-07 23:30:05 UTC);
    let spec = spec_with(SpecArgs {
        mode: Mode::Restyle,
        references: 2,
        preset_id: Some(Uuid::new_v4()),
        prompt: "rainy night".into(),
        ..SpecArgs::default()
    });
    let mut job = Job::new(
        id,
        spec,
        ProviderKind::ApiKey,
        usd("1.91", true),
        Uuid::new_v4(),
        created_at,
    );
    job.server_price = Some(usd("1.908", false));
    let mut remote = RemoteJob::new("req-42");
    remote.correlation_id = Some("corr".into());
    remote.status_url = Some(url::Url::parse("https://api.example/s?sig=abc").unwrap());
    remote.cancel_url = Some(url::Url::parse("https://api.example/c?sig=abc").unwrap());
    job.remote = Some(remote);
    job.finished_at = Some(created_at + Duration::seconds(90));
    job
}

#[test]
fn carries_the_job_inputs() {
    let job = sidecar_job();
    let sidecar = ExportSidecar::new(
        &job,
        Some("Film Noir"),
        ExportPreset::DavinciResolve,
        OffsetDateTime::now_utc(),
    );
    assert_eq!(
        sidecar.source,
        Some(SidecarSource {
            file_name: "source.mov".into(),
            range_start: 0.0,
            range_end: 10.0
        })
    );
    assert_eq!(sidecar.references, ["ref0.jpg", "ref1.jpg"]);
    assert_eq!(
        sidecar.style.and_then(|style| style.name).as_deref(),
        Some("Film Noir")
    );
    assert_eq!(sidecar.prompt, "rainy night");
    assert_eq!(sidecar.request_id.as_deref(), Some("req-42"));
    assert_eq!(
        sidecar.cost,
        SidecarPrice {
            amount: "1.908".into(),
            unit: "usd".into(),
            is_estimate: false
        }
    );
    assert_eq!(sidecar.export_preset, ExportPreset::DavinciResolve);
}

#[test]
fn round_trips_and_leaks_no_paths_or_urls() {
    let job = sidecar_job();
    let sidecar = ExportSidecar::new(&job, None, ExportPreset::Premiere, job.created_at);
    let data = sidecar.encoded().unwrap();
    assert_eq!(ExportSidecar::decode(data.as_bytes()).unwrap(), sidecar);
    for leak in [
        "/tmp",
        "bookmark",
        "sig=",
        "https",
        "corr",
        "idempotency",
        "apiKey",
    ] {
        assert!(!data.contains(leak), "sidecar contains {leak}");
    }
    assert!(data.contains("\"exportedAt\" : \"2026-03-07T23:30:05Z\""));
}
