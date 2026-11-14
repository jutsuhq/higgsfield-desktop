//! Enhance on this computer (Swift's `OnDeviceEnhancer`): Smooth Motion with ffmpeg's `minterpolate` and the macOS
//! cut rule, Upscale 4× with Real-ESRGAN ncnn-vulkan, behind `OnDeviceEnhancing`.

mod raw;
pub mod real_esrgan;
pub mod smooth_motion;
pub mod upscale;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use jutsu_core::{
    OnDeviceAction, OnDeviceEngine, OnDeviceEnhancing, OnDeviceOutput, OnDeviceProgress, PortError, Progress,
};
use tokio_util::sync::CancellationToken;
use url::Url;

pub use real_esrgan::RealEsrganEngine;
pub use upscale::FrameUpscaling;

use crate::error::MediaError;
use crate::tools::FfmpegTools;

/// Enhance on this computer: ffmpeg for Smooth Motion, the upscaler for Upscale. Without ffmpeg nothing runs.
#[derive(Clone)]
pub struct OnDeviceEnhancer {
    tools: Option<FfmpegTools>,
    upscaler: Arc<dyn FrameUpscaling>,
}

impl std::fmt::Debug for OnDeviceEnhancer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OnDeviceEnhancer")
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}

impl OnDeviceEnhancer {
    pub fn new(tools: Option<FfmpegTools>, upscaler: Arc<dyn FrameUpscaling>) -> Self {
        Self { tools, upscaler }
    }

    /// Why `engine` can't run on this computer; `None` when it can.
    pub fn unavailable_reason(&self, engine: OnDeviceEngine) -> Option<String> {
        if self.tools.is_none() {
            return Some("ffmpeg isn't installed.".to_owned());
        }
        match engine {
            OnDeviceEngine::FrameRateConversion => None,
            OnDeviceEngine::VideoSuperResolution | OnDeviceEngine::ImageSuperResolution => {
                self.upscaler.unavailable_reason()
            }
        }
    }

    /// What the upscaler's download weighs, in bytes.
    pub fn download_size(&self) -> Option<i64> {
        self.upscaler.download_size()
    }

    fn path(url: &Url) -> Result<PathBuf, PortError> {
        url.to_file_path()
            .map_err(|()| PortError::Other(format!("{url} isn't a file on this computer")))
    }
}

#[async_trait]
impl OnDeviceEnhancing for OnDeviceEnhancer {
    fn supports(&self, engine: OnDeviceEngine) -> bool {
        self.unavailable_reason(engine).is_none()
    }

    fn model_is_ready(&self, engine: OnDeviceEngine) -> bool {
        match engine {
            OnDeviceEngine::FrameRateConversion => true,
            OnDeviceEngine::VideoSuperResolution | OnDeviceEngine::ImageSuperResolution => {
                self.upscaler.is_installed()
            }
        }
    }

    /// Cancelling is dropping the returned future: ffmpeg and the upscaler die with their handles.
    async fn run(
        &self,
        action: &OnDeviceAction,
        source: &Url,
        output: &Url,
        progress: Progress<OnDeviceProgress>,
    ) -> Result<OnDeviceOutput, PortError> {
        let Some(tools) = self.tools.as_ref() else {
            return Err(MediaError::EngineUnavailable.into());
        };
        let source = Self::path(source)?;
        let output = Self::path(output)?;
        let cancel = CancellationToken::new();
        let result = match *action {
            OnDeviceAction::SmoothMotion(motion) => {
                let frames = Arc::clone(&progress);
                let fraction: Progress<f64> = Arc::new(move |value| frames(OnDeviceProgress::Working(value)));
                smooth_motion::render(tools, motion, &source, &output, Some(fraction), &cancel).await
            }
            OnDeviceAction::UpscaleVideo(target) => {
                upscale::upscale_video(
                    tools,
                    self.upscaler.as_ref(),
                    target,
                    &source,
                    &output,
                    Some(progress),
                    &cancel,
                )
                .await
            }
            OnDeviceAction::UpscaleImage(target) => {
                upscale::upscale_image(
                    tools,
                    self.upscaler.as_ref(),
                    target,
                    &source,
                    &output,
                    Some(progress),
                    &cancel,
                )
                .await
            }
        };
        result.map_err(PortError::from)
    }
}
