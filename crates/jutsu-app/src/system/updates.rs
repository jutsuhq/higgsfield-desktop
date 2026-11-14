//! Software Update (`SoftwareUpdater.swift` and Settings › General's Updates rows) on `tauri-plugin-updater`. macOS
//! uses Sparkle against the appcast of the latest GitHub release; here the updater reads a release manifest from the
//! configured endpoint and checks each download against the configured public key.
//!
//! The endpoint and the key are build configuration (`plugins.updater` in `tauri.conf.json`). Until both are filled
//! in, the build has no updates: the rows say so and nothing is ever requested.

use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, UtcOffset};

use crate::preferences::{PreferencesError, PreferencesStore};

/// Sparkle's own keys, kept with the other settings: Check automatically, Download and install automatically, and
/// when the last check ran (seconds since 1970).
pub const CHECKS_AUTOMATICALLY_KEY: &str = "SUEnableAutomaticChecks";
pub const DOWNLOADS_AUTOMATICALLY_KEY: &str = "SUAutomaticallyUpdate";
pub const LAST_CHECK_KEY: &str = "SULastCheckTime";

/// Once a day (`SUScheduledCheckInterval`).
pub const CHECK_INTERVAL_SECONDS: f64 = 86_400.0;

/// What the build's `plugins.updater` configuration names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct UpdaterConfig {
    #[serde(default)]
    pub endpoints: Vec<String>,
    #[serde(default)]
    pub pubkey: String,
}

impl UpdaterConfig {
    /// Both an endpoint and a public key are filled in.
    pub fn is_configured(&self) -> bool {
        !self.pubkey.trim().is_empty() && self.endpoints.iter().any(|endpoint| !endpoint.trim().is_empty())
    }
}

/// Whether this build updates itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdaterAvailability {
    /// A debug build: an update would install the release in its place (`SoftwareUpdater.isEnabled`).
    DevelopmentBuild,
    /// A release build without an endpoint or a public key.
    NotConfigured,
    Available,
}

impl UpdaterAvailability {
    pub fn new(is_debug_build: bool, config: &UpdaterConfig) -> Self {
        if is_debug_build {
            Self::DevelopmentBuild
        } else if config.is_configured() {
            Self::Available
        } else {
            Self::NotConfigured
        }
    }

    pub fn is_available(self) -> bool {
        self == Self::Available
    }

    /// The Updates section's footer.
    pub fn footer(self) -> &'static str {
        match self {
            Self::Available => {
                "New versions come from the project's GitHub releases and are checked against its signing key before they install."
            }
            Self::DevelopmentBuild => "Updates are off in development builds.",
            Self::NotConfigured => "Updates aren't available in this build.",
        }
    }
}

/// The two switches and the last check, as stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpdatePreferences {
    pub checks_automatically: bool,
    pub downloads_automatically: bool,
    pub last_check: Option<f64>,
}

impl UpdatePreferences {
    pub fn read(store: &PreferencesStore) -> Self {
        let flag = |key| store.value(key).and_then(|value| value.as_bool());
        Self {
            checks_automatically: flag(CHECKS_AUTOMATICALLY_KEY).unwrap_or(true),
            downloads_automatically: flag(DOWNLOADS_AUTOMATICALLY_KEY).unwrap_or(false),
            last_check: store
                .value(LAST_CHECK_KEY)
                .and_then(|value| value.as_f64())
                .filter(|seconds| seconds.is_finite() && *seconds > 0.0),
        }
    }

    /// An automatic check is due: on, and a day has passed since the last one.
    pub fn is_check_due(&self, now: f64) -> bool {
        self.checks_automatically
            && self
                .last_check
                .is_none_or(|last| now - last >= CHECK_INTERVAL_SECONDS)
    }

    /// Download and install automatically follows Check automatically.
    pub fn downloads(&self) -> bool {
        self.checks_automatically && self.downloads_automatically
    }
}

/// Which switch of the Updates section changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UpdateSwitch {
    ChecksAutomatically,
    DownloadsAutomatically,
}

pub fn set_switch(
    store: &PreferencesStore,
    switch: UpdateSwitch,
    is_on: bool,
) -> Result<(), PreferencesError> {
    let key = match switch {
        UpdateSwitch::ChecksAutomatically => CHECKS_AUTOMATICALLY_KEY,
        UpdateSwitch::DownloadsAutomatically => DOWNLOADS_AUTOMATICALLY_KEY,
    };
    store.set_value(key, Some(serde_json::Value::Bool(is_on)))
}

pub fn record_check(store: &PreferencesStore, now: f64) -> Result<(), PreferencesError> {
    store.set_value(LAST_CHECK_KEY, Some(serde_json::Value::from(now)))
}

