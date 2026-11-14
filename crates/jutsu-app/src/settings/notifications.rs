//! Settings › Notifications (`NotificationSettingsView.swift`): which job notifications post, and their sound, and the
//! row saying the system blocks them. Windows tells that through its registry; Linux desktops have no common way to
//! tell, so there the row never shows.

use serde::Serialize;

use crate::preferences::Preferences;

/// What the Notifications tab shows.
#[allow(clippy::struct_excessive_bools, reason = "one per switch of the tab")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    pub on_done: bool,
    /// Failed, rejected or paused: a paused job waits for the user.
    pub on_problems: bool,
    pub plays_sound: bool,
    /// The sound belongs to a notification, so it needs one of them on.
    pub can_play_sound: bool,
}

pub fn notification_settings(preferences: &Preferences) -> NotificationSettings {
    let wanted = preferences.notification_preferences();
    NotificationSettings {
        on_done: wanted.on_done,
        on_problems: wanted.on_problems,
        plays_sound: wanted.plays_sound,
        can_play_sound: wanted.on_done || wanted.on_problems,
    }
}

/// The row above the switches while the system blocks the app's notifications (`isBlockedBySystem`): the switches do
/// nothing until it lets them through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BlockedBySystem {
    pub label: String,
    pub detail: String,
    /// "Open System Settings"; `None` where the platform has no settings page to open.
    pub open_title: Option<String>,
}

/// The blocked row, when the system blocks notifications; `can_open_settings` where a settings page can be opened.
pub fn blocked_by_system(is_blocked: bool, can_open_settings: bool) -> Option<BlockedBySystem> {
    is_blocked.then(|| BlockedBySystem {
        label: "Turned off in system settings".to_owned(),
        detail: "The system doesn't show Jutsu Studio notifications.".to_owned(),
        open_title: can_open_settings.then(|| "Open System Settings".to_owned()),
    })
}

/// Where Windows keeps its notification switches, under `HKEY_CURRENT_USER`: Settings › System › Notifications for
/// every app, and the app's own switch under its `AppUserModelID`.
pub const WINDOWS_ALL_APPS_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\PushNotifications";
pub const WINDOWS_ALL_APPS_VALUE: &str = "ToastEnabled";
pub const WINDOWS_APP_VALUE: &str = "Enabled";

/// The app's own switch, under the `AppUserModelID` its notifications post with.
pub fn windows_app_key(app_user_model_id: &str) -> String {
    format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\{app_user_model_id}")
}

/// Windows blocks the app's notifications when either switch reads 0, as `reg query` prints them. Only an explicit
/// off counts, as on macOS: a value that was never written means on.
pub fn windows_blocks_notifications(all_apps: &str, this_app: &str) -> bool {
    reg_dword(all_apps, WINDOWS_ALL_APPS_VALUE) == Some(0)
        || reg_dword(this_app, WINDOWS_APP_VALUE) == Some(0)
}

/// The value `name` in `reg query` output: `    Enabled    REG_DWORD    0x0`.
fn reg_dword(output: &str, name: &str) -> Option<u32> {
    output.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        (parts.next()? == name && parts.next()? == "REG_DWORD").then_some(())?;
        u32::from_str_radix(parts.next()?.trim_start_matches("0x"), 16).ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_blocks_them_only_when_a_switch_is_explicitly_off() {
        let all_on = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\PushNotifications\r\n    ToastEnabled    REG_DWORD    0x1\r\n";
        let all_off = all_on.replace("0x1", "0x0");
        let app_off =
            "\r\nHKEY_CURRENT_USER\\...\\com.jutsustudio.desktop\r\n    Enabled    REG_DWORD    0x0\r\n";
        let app_on = app_off.replace("0x0", "0x1");
        // `reg query` fails without the value: nothing printed.
        assert!(!windows_blocks_notifications("", ""));
        assert!(!windows_blocks_notifications(all_on, &app_on));
        assert!(windows_blocks_notifications(&all_off, ""));
        assert!(windows_blocks_notifications(all_on, app_off));
        // Another value of the same key doesn't count.
        assert!(!windows_blocks_notifications(
            "    ShowInActionCenter    REG_DWORD    0x0",
            ""
        ));
        assert_eq!(
            windows_app_key("com.jutsustudio.desktop"),
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\com.jutsustudio.desktop"
        );
    }

    #[test]
    fn the_blocked_row_shows_only_while_the_system_blocks_them() {
        assert_eq!(blocked_by_system(false, true), None);
        let row = blocked_by_system(true, true);
        assert_eq!(
            row.as_ref().map(|row| row.label.as_str()),
            Some("Turned off in system settings")
        );
        assert_eq!(
            row.and_then(|row| row.open_title),
            Some("Open System Settings".to_owned())
        );
        assert_eq!(
            blocked_by_system(true, false).and_then(|row| row.open_title),
            None
        );
    }

    #[test]
    fn everything_is_on_by_default_and_the_sound_needs_a_notification() {
        assert_eq!(
            notification_settings(&Preferences::default()),
            NotificationSettings {
                on_done: true,
                on_problems: true,
                plays_sound: true,
                can_play_sound: true
            }
        );
        let quiet = Preferences {
            notify_on_done: Some(false),
            notify_on_problems: Some(false),
            ..Preferences::default()
        };
        let settings = notification_settings(&quiet);
        assert!(!settings.can_play_sound && settings.plays_sound);
        let problems_only = Preferences {
            notify_on_done: Some(false),
            ..Preferences::default()
        };
        assert!(notification_settings(&problems_only).can_play_sound);
    }
}
