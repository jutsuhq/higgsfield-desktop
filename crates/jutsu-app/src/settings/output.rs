//! Settings › Output (`OutputSettingsView.swift`): the output folder, the resolutions, parallel jobs and the file names
//! of results.

use std::path::Path;

use jutsu_core::{Cost, Job, JobSpec, Mode, OutputNamePattern, OutputNameToken, ProviderKind, Resolution};
use jutsu_store::FileLayout;
use rust_decimal::Decimal;
use serde::Serialize;
use time::{OffsetDateTime, UtcOffset};
use uuid::Uuid;

use super::output_folder::current_output_folder;
use crate::preferences::Preferences;

/// A ready-made file name pattern for the Templates menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OutputNameTemplate {
    pub title: String,
    pub pattern: String,
}

/// Ready-made patterns; the first is the default (`AppSettings.outputNameTemplates`).
pub fn output_name_templates() -> Vec<OutputNameTemplate> {
    [
        ("Date, mode, ID (default)", OutputNamePattern::DEFAULT_PATTERN),
        ("Date and time, mode, ID", "{date}_{time}_{mode}_{id}"),
        ("Mode, preset, resolution, ID", "{mode}_{preset}_{res}_{id}"),
        (
            "Date, draft mark, mode, resolution, ID",
            "{date}_{draft}_{mode}_{res}_{id}",
        ),
    ]
    .into_iter()
    .map(|(title, pattern)| OutputNameTemplate {
        title: title.to_owned(),
        pattern: pattern.to_owned(),
    })
    .collect()
}

/// What a token stands for, with an example (`OutputNamePattern.Token.meaning`).
pub fn token_meaning(token: OutputNameToken) -> &'static str {
    match token {
        OutputNameToken::Date => "date the job was queued, 2026-03-07",
        OutputNameToken::Time => "time it was queued, 14-05-09",
        OutputNameToken::Mode => "mode, restyle",
        OutputNameToken::Preset => "preset name, empty without a preset",
        OutputNameToken::Res => "resolution, 720p",
        OutputNameToken::Draft => "\"draft\" for drafts, otherwise empty",
        OutputNameToken::Id => "first 8 characters of the job ID",
    }
}