/// "Last checked Nov 13, 2026 at 3:04 PM", or "Not checked yet".
pub fn last_check_text(last_check: Option<f64>, offset: UtcOffset) -> String {
    #[allow(clippy::cast_possible_truncation, reason = "seconds since 1970 fit an i64")]
    let date = last_check
        .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds.floor() as i64).ok())
        .map(|date| date.to_offset(offset));
    let Some(date) = date else {
        return "Not checked yet".to_owned();
    };
    let month = match date.month() {
        time::Month::January => "Jan",
        time::Month::February => "Feb",
        time::Month::March => "Mar",
        time::Month::April => "Apr",
        time::Month::May => "May",
        time::Month::June => "Jun",
        time::Month::July => "Jul",
        time::Month::August => "Aug",
        time::Month::September => "Sep",
        time::Month::October => "Oct",
        time::Month::November => "Nov",
        time::Month::December => "Dec",
    };
    let (hour, minute) = (date.hour(), date.minute());
    let (twelve, period) = match hour {
        0 => (12, "AM"),
        1..=11 => (hour, "AM"),
        12 => (12, "PM"),
        _ => (hour - 12, "PM"),
    };
    format!(
        "Last checked {month} {}, {} at {twelve}:{minute:02} {period}",
        date.day(),
        date.year()
    )
}

/// What Check Now answers. With `install_title`, the window offers to install the update found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub title: String,
    pub message: String,
    /// "Install Update" when a newer version was found.
    pub install_title: Option<String>,
}

impl UpdateCheck {
    fn says(title: &str, message: String) -> Self {
        Self {
            title: title.to_owned(),
            message,
            install_title: None,
        }
    }

    /// Check Now in a build without updates: nothing is requested.
    pub fn unavailable(availability: UpdaterAvailability) -> Self {
        let message = match availability {
            UpdaterAvailability::DevelopmentBuild => "Updates are off in development builds.",
            UpdaterAvailability::NotConfigured | UpdaterAvailability::Available => {
                "Updates aren't available in this build."
            }
        };
        Self::says("Can't check for updates", message.to_owned())
    }

    /// Sparkle's texts for the three answers of a check.
    pub fn up_to_date(version: &str) -> Self {
        Self::says(
            "You’re up-to-date!",
            format!("Jutsu Studio {version} is currently the newest version available."),
        )
    }

    pub fn available(new_version: &str, current_version: &str) -> Self {
        Self {
            title: "A new version of Jutsu Studio is available!".to_owned(),
            message: format!(
                "Jutsu Studio {new_version} is now available—you have {current_version}. Would you like to download it now?"
            ),
            install_title: Some("Install Update".to_owned()),
        }
    }

    pub fn failed() -> Self {
        Self::says(
            "Update Error!",
            "An error occurred in retrieving update information. Please try again later.".to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured() -> UpdaterConfig {
        UpdaterConfig {
            endpoints: vec!["https://example.com/latest.json".to_owned()],
            pubkey: "key".to_owned(),
        }
    }

    #[test]
    fn a_build_without_an_endpoint_or_a_key_has_no_updates() {
        assert!(!UpdaterConfig::default().is_configured());
        assert!(
            !UpdaterConfig {
                endpoints: vec![" ".to_owned()],
                pubkey: "key".to_owned()
            }
            .is_configured()
        );
        assert!(
            !UpdaterConfig {
                pubkey: String::new(),
                ..configured()
            }
            .is_configured()
        );
        assert!(configured().is_configured());
        assert_eq!(
            UpdaterAvailability::new(false, &UpdaterConfig::default()),
            UpdaterAvailability::NotConfigured
        );
        assert_eq!(
            UpdaterAvailability::new(true, &configured()),
            UpdaterAvailability::DevelopmentBuild
        );
        assert!(UpdaterAvailability::new(false, &configured()).is_available());
        assert_eq!(
            UpdateCheck::unavailable(UpdaterAvailability::NotConfigured).message,
            "Updates aren't available in this build."
        );
    }

    #[test]
    fn the_switches_default_as_sparkle_and_checks_run_once_a_day() {
        let folder = tempfile::tempdir().unwrap();
        let store = PreferencesStore::open(folder.path());
        let defaults = UpdatePreferences::read(&store);
        assert!(defaults.checks_automatically && !defaults.downloads_automatically);
        assert!(defaults.is_check_due(1_000.0));
        record_check(&store, 1_000.0).unwrap();
        let checked = UpdatePreferences::read(&store);
        assert!(!checked.is_check_due(1_000.0 + CHECK_INTERVAL_SECONDS - 1.0));
        assert!(checked.is_check_due(1_000.0 + CHECK_INTERVAL_SECONDS));
        set_switch(&store, UpdateSwitch::DownloadsAutomatically, true).unwrap();
        assert!(UpdatePreferences::read(&store).downloads());
        set_switch(&store, UpdateSwitch::ChecksAutomatically, false).unwrap();
        let off = UpdatePreferences::read(&store);
        assert!(!off.is_check_due(1_000.0 + 2.0 * CHECK_INTERVAL_SECONDS));
        assert!(!off.downloads());
    }

    #[test]
    fn last_check_text_names_the_day_and_time() {
        assert_eq!(last_check_text(None, UtcOffset::UTC), "Not checked yet");
        // 2026-11-13 15:04:00 UTC.
        assert_eq!(
            last_check_text(Some(1_794_582_240.0), UtcOffset::UTC),
            "Last checked Nov 13, 2026 at 3:04 PM"
        );
    }
}
