//! `OnDeviceUpscaleTests.swift`.

use jutsu_core::*;

fn size(width: i64, height: i64) -> PixelSize {
    PixelSize::new(width, height)
}

fn target(name: &str) -> OnDeviceUpscaleTarget {
    OnDeviceUpscaleTarget::from_name(name).unwrap()
}

#[test]
fn videos_up_to_720p_are_offered() {
    for (size, is_offered) in [
        (size(854, 480), true),
        (size(480, 854), true),
        (size(1280, 720), true),
        (size(720, 1280), true),
        (size(1920, 1080), false),
        (size(1080, 1920), false),
        (size(1280, 800), false),
        (size(0, 0), false),
    ] {
        assert_eq!(OnDeviceUpscale::offers_video(size), is_offered, "{size:?}");
    }
}

#[test]
fn images_up_to_1080p_are_offered() {
    for (size, is_offered) in [
        (size(1920, 1080), true),
        (size(1080, 1920), true),
        (size(1600, 1000), true),
        (size(1920, 1200), false),
        (size(2048, 1024), false),
        (size(0, 0), false),
    ] {
        assert_eq!(OnDeviceUpscale::offers_image(size), is_offered, "{size:?}");
    }
}

#[test]
fn a_frame_that_fits_goes_in_as_it_is() {
    let plan = OnDeviceUpscale::plan(size(480, 854), OnDeviceUpscale::SCALER_LIMIT).unwrap();
    assert!(!plan.rotates);
    assert_eq!(plan.scaler_input, size(480, 854));
    assert_eq!(plan.output, size(1920, 3416));
}

#[test]
fn a_portrait_frame_too_tall_is_turned() {
    let plan = OnDeviceUpscale::plan(size(720, 1280), OnDeviceUpscale::SCALER_LIMIT).unwrap();
    assert!(plan.rotates);
    assert_eq!(plan.scaler_input, size(1280, 720));
    assert_eq!(plan.output, size(2880, 5120));
    assert_eq!(EnhanceCodec::of(plan.output, true), EnhanceCodec::Hevc);
}

#[test]
fn a_frame_neither_way_fits_has_no_plan() {
    assert_eq!(
        OnDeviceUpscale::plan(size(2048, 1080), OnDeviceUpscale::SCALER_LIMIT),
        None
    );
    assert_eq!(
        OnDeviceUpscale::plan(size(1088, 1920), OnDeviceUpscale::SCALER_LIMIT),
        None
    );
}

#[test]
fn the_estimate_follows_the_benchmark() {
    // One second at 30 fps: 30 frames, plus the engine's start.
    let per_frame_small = (OnDeviceEstimate::upscale_video(1.0, 30.0, size(480, 854)) - 10.0) / 30.0;
    let per_frame_large = (OnDeviceEstimate::upscale_video(1.0, 30.0, size(720, 1280)) - 10.0) / 30.0;
    assert!((per_frame_small - 1.0).abs() < 0.01);
    assert!((per_frame_large - 1.95).abs() < 0.01);
}

#[test]
fn only_super_resolution_downloads_a_model() {
    assert!(!OnDeviceEngine::FrameRateConversion.downloads_model());
    assert!(OnDeviceEngine::VideoSuperResolution.downloads_model());
    assert_eq!(
        OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full).engine(),
        OnDeviceEngine::VideoSuperResolution
    );
    assert_eq!(
        OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full).file_suffix(),
        "4x"
    );
    assert!(OnDeviceEngine::ImageSuperResolution.downloads_model());
    assert_eq!(
        OnDeviceAction::UpscaleImage(OnDeviceUpscaleTarget::Full).engine(),
        OnDeviceEngine::ImageSuperResolution
    );
}

/// A delivery size keeps the source's shape with even sides, the short side exactly the one named; a factor
/// scales both sides. 4× is the most, and a target must be larger than the source.
#[test]
fn a_target_keeps_the_shape() {
    for (name, source, output) in [
        ("1080p", size(720, 1280), size(1080, 1920)),
        ("1080p", size(576, 1024), size(1080, 1920)),
        ("1080p", size(554, 740), size(1080, 1442)),
        ("2k", size(1280, 720), size(2560, 1440)),
        ("4k", size(720, 1280), size(2160, 3840)),
        ("2x", size(480, 854), size(960, 1708)),
        ("2.5x", size(481, 270), size(1202, 676)),
        ("4x", size(480, 854), size(1920, 3416)),
    ] {
        assert_eq!(target(name).output(source), Some(output), "{name} {source:?}");
    }
}

#[test]
fn a_target_must_be_larger_and_at_most_four_times() {
    assert_eq!(
        OnDeviceUpscaleTarget::from_name("4x"),
        Some(OnDeviceUpscaleTarget::Full)
    );
    assert_eq!(
        OnDeviceUpscaleTarget::from_name("4K"),
        Some(OnDeviceUpscaleTarget::ShortSide(2160))
    );
    for name in ["1x", "0.5x", "4.5x", "8k", "x", "1080", "nanx", "infx"] {
        assert_eq!(OnDeviceUpscaleTarget::from_name(name), None, "{name}");
    }
    // 4k from 480p needs 4.5×; 1080p from a 1080 wide portrait clip adds nothing.
    assert_eq!(
        OnDeviceUpscaleTarget::ShortSide(2160).output(size(480, 854)),
        None
    );
    assert_eq!(
        OnDeviceUpscaleTarget::ShortSide(1080).output(size(1080, 1350)),
        None
    );
    assert_eq!(
        OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::ShortSide(1080)).file_suffix(),
        "1080p"
    );
    assert_eq!(
        OnDeviceAction::UpscaleImage(OnDeviceUpscaleTarget::ShortSide(1440)).file_suffix(),
        "2k"
    );
    assert_eq!(OnDeviceAction::UpscaleImage(target("2.5x")).file_suffix(), "2.5x");
    assert_eq!(OnDeviceAction::UpscaleVideo(target("2x")).file_suffix(), "2x");
}
