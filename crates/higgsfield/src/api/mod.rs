//! REST API at `api.higgsfield.ai`, authorized with an API key.

mod client;
pub(crate) mod errors;
mod requests;
mod wire;

pub use client::HiggsfieldApiClient;
pub(crate) use client::file_put;
pub use requests::{
    GenerationRequest, GrokImageRequest, Kling4Request, KlingMotionRequest, KlingQuality,
    KlingVideoEditRequest, KlingVideoReferenceRequest, Orientation, OriginalSound, QwenEditRequest,
    ReferenceToVideoRequest, SeedanceFramesRequest, SeedanceRequest, SeedanceVideoRequest, SoulImageRequest,
};
pub use wire::{ModelEndpoint, Preset, SubmittedRequest, UploadTarget};
