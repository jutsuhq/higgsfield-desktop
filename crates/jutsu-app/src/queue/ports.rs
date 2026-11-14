//! The engine's system ports on Windows and Linux: whether the computer is online (`PathConnectivity` on macOS), and
//! media preparation when ffmpeg is missing.

use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use jutsu_core::{
    Connectivity, JobSpec, LocalAudio, LocalImage, MediaPreparing, OutputMedia, PixelSize, PortError,
    PreparedFile, Progress, SourceClip,
};
use url::Url;

/// Online unless a cheap check says otherwise: every few seconds it asks the system for a route to a public
/// address. Connecting a UDP socket only picks a route; nothing is sent. Reports the current state first, then
/// every change.
#[derive(Debug, Clone)]
pub struct RouteConnectivity {
    pub interval: Duration,
}

impl Default for RouteConnectivity {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(5),
        }
    }
}

impl RouteConnectivity {
    const TARGETS: [&'static str; 2] = ["1.1.1.1:443", "[2606:4700:4700::1111]:443"];

    /// Whether the system has a route to the internet over IPv4 or IPv6.
    pub fn has_route() -> bool {
        Self::TARGETS.iter().any(|target| {
            let Ok(address) = target.parse::<SocketAddr>() else {
                return false;
            };
            let local: SocketAddr = if address.is_ipv4() {
                ([0, 0, 0, 0], 0).into()
            } else {
                ([0u16; 8], 0).into()
            };
            UdpSocket::bind(local)
                .and_then(|socket| socket.connect(address))
                .is_ok()
        })
    }
}

impl Connectivity for RouteConnectivity {
    fn updates(&self) -> BoxStream<'static, bool> {
        let interval = self.interval;
        futures::stream::unfold(None::<bool>, move |last| async move {
            loop {
                if last.is_some() {
                    tokio::time::sleep(interval).await;
                }
                let online = tokio::task::spawn_blocking(Self::has_route).await.unwrap_or(true);
                if last != Some(online) {
                    return Some((online, Some(online)));
                }
            }
        })
        .boxed()
    }
}

/// Media preparation without ffmpeg: the app starts, and every job that needs its inputs prepared fails with this.
#[derive(Debug, Clone, Copy, Default)]
pub struct MissingFfmpeg;

impl MissingFfmpeg {
    pub const MESSAGE: &'static str = "Jutsu Studio needs ffmpeg and ffprobe to prepare videos. Install them, make sure they are on the PATH, then restart Jutsu Studio and retry.";

    fn error() -> PortError {
        PortError::Other(Self::MESSAGE.to_owned())
    }
}

#[async_trait]
impl MediaPreparing for MissingFfmpeg {
    async fn prepare_video(
        &self,
        _source: &SourceClip,
        _progress: Progress<f64>,
    ) -> Result<PreparedFile, PortError> {
        Err(Self::error())
    }

    async fn prepare_image(&self, _image: &LocalImage) -> Result<PreparedFile, PortError> {
        Err(Self::error())
    }

    async fn prepare_audio(&self, _audio: &LocalAudio) -> Result<PreparedFile, PortError> {
        Err(Self::error())
    }

    async fn restore_audio(&self, _spec: &JobSpec, _result: &Url, _output: &Url) -> Result<bool, PortError> {
        Err(Self::error())
    }

    async fn image_size(&self, _image: &LocalImage) -> Option<PixelSize> {
        None
    }

    async fn output_media(&self, _file: &Url) -> Option<OutputMedia> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn without_ffmpeg_preparing_fails_with_a_clear_message() {
        let image = LocalImage::new(Url::parse("file:///tmp/a.png").unwrap());
        let error = MissingFfmpeg.prepare_image(&image).await.unwrap_err();
        assert!(error.to_string().contains("ffmpeg"));
        assert_eq!(MissingFfmpeg.image_size(&image).await, None);
    }
}
