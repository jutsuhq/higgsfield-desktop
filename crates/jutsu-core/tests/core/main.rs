//! The ported `JutsuCoreTests`: one module per Swift test file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers fail the test the way `#require` does"
)]
#![allow(clippy::assert_is_empty, reason = "reads as the Swift `#expect(x.isEmpty)`")]
#![allow(
    clippy::doc_markdown,
    reason = "as in the library: the doc comments are the Swift ones"
)]
#![allow(clippy::cast_precision_loss, reason = "small test indices")]

mod common;

mod agent_money;
mod aspect_label;
mod audio_plan;
mod budget;
mod cinema_controls;
mod credential;
mod cue_builder;
mod delivery_set;
mod editing_export;
mod failure_reason;
mod generation_times;
mod history_item;
mod job_template;
mod job_transition;
mod jobs;
mod kling_o3;
mod media_link;
mod model_constraints;
mod model_resolutions;
mod models;
mod on_device_upscale;
mod output_name_pattern;
mod output_subfolder;
mod post_operation;
mod pricing;
mod reference_image;
mod saved_image;
mod server_files;
mod smooth_motion;
mod speech_selection;
mod validation;
mod variant;
mod video_model_catalog;
