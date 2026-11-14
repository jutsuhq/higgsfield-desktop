//! Settings › General (`GeneralSettingsView.swift`): the mode Create starts in, prompt history, the system tray, opening
//! at login and Software Update.

use jutsu_core::Mode;
use serde::Serialize;

use crate::connection::mode_title;
use crate::preferences::{Preferences, PreferencesError, PreferencesStore};
use crate::system::updates::{UpdatePreferences, UpdaterAvailability};

/// The Open at login switch, from the autostart entry. macOS's "requires approval" state has no equivalent here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItemStatus {
    Enabled,
    Disabled,
}

impl LoginItemStatus {
    pub fn is_on(self) -> bool {
        self == Self::Enabled
    }
}

/// `MenuBarMode.showsExtra`: the tray icon is the way back to a closed window in tray mode, so the mode shows it
/// whatever Show in the system tray says.
pub fn shows_tray_icon(show_in_tray: bool, runs_in_tray: bool) -> bool {
    show_in_tray || runs_in_tray
}

/// "Version 0.1.0 (1)" (`AboutContent.versionText`).
pub fn version_text(version: Option<&str>, build: Option<&str>) -> String {
    match (version, build) {
        (Some(version), Some(build)) if version != build => format!("Version {version} ({build})"),
        (Some(version), _) => format!("Version {version}"),
        (None, Some(build)) => format!("Build {build}"),
        (None, None) => "Development build".to_owned(),
    }
}

/// One choice of a segmented control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ModeOption {
    pub value: Mode,
    pub title: String,
}

/// What the General tab shows.
#[allow(clippy::struct_excessive_bools, reason = "one per switch of the tab")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct GeneralSettings {
    pub default_mode: Mode,
    /// The modes Create starts in: not Edit and Extend, which need a clip first.
    pub modes: Vec<ModeOption>,
    /// Show in the system tray as the switch shows it: on while tray mode forces the icon.
    pub show_in_tray: bool,
    /// Off while tray mode forces the icon.
    pub can_change_show_in_tray: bool,
    pub run_in_tray: bool,
    pub show_active_count: bool,
    /// The count sits next to the tray icon, so it needs one.
    pub can_show_active_count: bool,
    pub open_at_login: bool,
    pub system_footer: String,
    pub version_text: String,
    pub last_check_text: String,
    /// Whether the updater runs in this build; its switches are off until it does.
    pub updates_enabled: bool,
    pub updates_footer: String,
    /// Check automatically, once a day.
    pub checks_automatically: bool,
    /// Download and install automatically; it follows Check automatically.
    pub downloads_automatically: bool,
    /// Download and install automatically can change, and isn't dimmed: only while Check automatically is on, in a build
    /// with updates.
    pub can_change_downloads_automatically: bool,
    /// Off while a check or an install is under way, and in a build without updates.
    pub can_check_for_updates: bool,
}

/// The updater as the Updates rows show it.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdatesState {
    pub availability: UpdaterAvailability,
    pub preferences: UpdatePreferences,
    /// "Last checked Nov 13, 2026 at 3:04 PM".
    pub last_check_text: String,
    pub is_busy: bool,
}

impl UpdatesState {
    /// A build without updates, never checked.
    pub fn unavailable() -> Self {
        Self {
            availability: UpdaterAvailability::DevelopmentBuild,
            preferences: UpdatePreferences {
                checks_automatically: true,
                downloads_automatically: false,
                last_check: None,
            },
            last_check_text: "Not checked yet".to_owned(),
            is_busy: false,
        }
    }
}

/// The tab for `preferences`, the autostart entry, the app's version and the updater.
pub fn general_settings(
    preferences: &Preferences,
    login: LoginItemStatus,
    version: &str,
    updates: &UpdatesState,
) -> GeneralSettings {
    let updates_enabled = updates.availability.is_available();
    let show_in_tray = preferences
        .show_in_menu_bar
        .unwrap_or(Preferences::DEFAULT_SHOW_IN_MENU_BAR);
    let run_in_tray = preferences.runs_in_menu_bar();
    let shows_icon = shows_tray_icon(show_in_tray, run_in_tray);
    let system_footer = if run_in_tray {
        "In the system tray, Jutsu Studio opens at login with no window."
    } else {
        "Closing the window quits Jutsu Studio unless jobs are running."
    };
    GeneralSettings {
        default_mode: preferences.default_mode(),
        modes: Mode::ALL
            .iter()
            .filter(|mode| !mode.is_video_operation())
            .map(|&mode| ModeOption {
                value: mode,
                title: mode_title(mode).to_owned(),
            })
            .collect(),
        show_in_tray: shows_icon,
        can_change_show_in_tray: !run_in_tray,
        run_in_tray,
        show_active_count: preferences
            .show_active_count
            .unwrap_or(Preferences::DEFAULT_SHOW_ACTIVE_COUNT),
        can_show_active_count: shows_icon,
        open_at_login: login.is_on(),
        system_footer: system_footer.to_owned(),
        version_text: version_text(Some(version), None),
        last_check_text: updates.last_check_text.clone(),
        updates_enabled,
        updates_footer: updates.availability.footer().to_owned(),
        checks_automatically: updates.preferences.checks_automatically,
        downloads_automatically: updates.preferences.downloads_automatically,
        can_change_downloads_automatically: updates_enabled && updates.preferences.checks_automatically,
        can_check_for_updates: updates_enabled && !updates.is_busy,
    }
}

