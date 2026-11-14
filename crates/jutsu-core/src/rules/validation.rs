//! API input rules shared by all providers.

use crate::models::{JobSpec, Mode, ModelConstraintTable, PixelSize, VideoControl};
use crate::rules::{Pricing, ValidationIssue};
use crate::support::Seconds;
use crate::support::range::ClosedRange;
use crate::support::text::trim;

/// API input rules shared by all providers.
pub struct Validation;

impl Validation {
    /// Seconds of source one job takes.
    pub const DURATION_RANGE: ClosedRange<Seconds> = ClosedRange::new(4.0, 30.0);
    /// The fewest pixels an Object Swap frame has.
    pub const OBJECT_SWAP_MINIMUM_PIXELS: i64 = 409_600;
    /// The longest prompt, in Unicode scalars.
    pub const MAXIMUM_PROMPT_LENGTH: usize = 10_000;

    /// Empty when the spec can be submitted; limits by `constraints` where they say (`ModelConstraintTable::built_in`
    /// in Swift's default).
    pub fn check(spec: &JobSpec, constraints: &ModelConstraintTable) -> Vec<ValidationIssue> {
        let limits = spec.limits(constraints);
        let mut issues = Self::source_issues(spec);
        if spec.model.declares(VideoControl::Speech) {
            // The speech sets the length.
            issues.extend(Self::speech_issues(spec));
        } else if !issues.contains(&ValidationIssue::SelectionOutsideSource)
            // The selection's length means nothing until the selection is inside the video.
            && let Some(issue) = Self::duration_issue(spec, limits.duration)
        {
            issues.push(issue);
        }

        if spec.references.len() < limits.reference_images.lower() {
            issues.push(ValidationIssue::TooFewReferences {
                minimum: limits.reference_images.lower(),
            });
        } else if spec.references.len() > limits.reference_images.upper() {
            issues.push(ValidationIssue::TooManyReferences {
                maximum: limits.reference_images.upper(),
            });
        }

        if limits.requires_preset && spec.preset_id.is_none() {
            issues.push(ValidationIssue::PresetRequired);
        }

        if let Some(issue) = Self::frame_issue(spec) {
            issues.push(issue);
        }

        let prompt_length = spec.prompt.chars().count();
        if prompt_length > Self::MAXIMUM_PROMPT_LENGTH {
            issues.push(ValidationIssue::PromptTooLong {
                length: prompt_length,
            });
        }
        issues
    }

    /// A selection outside the video; without a video, a model that needs one, or nothing to go on; Edit, Extend
    /// and a model that needs a prompt (`requires_prompt`) without one, also without a video unless the clip starts
    /// from frames (Kling 4.0's `image-to-video` is the one of its endpoints that takes no prompt).
    fn source_issues(spec: &JobSpec) -> Vec<ValidationIssue> {
        let tolerance = Pricing::DURATION_TOLERANCE;
        let has_prompt = !trim(&spec.prompt).is_empty();
        let Some(source) = &spec.source else {
            if !spec.model.makes_clips_without_source() || spec.mode != Mode::MotionTransfer {
                return vec![ValidationIssue::SourceRequired];
            }
            if has_prompt {
                return Vec::new();
            }
            if spec.model.descriptor().requires_prompt {
                return if spec.uses_frames() {
                    Vec::new()
                } else {
                    vec![ValidationIssue::PromptRequired]
                };
            }
            return if spec.references.is_empty() {
                vec![ValidationIssue::PromptRequired]
            } else {
                Vec::new()
            };
        };
        let range = source.range;
        let is_inside = source.duration.is_finite()
            && range.lower().is_finite()
            && range.upper().is_finite()
            && range.lower() >= -tolerance
            && range.upper() <= source.duration + tolerance;
        let mut issues = if is_inside {
            Vec::new()
        } else {
            vec![ValidationIssue::SelectionOutsideSource]
        };
        if (spec.mode.is_video_operation() || spec.model.descriptor().requires_prompt) && !has_prompt {
            issues.push(ValidationIssue::PromptRequired);
        }
        issues
    }

    /// A model that voices a speech: a source it doesn't take, and the speech missing or outside its window.
    fn speech_issues(spec: &JobSpec) -> Vec<ValidationIssue> {
        let mut issues = if spec.source.is_none() {
            Vec::new()
        } else {
            vec![ValidationIssue::SourceNotTaken]
        };
        let Some(speech) = &spec.speech else {
            issues.push(ValidationIssue::SpeechRequired);
            return issues;
        };
        let tolerance = Pricing::DURATION_TOLERANCE;
        if let Some(window) = spec.model.descriptor().speech_seconds {
            if speech.duration < window.lower() - tolerance {
                issues.push(ValidationIssue::SpeechTooShort(speech.duration));
            }
            if speech.duration > window.upper() + tolerance {
                issues.push(ValidationIssue::SpeechTooLong(speech.duration));
            }
        }
        issues
    }

    /// The source selection, or the length of a clip without one, outside the model's window; for Extend, the
    /// selection or the added length.
    fn duration_issue(spec: &JobSpec, limits: ClosedRange<Seconds>) -> Option<ValidationIssue> {
        let tolerance = Pricing::DURATION_TOLERANCE;
        let mut durations = vec![
            spec.source
                .as_ref()
                .map_or_else(|| spec.duration(), crate::models::SourceClip::selected_duration),
        ];
        if spec.mode == Mode::Extend {
            durations.push(spec.duration());
        }
        for duration in durations {
            if duration < limits.lower() - tolerance {
                return Some(ValidationIssue::DurationTooShort(duration));
            }
            if duration > limits.upper() + tolerance {
                return Some(ValidationIssue::DurationTooLong(duration));
            }
        }
        None
    }

    /// The source's frame; a clip made without a source has none to check.
    fn frame_issue(spec: &JobSpec) -> Option<ValidationIssue> {
        let source = spec.source.as_ref()?;
        let size = source.pixel_size;
        if !size.is_valid() {
            return Some(ValidationIssue::InvalidFrameSize(size));
        }
        if spec.mode == Mode::ObjectSwap
            && size.pixel_count() < Self::OBJECT_SWAP_MINIMUM_PIXELS
            && !source.allows_upscale
        {
            return Some(ValidationIssue::FrameTooSmall(size));
        }
        None
    }

    /// Smallest size with the same aspect ratio, even sides (H.264 needs them) and at least
    /// `OBJECT_SWAP_MINIMUM_PIXELS`. Sizes already large enough are returned unchanged.
    pub fn upscaled_size(width: i64, height: i64) -> PixelSize {
        let size = PixelSize::new(width, height);
        if width <= 0 || height <= 0 || size.pixel_count() >= Self::OBJECT_SWAP_MINIMUM_PIXELS {
            return size;
        }
        let scale = (Self::OBJECT_SWAP_MINIMUM_PIXELS as f64 / size.pixel_count() as f64).sqrt();
        // The slack keeps 480.0000000001 at 480; the exact result is the fallback if that lands short.
        let scaled = |slack: f64| {
            let even_ceil = |side: i64| ((side as f64 * scale / 2.0 - slack).ceil() as i64) * 2;
            PixelSize::new(even_ceil(width), even_ceil(height))
        };
        let tight = scaled(1e-9);
        if tight.pixel_count() >= Self::OBJECT_SWAP_MINIMUM_PIXELS {
            tight
        } else {
            scaled(0.0)
        }
    }
}
