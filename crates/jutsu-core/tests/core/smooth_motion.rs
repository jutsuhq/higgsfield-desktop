//! `SmoothMotionTests.swift`.

use jutsu_core::*;

#[test]
fn slow_motion_keeps_the_clips_rate_and_drops_the_sound() {
    assert_eq!(SmoothMotion::Fps60.output_frame_rate(24.0), 60);
    assert_eq!(SmoothMotion::SlowMotion2x.output_frame_rate(29.97), 30);
    assert_eq!(SmoothMotion::SlowMotion4x.output_frame_rate(0.0), 30);
    assert!(SmoothMotion::Fps60.keeps_audio());
    assert!(!SmoothMotion::SlowMotion2x.keeps_audio());
    assert!(!SmoothMotion::SlowMotion4x.keeps_audio());
}

#[test]
fn sixty_frames_a_second_is_offered_only_below_fifty() {
    assert_eq!(
        SmoothMotion::offered(30.0),
        [
            SmoothMotion::Fps60,
            SmoothMotion::SlowMotion2x,
            SmoothMotion::SlowMotion4x
        ]
    );
    assert_eq!(
        SmoothMotion::offered(60.0),
        [SmoothMotion::SlowMotion2x, SmoothMotion::SlowMotion4x]
    );
}

/// Feeds a clip's frame times through the timeline the way the renderer does, pair by pair.
fn plan(timeline: &mut FrameTimeline, source_times: &[f64]) -> Vec<Vec<TimelineFrame>> {
    (0..source_times.len())
        .map(|index| timeline.frames(source_times[index], source_times.get(index + 1).copied()))
        .collect()
}

fn indices(frames: &[TimelineFrame]) -> Vec<i64> {
    frames.iter().map(|frame| frame.index).collect()
}

#[test]
fn sixty_from_thirty_makes_one_frame_between_each_pair() {
    let mut timeline = FrameTimeline::new(1.0, 60, 1);
    let times: Vec<f64> = (0..30).map(|i| f64::from(i) / 30.0).collect();
    let pairs = plan(&mut timeline, &times);

    assert_eq!(timeline.frame_count(), 60);
    assert_eq!(indices(&pairs.concat()), (0..60).collect::<Vec<_>>());
    assert_eq!(
        pairs[0].iter().map(|frame| frame.phase).collect::<Vec<_>>(),
        [0.0, 0.5]
    );
    assert!((timeline.output_time(59) - 59.0 / 60.0).abs() < 1e-9);
}

#[test]
fn four_times_slower_samples_quarter_phases_and_lasts_four_times_as_long() {
    let mut timeline = FrameTimeline::new(1.0, 24, 4);
    let times: Vec<f64> = (0..24).map(|i| f64::from(i) / 24.0).collect();
    let pairs = plan(&mut timeline, &times);

    assert_eq!(timeline.frame_count(), 96);
    assert!((timeline.output_time(timeline.frame_count()) - 4.0).abs() < 1e-9);
    for pair in &pairs[..pairs.len() - 1] {
        let phases: Vec<f64> = pair
            .iter()
            .map(|frame| (frame.phase * 100.0).round() / 100.0)
            .collect();
        assert_eq!(phases, [0.0, 0.25, 0.5, 0.75]);
    }
}

#[test]
fn the_last_frame_holds_to_the_end_and_a_first_frame_late_holds_back() {
    let mut timeline = FrameTimeline::new(1.0, 10, 1);
    // The clip's frames start late and stop early.
    let pairs = plan(&mut timeline, &[0.25, 0.5]);

    assert_eq!(indices(&pairs[0]), [0, 1, 2, 3, 4]);
    assert!(pairs[0][..3].iter().all(|frame| frame.phase == 0.0));
    assert_eq!(indices(&pairs[1]), [5, 6, 7, 8, 9]);
    assert!(pairs[1].iter().all(|frame| frame.phase == 0.0));
    assert!(timeline.frames(1.0, None).is_empty());
}

#[test]
fn cuts_stand_out_from_motion() {
    let gradient: Vec<u8> = (0..64 * 36).map(|i| u8::try_from(i % 64 * 3).unwrap()).collect();
    // The same picture moved two pixels to the side: motion, not a cut.
    let moved: Vec<u8> = (0..gradient.len())
        .map(|i| gradient[(i + 2) % gradient.len()])
        .collect();
    let other_shot: Vec<u8> = gradient.iter().map(|value| 255 - value).collect();

    assert_eq!(CutDetector::difference(&gradient, &gradient), Some(0.0));
    assert!(!CutDetector::is_cut(&gradient, &moved));
    assert!(CutDetector::is_cut(&gradient, &other_shot));
    assert_eq!(CutDetector::difference(&gradient, &[]), None);
    assert!(!CutDetector::is_cut(&gradient, &[1, 2]));
}

#[test]
fn h264_only_where_it_holds_the_frame() {
    assert_eq!(
        EnhanceCodec::of(PixelSize::new(1920, 1080), true),
        EnhanceCodec::H264
    );
    assert_eq!(
        EnhanceCodec::of(PixelSize::new(3416, 1920), true),
        EnhanceCodec::H264
    );
    assert_eq!(
        EnhanceCodec::of(PixelSize::new(2880, 5120), true),
        EnhanceCodec::Hevc
    );
    assert_eq!(
        EnhanceCodec::of(PixelSize::new(1920, 1080), false),
        EnhanceCodec::Hevc
    );
}

#[test]
fn larger_frames_take_longer() {
    let small = OnDeviceEstimate::smooth_motion(10.0, 24.0, PixelSize::new(854, 480));
    let large = OnDeviceEstimate::smooth_motion(10.0, 24.0, PixelSize::new(1920, 1080));
    assert!(small > 10.0);
    assert!(large > small);
    // Per frame with the pixel count, not one step above 480p.
    let middle = OnDeviceEstimate::smooth_motion(10.0, 24.0, PixelSize::new(1280, 720));
    assert!(small < middle && middle < large);
    assert!(((small - 10.0) / 240.0 - 0.05).abs() < 0.001);
}
