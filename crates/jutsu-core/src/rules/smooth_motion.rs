//! Enhance › Smooth Motion: which frames a result is made from, and where a clip cuts.

use crate::support::Seconds;
use crate::support::string_enum;

string_enum! {
    /// Enhance › Smooth Motion: frames made on this Mac between the clip's own (#13). 60 fps keeps the clip's length
    /// and sound; slow motion plays it 2× or 4× longer at the clip's own frame rate, without sound.
    pub enum SmoothMotion {
        /// 60 frames a second.
        Fps60 = "fps60",
        /// Twice as long.
        SlowMotion2x = "slowMotion2x",
        /// Four times as long.
        SlowMotion4x = "slowMotion4x",
    }
}

impl SmoothMotion {
    /// How many times longer the result plays than the clip.
    pub fn slowdown(self) -> i64 {
        match self {
            Self::Fps60 => 1,
            Self::SlowMotion2x => 2,
            Self::SlowMotion4x => 4,
        }
    }

    /// Slowed-down sound would no longer match the picture, so slow motion leaves it out.
    pub fn keeps_audio(self) -> bool {
        self.slowdown() == 1
    }

    /// The result's frames per second: 60, or for slow motion the clip's own (30 when the clip doesn't say).
    pub fn output_frame_rate(self, source: f64) -> i64 {
        if self.slowdown() == 1 {
            60
        } else if source >= 1.0 {
            source.round() as i64
        } else {
            30
        }
    }

    /// 60 fps adds nothing to a clip that already has 50 or more frames a second.
    pub fn is_offered(self, source_frame_rate: f64) -> bool {
        self.slowdown() > 1 || source_frame_rate < 50.0
    }

    /// The motions offered for a clip of `source_frame_rate`, in menu order.
    pub fn offered(source_frame_rate: f64) -> Vec<Self> {
        Self::ALL
            .iter()
            .copied()
            .filter(|motion| motion.is_offered(source_frame_rate))
            .collect()
    }
}

/// One frame of a Smooth Motion result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineFrame {
    /// The output frame.
    pub index: i64,
    /// 0 ≤ phase < 1, from the earlier source frame to the later one.
    pub phase: f64,
}

/// Which source frames each frame of a Smooth Motion result is made from. Output frame n is shown at n / rate and
/// samples the clip at n / (rate × slowdown), between the two source frames around that time at `phase`: 0 is the
/// earlier frame itself, which is copied rather than made.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameTimeline {
    frame_rate: i64,
    slowdown: i64,
    frame_count: i64,
    next: i64,
}

impl FrameTimeline {
    /// Timestamps are rationals read back as seconds; closer than a microsecond is the same time.
    pub const TOLERANCE: f64 = 1e-6;

    /// The timeline of a clip of `duration` at `frame_rate`, slowed down `slowdown` times.
    pub fn new(duration: Seconds, frame_rate: i64, slowdown: i64) -> Self {
        let frame_rate = frame_rate.max(1);
        let slowdown = slowdown.max(1);
        let frame_count = ((duration * (slowdown * frame_rate) as f64).round() as i64).max(0);
        Self {
            frame_rate,
            slowdown,
            frame_count,
            next: 0,
        }
    }

    /// The result's frames per second.
    pub fn frame_rate(&self) -> i64 {
        self.frame_rate
    }

    /// How many times longer the result plays.
    pub fn slowdown(&self) -> i64 {
        self.slowdown
    }

    /// Every frame of the result: the clip's length × slowdown × rate, rounded.
    pub fn frame_count(&self) -> i64 {
        self.frame_count
    }

    /// The next frame `frames` hands out.
    pub fn next(&self) -> i64 {
        self.next
    }

    /// The clip time output frame `index` samples.
    pub fn source_time(&self, index: i64) -> Seconds {
        index as f64 / (self.frame_rate * self.slowdown) as f64
    }

    /// The result's time of output frame `index`.
    pub fn output_time(&self, index: i64) -> Seconds {
        index as f64 / self.frame_rate as f64
    }

    /// The frames not handed out yet that sample the clip before `later`, each with its phase between `earlier` and
    /// `later`. With no later frame, every frame left: the last source frame holds to the end.
    pub fn frames(&mut self, earlier: Seconds, later: Option<Seconds>) -> Vec<TimelineFrame> {
        let mut frames = Vec::new();
        while self.next < self.frame_count {
            let time = self.source_time(self.next);
            let mut phase = 0.0;
            if let Some(later) = later {
                if time >= later - Self::TOLERANCE {
                    break;
                }
                let span = later - earlier;
                phase = if span > 0.0 {
                    ((time - earlier) / span).clamp(0.0, 1.0)
                } else {
                    0.0
                };
            }
            frames.push(TimelineFrame {
                index: self.next,
                phase: if phase < Self::TOLERANCE { 0.0 } else { phase },
            });
            self.next += 1;
        }
        frames
    }
}

/// Tells a hard cut from motion between two frames, so Smooth Motion repeats a frame there instead of blending two
/// shots into a ghost. Frames are compared as small grey thumbnails, by the mean absolute difference of their luma.
pub struct CutDetector;

impl CutDetector {
    /// The thumbnails' long side, whatever the frame's size; the other follows the frame's shape.
    pub const THUMBNAIL_SIDE: i64 = 360;
    /// On the 0…255 scale. Measured on 360-pixel frames of real clips (#13): hard cuts 37–45, the fastest motion
    /// without a cut 23.7.
    pub const THRESHOLD: f64 = 30.0;

    /// The mean absolute difference of two thumbnails of the same size; `None` when the sizes differ or they're empty.
    pub fn difference(first: &[u8], second: &[u8]) -> Option<f64> {
        if first.is_empty() || first.len() != second.len() {
            return None;
        }
        let total: u64 = first
            .iter()
            .zip(second)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        Some(total as f64 / first.len() as f64)
    }

    /// Whether the two thumbnails differ by more than `THRESHOLD`.
    pub fn is_cut(first: &[u8], second: &[u8]) -> bool {
        Self::difference(first, second).unwrap_or(0.0) > Self::THRESHOLD
    }
}
