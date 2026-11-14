//! Edit… and Extend… on a finished result (`ResultFollowUp.swift`): the result becomes the source of a Seedance job.

use jutsu_core::{ClosedRange, Job, JobSpec, Mode, PixelSize, SourceClip, Validation};

use super::form::CreateForm;

/// The inputs of an Edit or Extend (`mode`) of `job`'s result, for Create to open with the result as the source on
/// Seedance 2.5, at the result's resolution. The prompt is left for the user: Edit and Extend need their own. Edit
/// selects up to the first 30 s, Extend up to the last 30 s, ending at the end of the clip. `None` for another mode,
/// or without a downloaded result.
pub fn follow_up(job: &Job, mode: Mode) -> Option<JobSpec> {
    if !mode.is_video_operation() {
        return None;
    }
    let file = job.output_file.as_ref().filter(|file| file.scheme() == "file")?;
    // Create probes the file again and `CreateForm::load` fits the range to it.
    let duration = job.output_duration().max(0.0);
    let window = Validation::DURATION_RANGE.upper();
    let range = if mode == Mode::Extend {
        ClosedRange::new((duration - window).max(0.0), duration)
    } else {
        ClosedRange::new(0.0, duration.min(window))
    };
    let source = SourceClip::new(
        file.clone(),
        duration,
        job.output_pixel_size().unwrap_or(PixelSize::new(0, 0)),
        range,
    );
    let mut spec = JobSpec::new(mode, mode.default_model(), Some(source), job.spec.resolution);
    spec.length = (mode == Mode::Extend).then_some(CreateForm::DEFAULT_LENGTH);
    Some(spec)
}