/// Clear History…: prompts of jobs before `now` (seconds since 1970) leave the prompt picker.
pub fn clear_prompt_history(store: &PreferencesStore, now: f64) -> Result<Preferences, PreferencesError> {
    store.update(Preferences {
        prompt_history_cleared_at: Some(now),
        ..Preferences::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_text_names_version_and_build() {
        assert_eq!(version_text(Some("0.1.0"), Some("1")), "Version 0.1.0 (1)");
        assert_eq!(version_text(Some("0.1.0"), Some("0.1.0")), "Version 0.1.0");
        assert_eq!(version_text(Some("0.1.0"), None), "Version 0.1.0");
        assert_eq!(version_text(None, Some("7")), "Build 7");
        assert_eq!(version_text(None, None), "Development build");
    }

    #[test]
    fn defaults_and_tray_mode() {
        let settings = general_settings(
            &Preferences::default(),
            LoginItemStatus::Disabled,
            "0.1.0",
            &UpdatesState::unavailable(),
        );
        assert_eq!(settings.default_mode, Mode::MotionTransfer);
        assert_eq!(
            settings
                .modes
                .iter()
                .map(|mode| mode.title.as_str())
                .collect::<Vec<_>>(),
            ["Motion transfer", "Restyle", "Object swap"]
        );
        assert!(settings.show_in_tray && settings.can_change_show_in_tray);
        assert!(!settings.run_in_tray && !settings.show_active_count && settings.can_show_active_count);
        assert!(!settings.open_at_login);
        assert_eq!(
            settings.system_footer,
            "Closing the window quits Jutsu Studio unless jobs are running."
        );
        assert_eq!(settings.version_text, "Version 0.1.0");
        assert!(!settings.updates_enabled && !settings.can_check_for_updates);
        assert_eq!(settings.updates_footer, "Updates are off in development builds.");

        let hidden = Preferences {
            show_in_menu_bar: Some(false),
            ..Preferences::default()
        };
        let settings = general_settings(
            &hidden,
            LoginItemStatus::Enabled,
            "0.1.0",
            &UpdatesState::unavailable(),
        );
        assert!(!settings.show_in_tray && !settings.can_show_active_count);
        assert!(settings.open_at_login);

        let tray = Preferences {
            show_in_menu_bar: Some(false),
            run_in_menu_bar: Some(true),
            ..Preferences::default()
        };
        let settings = general_settings(
            &tray,
            LoginItemStatus::Enabled,
            "0.1.0",
            &UpdatesState::unavailable(),
        );
        assert!(settings.show_in_tray && !settings.can_change_show_in_tray && settings.can_show_active_count);
        assert_eq!(
            settings.system_footer,
            "In the system tray, Jutsu Studio opens at login with no window."
        );
    }

    #[test]
    fn clearing_prompt_history_stores_the_time() {
        let folder = tempfile::tempdir().unwrap();
        let store = PreferencesStore::open(folder.path());
        clear_prompt_history(&store, 1_700_000_000.0).unwrap();
        assert_eq!(store.get().prompt_history_cleared_at, Some(1_700_000_000.0));
    }
}

#[cfg(test)]
mod update_rows_tests {
    use super::*;
    use crate::system::updates::{UpdaterConfig, last_check_text};

    #[test]
    fn the_update_rows_follow_the_build_and_the_switches() {
        let state = |availability, is_busy| UpdatesState {
            availability,
            preferences: UpdatePreferences {
                checks_automatically: false,
                downloads_automatically: true,
                last_check: None,
            },
            last_check_text: last_check_text(None, time::UtcOffset::UTC),
            is_busy,
        };
        let release = UpdaterAvailability::new(false, &UpdaterConfig::default());
        let settings = general_settings(
            &Preferences::default(),
            LoginItemStatus::Disabled,
            "0.1.0",
            &state(release, false),
        );
        assert!(!settings.updates_enabled && !settings.can_check_for_updates);
        assert_eq!(settings.updates_footer, "Updates aren't available in this build.");
        assert_eq!(settings.last_check_text, "Not checked yet");

        let available = state(UpdaterAvailability::Available, false);
        let settings = general_settings(
            &Preferences::default(),
            LoginItemStatus::Disabled,
            "0.1.0",
            &available,
        );
        assert!(settings.updates_enabled && settings.can_check_for_updates);
        assert!(!settings.checks_automatically && settings.downloads_automatically);
        // Download and install automatically follows Check automatically (GeneralSettingsView).
        assert!(!settings.can_change_downloads_automatically);
        let mut checking = available.clone();
        checking.preferences.checks_automatically = true;
        assert!(
            general_settings(
                &Preferences::default(),
                LoginItemStatus::Disabled,
                "0.1.0",
                &checking
            )
            .can_change_downloads_automatically
        );
        let busy = state(UpdaterAvailability::Available, true);
        assert!(
            !general_settings(&Preferences::default(), LoginItemStatus::Disabled, "0.1.0", &busy)
                .can_check_for_updates
        );
    }
}
