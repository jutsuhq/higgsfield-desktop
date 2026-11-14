//! File names for editing exports and delivery sets.

use std::collections::{BTreeSet, HashMap, HashSet};

use uuid::Uuid;

use crate::models::{DeliveryOutput, ExportPreset, JobSpec, Mode};
use crate::rules::{OutputNamePattern, SubtitleFormat};
use crate::support::Seconds;
use crate::support::range::ClosedRange;
use crate::support::text::{drop_last_char, trim_whitespace};
use crate::support::url_ext::{deleting_path_extension, last_path_component};

/// The names of one editing export's files.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExportNames {
    /// The video.
    pub video: String,
    /// The JSON sidecar.
    pub sidecar: String,
    /// The take's subtitles, when it has them.
    pub subtitles: String,
}

/// File names for editing exports, `<source>_<range>_<mode>_<style or refs>_v<N>.<ext>`, e.g.
/// `rooftop_00m04.0s-00m10.0s_restyle_Film Noir_v2.mov`. Takes of one shot sort together and in order.
pub struct ExportNaming;

impl ExportNaming {
    const MAX_SOURCE_BYTES: usize = 120;
    const MAX_STYLE_BYTES: usize = 40;
    /// Characters other file systems an editor's media may live on (exFAT, SMB) reject.
    const UNSAFE: [char; 7] = ['\\', '*', '?', '"', '<', '>', '|'];

    /// The first version whose video, under any preset's extension, sidecar and subtitles names are all free, so a
    /// new take never overwrites one and never shares a version number with an earlier take.
    pub fn names(
        spec: &JobSpec,
        preset_name: Option<&str>,
        preset: ExportPreset,
        is_taken: impl Fn(&str) -> bool,
    ) -> ExportNames {
        let base = Self::base_name(spec, preset_name);
        let mut extensions: Vec<&str> = ExportPreset::ALL
            .iter()
            .map(|preset| preset.file_extension())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        extensions.extend(["json", SubtitleFormat::Srt.file_extension()]);
        let mut version = 1;
        while extensions
            .iter()
            .any(|ext| is_taken(&format!("{base}_v{version}.{ext}")))
        {
            version += 1;
        }
        let stem = format!("{base}_v{version}");
        ExportNames {
            video: format!("{stem}.{}", preset.file_extension()),
            sidecar: format!("{stem}.json"),
            subtitles: format!("{stem}.{}", SubtitleFormat::Srt.file_extension()),
        }
    }

    /// The name before `_v<N>`.
    pub fn base_name(spec: &JobSpec, preset_name: Option<&str>) -> String {
        let source = Self::field(&spec.name(), Self::MAX_SOURCE_BYTES);
        let range = spec
            .source
            .as_ref()
            .map_or(ClosedRange::new(0.0, spec.duration()), |source| source.range);
        let fields = [
            if source.is_empty() {
                "clip".to_owned()
            } else {
                source
            },
            Self::range_text(range),
            Self::mode_text(spec.mode).to_owned(),
            Self::field(&Self::style_text(spec, preset_name), Self::MAX_STYLE_BYTES),
        ];
        join_fields(&fields)
    }

    /// `00m04.0s-00m10.0s`: zero-padded so names sort by start time for ranges under 100 minutes.
    pub fn range_text(range: ClosedRange<Seconds>) -> String {
        format!("{}-{}", time_text(range.lower()), time_text(range.upper()))
    }

    /// The mode's part of a name.
    pub fn mode_text(mode: Mode) -> &'static str {
        match mode {
            Mode::MotionTransfer => "motion",
            Mode::Restyle => "restyle",
            Mode::ObjectSwap => "swap",
            Mode::Edit => "edit",
            Mode::Extend => "extend",
        }
    }

    /// The preset for restyle; otherwise the first reference image's name, `+N` for the others.
    fn style_text(spec: &JobSpec, preset_name: Option<&str>) -> String {
        if spec.mode == Mode::Restyle {
            let fallback = if spec.preset_id.is_none() { "" } else { "preset" };
            return preset_name.unwrap_or(fallback).to_owned();
        }
        let Some(first) = spec.references.first() else {
            return String::new();
        };
        let name = deleting_path_extension(&last_path_component(&first.file_url)).to_owned();
        if spec.references.len() > 1 {
            format!("{name}+{}", spec.references.len() - 1)
        } else {
            name
        }
    }

    /// `<set>_<NN>_<item>_<output>`, e.g. `Spring launch_03_knight_web`: a set's files sort by item, then output.
    /// `NN` is the 1-based position, zero-padded to the width of `count` (at least two digits).
    pub fn delivery_base_name(set: &str, position: usize, count: usize, item: &str, output: &str) -> String {
        let digits = 2.max(count.max(1).to_string().len());
        let fields = [
            Self::field(set, Self::MAX_STYLE_BYTES),
            format!("{position:0>digits$}"),
            Self::field(item, Self::MAX_STYLE_BYTES),
            Self::field(output, Self::MAX_STYLE_BYTES),
        ];
        join_fields(&fields)
    }

    /// The JSON written next to a delivery's files.
    pub fn delivery_manifest_name(set: &str) -> String {
        let name = Self::field(set, Self::MAX_SOURCE_BYTES);
        format!(
            "{}_manifest.json",
            if name.is_empty() { "delivery" } else { &name }
        )
    }

    /// The file-name part of each output, in order: its name, else its summary; a repeat gets `-2`, `-3`…
    pub fn output_labels(outputs: &[DeliveryOutput]) -> HashMap<Uuid, String> {
        let mut used: HashSet<String> = HashSet::new();
        let mut labels = HashMap::new();
        for output in outputs {
            let text = if output.name.is_empty() {
                output.summary()
            } else {
                output.name.clone()
            };
            let base = Self::field(&text, Self::MAX_STYLE_BYTES);
            let mut label = base.clone();
            let mut number = 2;
            while !used.insert(label.to_lowercase()) {
                label = format!("{base}-{number}");
                number += 1;
            }
            labels.insert(output.id, label);
        }
        labels
    }

    /// One name field, safe as part of a path component on macOS and the usual shared volumes.
    pub fn field(text: &str, max_bytes: usize) -> String {
        let mut result: String = OutputNamePattern::sanitize(text)
            .chars()
            .map(|c| if Self::UNSAFE.contains(&c) { '-' } else { c })
            .collect();
        while result.len() > max_bytes {
            let kept = drop_last_char(&result).len();
            result.truncate(kept);
        }
        trim_whitespace(&result).to_owned()
    }
}

fn join_fields(fields: &[String]) -> String {
    fields
        .iter()
        .filter(|field| !field.is_empty())
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("_")
}

fn time_text(seconds: Seconds) -> String {
    let tenths = if seconds.is_finite() {
        ((seconds * 10.0).round() as i64).max(0)
    } else {
        0
    };
    format!("{:02}m{:02}.{}s", tenths / 600, tenths % 600 / 10, tenths % 10)
}
