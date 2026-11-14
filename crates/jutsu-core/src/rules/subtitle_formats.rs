//! Subtitle files.

use crate::rules::SubtitleCue;
use crate::support::Seconds;
use crate::support::string_enum;

string_enum! {
    /// Subtitle files: SubRip (`.srt`) and WebVTT (`.vtt`), timed as the cues are, from the start of the video.
    pub enum SubtitleFormat {
        /// SubRip.
        Srt = "srt",
        /// WebVTT.
        Vtt = "vtt",
    }
}

impl SubtitleFormat {
    /// "srt" or "vtt".
    pub fn file_extension(self) -> &'static str {
        self.raw_value()
    }

    /// The file's text, UTF-8, with "\n" line ends. Cues without text are left out; an arrow in a cue's text is
    /// written as "→", as "-->" would end its timing line.
    pub fn text(self, cues: &[SubtitleCue]) -> String {
        let shown = cues.iter().filter(|cue| !cue.lines().is_empty());
        match self {
            Self::Srt => shown
                .enumerate()
                .map(|(offset, cue)| {
                    format!(
                        "{}\n{} --> {}\n{}\n",
                        offset + 1,
                        Self::timestamp(cue.start, ","),
                        Self::timestamp(cue.end, ","),
                        Self::body(cue)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Vtt => {
                let cues = shown
                    .map(|cue| {
                        format!(
                            "{} --> {}\n{}\n",
                            Self::timestamp(cue.start, "."),
                            Self::timestamp(cue.end, "."),
                            Self::body(cue)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("WEBVTT\n\n{cues}")
            }
        }
    }

    fn body(cue: &SubtitleCue) -> String {
        cue.lines()
            .iter()
            .map(|line| line.replace("-->", "→"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// "00:01:02,345" for SRT, "00:01:02.345" for VTT; rounded to the millisecond.
    pub fn timestamp(seconds: Seconds, separator: &str) -> String {
        let total = (seconds.max(0.0) * 1000.0).round() as i64;
        let hours = total / 3_600_000;
        let minutes = total / 60_000 % 60;
        let secs = total / 1000 % 60;
        let millis = total % 1000;
        format!("{hours:02}:{minutes:02}:{secs:02}{separator}{millis:03}")
    }
}
