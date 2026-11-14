//! The About window (`AboutView.swift`, `AboutContent.swift`): the version, the disclaimer, the license and the
//! credits, and the notices of the programs the installers include or download.
//!
//! The macOS app credits Sparkle and its Parakeet speech model there. Here Software Update is Tauri's own plugin, so
//! that line goes, and the speech model's credit is Whisper's; `FFmpeg`, Real-ESRGAN and `whisper.cpp` join them, with
//! `THIRD_PARTY_NOTICES.md` in full behind a button.

use serde::Serialize;

use crate::settings::version_text;
use crate::subtitles::text::WHISPER_CREDIT;

pub const DISCLAIMER: &str = "Unofficial, not affiliated with or endorsed by Higgsfield AI.";
pub const LICENSE: &str = "Released under the MIT License.";
pub const FONT_CREDIT: &str = "Barlow Condensed by The Barlow Project Authors, SIL Open Font License 1.1.";
pub const FFMPEG_CREDIT: &str = "Video by FFmpeg, GNU Lesser General Public License 2.1 or later.";
pub const UPSCALE_CREDIT: &str = "Upscale on this computer by Real-ESRGAN ncnn Vulkan, BSD 3-Clause License.";
pub const REPOSITORY: &str = "https://github.com/jutsuhq/jutsu-studio";
pub const REPOSITORY_TITLE: &str = "github.com/jutsuhq/jutsu-studio";
pub const NOTICES_TITLE: &str = "Third-Party Notices";

/// `THIRD_PARTY_NOTICES.md` as the repository has it.
const NOTICES: &str = include_str!("../../../../THIRD_PARTY_NOTICES.md");

/// What the About window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct About {
    /// "Version 0.1.0".
    pub version: String,
    pub disclaimer: String,
    /// The small lines under the disclaimer: the license, then the credits.
    pub credits: Vec<String>,
    pub repository: String,
    pub repository_title: String,
    pub notices_title: String,
    /// The notices' text, Markdown as written.
    pub notices: String,
}

/// The About window of version `version` (`CARGO_PKG_VERSION`); a build number exists only on macOS.
pub fn about(version: Option<&str>) -> About {
    About {
        version: version_text(version, None),
        disclaimer: DISCLAIMER.to_owned(),
        credits: [
            LICENSE,
            FONT_CREDIT,
            FFMPEG_CREDIT,
            UPSCALE_CREDIT,
            WHISPER_CREDIT,
        ]
        .map(str::to_owned)
        .to_vec(),
        repository: REPOSITORY.to_owned(),
        repository_title: REPOSITORY_TITLE.to_owned(),
        notices_title: NOTICES_TITLE.to_owned(),
        notices: NOTICES.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_every_program_the_installers_include_or_download() {
        let about = about(Some("0.1.0"));
        for name in ["FFmpeg", "Real-ESRGAN", "whisper.cpp"] {
            assert!(about.credits.iter().any(|line| line.contains(name)), "{name}");
            assert!(about.notices.contains(name), "{name}");
        }
        assert!(about.notices.contains("Lesser General Public License"));
    }
}