/// The pattern field's hover help: one token per line.
pub fn token_help() -> String {
    OutputNameToken::ALL
        .iter()
        .map(|&token| format!("{} — {}", token.placeholder(), token_meaning(token)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The note under the pattern, warning first when the pattern falls back to the default.
pub fn name_footer(pattern: &str) -> String {
    let tokens = OutputNameToken::ALL
        .iter()
        .map(|token| token.placeholder())
        .collect::<Vec<_>>()
        .join(" ");
    let usage =
        format!("Tokens: {tokens}. Applies to new results only; a name already in the folder gets -2, -3…");
    if OutputNamePattern::is_valid(pattern) {
        usage
    } else {
        format!("Empty pattern or unknown token: the default is used. {usage}")
    }
}

/// The name `pattern` gives a 720p restyle with the "Claymation" preset queued at `date`
/// (`AppSettings.outputNameExample`).
pub fn output_name_example(pattern: &str, date: OffsetDateTime, offset: UtcOffset) -> String {
    let mut spec = JobSpec::new(
        Mode::Restyle,
        Mode::Restyle.default_model(),
        None,
        Resolution::P720,
    );
    spec.preset_id = Some(Uuid::new_v4());
    let id = Uuid::from_bytes([0x1a, 0x2b, 0x3c, 0x4d, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let job = Job::new(
        id,
        spec,
        ProviderKind::ApiKey,
        Cost::usd(Decimal::ZERO, true),
        Uuid::new_v4(),
        date,
    );
    OutputNamePattern::new(pattern).file_name(&job, Some("Claymation"), offset, None)
}

/// The live example under the pattern field, and its note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OutputNamePreview {
    pub example: String,
    pub footer: String,
}

/// The preview of `pattern` now, in the window's time zone, `utc_offset_minutes` east of UTC.
pub fn output_name_preview(pattern: &str, utc_offset_minutes: i32) -> OutputNamePreview {
    let offset =
        UtcOffset::from_whole_seconds(utc_offset_minutes.saturating_mul(60)).unwrap_or(UtcOffset::UTC);
    OutputNamePreview {
        example: output_name_example(pattern, OffsetDateTime::now_utc(), offset),
        footer: name_footer(pattern),
    }
}

/// The folder as the user knows it: inside the home folder it starts with `~` (`AppSettings.displayPath`).
pub fn display_path(path: &Path, home: &Path) -> String {
    if home.components().count() < 2 {
        return path.display().to_string();
    }
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// What the Output tab shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OutputSettings {
    /// The folder results download to.
    pub folder: String,
    /// `folder` with the home folder as `~`.
    pub folder_display: String,
    pub is_custom_folder: bool,
    pub resolution: Resolution,
    pub draft_resolution: Resolution,
    /// The defaults every model without its own list renders at.
    pub resolutions: Vec<Resolution>,
    pub max_concurrency: i32,
    pub min_concurrency: i32,
    pub max_concurrency_limit: i32,
    pub concurrency_detail: String,
    /// The pattern as stored, which may be invalid; the default when none is.
    pub name_pattern: String,
    pub templates: Vec<OutputNameTemplate>,
    pub token_help: String,
}

/// The tab for `preferences` and the connection of `kind`; `home` is the user's home folder.
pub fn output_settings(
    preferences: &Preferences,
    layout: &FileLayout,
    kind: Option<ProviderKind>,
    home: &Path,
) -> OutputSettings {
    let folder = current_output_folder(preferences, layout);
    let narrow = |value: i64| i32::try_from(value).unwrap_or(i32::MAX);
    OutputSettings {
        folder: folder.display().to_string(),
        folder_display: display_path(&folder, home),
        is_custom_folder: preferences.output_folder.is_some(),
        resolution: preferences.default_resolution(),
        draft_resolution: preferences.draft_resolution(),
        resolutions: Resolution::STANDARD.to_vec(),
        max_concurrency: narrow(preferences.max_concurrency(kind)),
        min_concurrency: narrow(Preferences::MAX_CONCURRENCY_RANGE.0),
        max_concurrency_limit: narrow(Preferences::MAX_CONCURRENCY_RANGE.1),
        concurrency_detail: format!(
            "Generating at once. By default {} on an account, {} with an API key",
            Preferences::DEFAULT_ACCOUNT_MAX_CONCURRENCY,
            Preferences::DEFAULT_MAX_CONCURRENCY
        ),
        name_pattern: preferences
            .output_name_pattern
            .clone()
            .unwrap_or_else(|| OutputNamePattern::DEFAULT_PATTERN.to_owned()),
        templates: output_name_templates(),
        token_help: token_help(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn every_name_template_is_valid_and_the_first_is_the_default() {
        let templates = output_name_templates();
        assert_eq!(templates[0].pattern, OutputNamePattern::DEFAULT_PATTERN);
        assert!(
            templates
                .iter()
                .all(|template| OutputNamePattern::is_valid(&template.pattern))
        );
        let patterns: HashSet<_> = templates.iter().map(|template| &template.pattern).collect();
        assert_eq!(patterns.len(), templates.len());
    }

    #[test]
    fn output_name_example_is_a_claymation_restyle() {
        // 2025-10-13 12:00 UTC.
        let date = OffsetDateTime::from_unix_timestamp(1_760_356_800).unwrap();
        assert_eq!(
            output_name_example("{preset} {res} {time}", date, UtcOffset::UTC),
            "Claymation 720p 12-00-00.mp4"
        );
        assert_eq!(
            output_name_example("", date, UtcOffset::UTC),
            "2025-10-13_restyle_1a2b3c4d.mp4"
        );
    }

    #[test]
    fn the_preview_follows_the_windows_time_zone() {
        let preview = output_name_preview("{draft}{id}", 540);
        assert_eq!(preview.example, "1a2b3c4d.mp4");
        assert!(
            preview
                .footer
                .starts_with("Tokens: {date} {time} {mode} {preset} {res} {draft} {id}. ")
        );
        let invalid = output_name_preview("{resolution}", 0);
        assert!(invalid.example.ends_with("_restyle_1a2b3c4d.mp4"));
        assert!(
            invalid
                .footer
                .starts_with("Empty pattern or unknown token: the default is used. Tokens:")
        );
    }

    /// `SettingsTests.displayPath`, without the macOS sandbox container.
    // Unix paths: the rule is the same on Windows, with its own separators and drives.
    #[cfg(unix)]
    #[test]
    fn the_home_folder_is_a_tilde() {
        let home = Path::new("/home/me");
        for (path, expected) in [
            ("/home/me/Videos/Jutsu Studio", "~/Videos/Jutsu Studio"),
            ("/home/me/Library/Preferences", "~/Library/Preferences"),
            ("/Volumes/Media/Renders", "/Volumes/Media/Renders"),
            ("/home/meow/Renders", "/home/meow/Renders"),
            ("/home/me", "~"),
        ] {
            assert_eq!(display_path(Path::new(path), home), expected);
        }
        assert_eq!(
            display_path(Path::new("/home/me/x"), Path::new("/")),
            "/home/me/x"
        );
    }

    // Unix paths: the rule is the same on Windows, with its own separators and drives.
    #[cfg(unix)]
    #[test]
    fn the_tab_reads_the_settings_and_the_connection() {
        let layout = FileLayout::new("/tmp/data", "/home/me/Videos/Jutsu Studio");
        let settings = output_settings(
            &Preferences::default(),
            &layout,
            Some(ProviderKind::Account),
            Path::new("/home/me"),
        );
        assert_eq!(settings.folder_display, "~/Videos/Jutsu Studio");
        assert!(!settings.is_custom_folder);
        assert_eq!(settings.resolution, Resolution::P480);
        assert_eq!(
            settings.resolutions,
            [Resolution::P480, Resolution::P720, Resolution::P1080]
        );
        assert_eq!(settings.max_concurrency, 8);
        assert_eq!(
            (settings.min_concurrency, settings.max_concurrency_limit),
            (1, 40)
        );
        assert_eq!(
            settings.concurrency_detail,
            "Generating at once. By default 8 on an account, 2 with an API key"
        );
        assert_eq!(settings.name_pattern, "{date}_{mode}_{id}");

        let picked = Preferences {
            output_folder: Some("/media/Renders".to_owned()),
            max_concurrency: Some(3),
            output_name_pattern: Some("{bad}".to_owned()),
            ..Preferences::default()
        };
        let settings = output_settings(
            &picked,
            &layout,
            Some(ProviderKind::ApiKey),
            Path::new("/home/me"),
        );
        assert_eq!(PathBuf::from(&settings.folder), PathBuf::from("/media/Renders"));
        assert!(settings.is_custom_folder);
        assert_eq!(settings.max_concurrency, 3);
        assert_eq!(settings.name_pattern, "{bad}");
    }
}
