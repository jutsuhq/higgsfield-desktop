//! The app's settings, as the macOS app keeps them in `UserDefaults` under `AppSettings`' keys: one JSON file in the
//! app data dir, written atomically, with a change broadcast. Every reader goes through the methods of
//! [`Preferences`], so a missing or out-of-range value means the default (`AppSettings.swift`).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use jutsu_core::{Mode, OutputNamePattern, ProviderKind, Resolution, SpendCap};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;

use crate::connection::{ConnectionMetadata, ConnectionMetadataStore};

/// Every `@AppStorage` key of `AppSettings`, under the same name; `None` is a key that was never set. The
/// values a later version may not understand (a resolution, a mode) stay strings, so an unknown one falls back
/// to the default instead of failing the whole file.
///
/// It doubles as a patch for `PreferencesStore::update`: every field that is `Some` is written.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Preferences {
    #[serde(
        rename = "settings.defaultResolution",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub default_resolution: Option<String>,
    #[serde(
        rename = "settings.maxConcurrency",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_concurrency: Option<i32>,
    #[serde(
        rename = "settings.spendCapEnabled",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub spend_cap_enabled: Option<bool>,
    #[serde(
        rename = "settings.spendCapUSD",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub spend_cap_usd: Option<f64>,
    /// The key predates the split by kind, so it now holds the "done" switch.
    #[serde(
        rename = "settings.notifyOnFinish",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub notify_on_done: Option<bool>,
    /// Failed, rejected and paused.
    #[serde(
        rename = "settings.notifyOnProblems",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub notify_on_problems: Option<bool>,
    #[serde(
        rename = "settings.notificationSound",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub notification_sound: Option<bool>,
    #[serde(
        rename = "settings.spendCapCreditsEnabled",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub spend_cap_credits_enabled: Option<bool>,
    #[serde(
        rename = "settings.spendCapCredits",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub spend_cap_credits: Option<i32>,
    #[serde(
        rename = "settings.lowCreditThreshold",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub low_credit_threshold: Option<i32>,
    /// The account pays from the plan's unlimited allowance where `models_explore` says it applies.
    #[serde(
        rename = "settings.useUnlimited",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub use_unlimited: Option<bool>,
    #[serde(
        rename = "settings.draftResolution",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub draft_resolution: Option<String>,
    #[serde(
        rename = "settings.defaultMode",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub default_mode: Option<String>,
    #[serde(
        rename = "settings.showInMenuBar",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub show_in_menu_bar: Option<bool>,
    #[serde(
        rename = "settings.showActiveCount",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub show_active_count: Option<bool>,
    /// Run in the tray (`MenuBarMode`).
    #[serde(
        rename = "settings.runInMenuBar",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub run_in_menu_bar: Option<bool>,
    /// A custom output folder; absent means the default folder. The macOS app keeps a security-scoped bookmark
    /// under this key; here it is the folder's plain path.
    #[serde(
        rename = "settings.outputFolderBookmark",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_folder: Option<String>,
    /// Custom folders picked before the current one that may still hold results (`OutputFolderHistory`).
    #[serde(
        rename = "settings.previousOutputFolders",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub previous_output_folders: Option<Vec<String>>,
    #[serde(
        rename = "settings.outputNamePattern",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_name_pattern: Option<String>,
    /// Seconds since 1970 of the last "Clear prompt history"; 0 means never cleared.
    #[serde(
        rename = "settings.promptHistoryClearedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub prompt_history_cleared_at: Option<f64>,
    /// `[StylePreset]` snapshots, newest first; see `PresetShelf`.
    #[serde(
        rename = "presets.favorites",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[specta(type = Option<specta_typescript::Unknown>)]
    pub favorite_presets: Option<Value>,
    #[serde(rename = "presets.recent", default, skip_serializing_if = "Option::is_none")]
    #[specta(type = Option<specta_typescript::Unknown>)]
    pub recent_presets: Option<Value>,
    /// Seconds since 1970 of the Queue's last "Clear all" of jobs that didn't finish; 0 means never.
    #[serde(
        rename = "queue.problemsClearedAt",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub queue_problems_cleared_at: Option<f64>,
    /// Comma-separated ids of the jobs removed from the Queue's "Didn't finish" section; see `QueueProblems`.
    #[serde(
        rename = "queue.removedProblems",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub queue_removed_problems: Option<String>,
    /// Export for Editing writes the JSON sidecar; on unless turned off.
    #[serde(
        rename = "export.writesSidecar",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub writes_sidecar: Option<bool>,
    /// Export for Editing writes the version's subtitles as SRT; on unless turned off.
    #[serde(
        rename = "export.writesSubtitles",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub writes_subtitles: Option<bool>,
}

/// Which job notifications the user wants. Paused counts as a problem: the job waits for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotificationPreferences {
    pub on_done: bool,
    pub on_problems: bool,
    pub plays_sound: bool,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            on_done: true,
            on_problems: true,
            plays_sound: true,
        }
    }
}

impl Preferences {
    pub const DEFAULT_RESOLUTION: Resolution = Resolution::P480;
    /// Parallel jobs for an API key, whose Higgsfield limit is low.
    pub const DEFAULT_MAX_CONCURRENCY: i64 = 2;
    /// Parallel jobs for an account. Higgsfield names no limit for a plan; a refusal lowers it until the next launch.
    pub const DEFAULT_ACCOUNT_MAX_CONCURRENCY: i64 = 8;
    pub const MAX_CONCURRENCY_RANGE: (i64, i64) = (1, 40);
    pub const DEFAULT_SPEND_CAP_ENABLED: bool = true;
    pub const DEFAULT_SPEND_CAP_USD: f64 = 75.0;
    /// Off: an account had no cap before, its plan limits it.
    pub const DEFAULT_SPEND_CAP_CREDITS_ENABLED: bool = false;
    pub const DEFAULT_SPEND_CAP_CREDITS: i64 = 1_000;
    pub const DEFAULT_LOW_CREDIT_THRESHOLD: i64 = 50;
    /// Off: unlimited generations are spent only after the user chose to.
    pub const DEFAULT_USE_UNLIMITED: bool = false;
    pub const DEFAULT_NOTIFY: bool = true;
    pub const DEFAULT_NOTIFICATION_SOUND: bool = true;
    pub const DEFAULT_DRAFT_RESOLUTION: Resolution = Resolution::P480;
    pub const DEFAULT_MODE: Mode = Mode::MotionTransfer;
    pub const DEFAULT_SHOW_IN_MENU_BAR: bool = true;
    pub const DEFAULT_SHOW_ACTIVE_COUNT: bool = false;
    /// Off: the person decides whether the app lives in the tray.
    pub const DEFAULT_RUN_IN_MENU_BAR: bool = false;

    /// The key of the connection's image models as last loaded (`AppSettings.imageModelsKey`).
    pub fn image_models_key(kind: ProviderKind, owner: Option<&str>) -> String {
        format!("images.models.{}.{}", kind.raw_value(), owner.unwrap_or_default())
    }

    pub fn clamp_concurrency(value: i64) -> i64 {
        value.clamp(Self::MAX_CONCURRENCY_RANGE.0, Self::MAX_CONCURRENCY_RANGE.1)
    }

    /// Brings values stored by earlier versions to the current keys. Run once at launch, before anything reads
    /// them. Whether it changed anything.
    pub fn migrate(&mut self) -> bool {
        // One switch used to cover every kind; whoever turned it off keeps problems quiet too.
        if self.notify_on_problems.is_none()
            && let Some(notify) = self.notify_on_done
        {
            self.notify_on_problems = Some(notify);
            return true;
        }
        false
    }

    /// One of `Resolution::STANDARD`, the ones Settings offers; any other stored value gives the default.
    pub fn default_resolution(&self) -> Resolution {
        settings_resolution(self.default_resolution.as_deref()).unwrap_or(Self::DEFAULT_RESOLUTION)
    }

    pub fn draft_resolution(&self) -> Resolution {
        settings_resolution(self.draft_resolution.as_deref()).unwrap_or(Self::DEFAULT_DRAFT_RESOLUTION)
    }

    pub fn default_mode(&self) -> Mode {
        self.default_mode
            .as_deref()
            .and_then(Mode::from_raw)
            .unwrap_or(Self::DEFAULT_MODE)
    }

    /// What runs at once when nothing is set in Settings: more for an account than for an API key.
    pub fn default_max_concurrency(connection: Option<ProviderKind>) -> i64 {
        if connection == Some(ProviderKind::Account) {
            Self::DEFAULT_ACCOUNT_MAX_CONCURRENCY
        } else {
            Self::DEFAULT_MAX_CONCURRENCY
        }
    }

    /// The limit set in Settings, else the connection's default.
    pub fn max_concurrency(&self, connection: Option<ProviderKind>) -> i64 {
        self.max_concurrency.map_or_else(
            || Self::default_max_concurrency(connection),
            |value| Self::clamp_concurrency(i64::from(value)),
        )
    }

    /// The daily cap for `Budget` on `connection`: USD for an API key, credits for an account. `None` when that
    /// cap is off or nothing is connected.
    pub fn spend_cap(&self, connection: Option<ProviderKind>) -> Option<SpendCap> {
        match connection? {
            ProviderKind::ApiKey => {
                if !self.spend_cap_enabled.unwrap_or(Self::DEFAULT_SPEND_CAP_ENABLED) {
                    return None;
                }
                let amount = self
                    .spend_cap_usd
                    .filter(|amount| *amount > 0.0 && amount.is_finite())
                    .unwrap_or(Self::DEFAULT_SPEND_CAP_USD);
                Some(SpendCap::usd(cents(amount)))
            }
            ProviderKind::Account => {
                if !self
                    .spend_cap_credits_enabled
                    .unwrap_or(Self::DEFAULT_SPEND_CAP_CREDITS_ENABLED)
                {
                    return None;
                }
                let amount = self
                    .spend_cap_credits
                    .map(i64::from)
                    .filter(|amount| *amount > 0)
                    .unwrap_or(Self::DEFAULT_SPEND_CAP_CREDITS);
                Some(SpendCap::credits(amount))
            }
        }
    }

    pub fn runs_in_menu_bar(&self) -> bool {
        self.run_in_menu_bar.unwrap_or(Self::DEFAULT_RUN_IN_MENU_BAR)
    }

    pub fn uses_unlimited(&self) -> bool {
        self.use_unlimited.unwrap_or(Self::DEFAULT_USE_UNLIMITED)
    }

    pub fn low_credit_threshold(&self) -> i64 {
        self.low_credit_threshold
            .map_or(Self::DEFAULT_LOW_CREDIT_THRESHOLD, i64::from)
    }

    pub fn notification_preferences(&self) -> NotificationPreferences {
        NotificationPreferences {
            on_done: self.notify_on_done.unwrap_or(Self::DEFAULT_NOTIFY),
            on_problems: self.notify_on_problems.unwrap_or(Self::DEFAULT_NOTIFY),
            plays_sound: self
                .notification_sound
                .unwrap_or(Self::DEFAULT_NOTIFICATION_SOUND),
        }
    }

    pub fn output_name_pattern(&self) -> OutputNamePattern {
        OutputNamePattern::new(
            self.output_name_pattern
                .as_deref()
                .unwrap_or(OutputNamePattern::DEFAULT_PATTERN),
        )
    }

    /// Writes every field of `patch` that is set.
    pub fn apply(&mut self, patch: Preferences) {
        macro_rules! take {
            ($($field:ident),+) => { $( if patch.$field.is_some() { self.$field = patch.$field; } )+ };
        }
        take!(
            default_resolution,
            max_concurrency,
            spend_cap_enabled,
            spend_cap_usd,
            notify_on_done,
            notify_on_problems,
            notification_sound,
            spend_cap_credits_enabled,
            spend_cap_credits,
            low_credit_threshold,
            use_unlimited,
            draft_resolution,
            default_mode,
            show_in_menu_bar,
            show_active_count,
            run_in_menu_bar,
            output_folder,
            previous_output_folders,
            output_name_pattern,
            prompt_history_cleared_at,
            favorite_presets,
            recent_presets,
            queue_problems_cleared_at,
            queue_removed_problems,
            writes_sidecar,
            writes_subtitles
        );
    }
}

fn settings_resolution(stored: Option<&str>) -> Option<Resolution> {
    stored
        .and_then(Resolution::from_raw)
        .filter(|resolution| Resolution::STANDARD.contains(resolution))
}

/// Dollars rounded to cents, through `Decimal` as Swift does: a stored amount from before the upper bound could be
/// too big for an integer.
fn cents(amount: f64) -> Decimal {
    Decimal::from_f64((amount * 100.0).round()).map_or(Decimal::MAX, |cents| cents / Decimal::from(100))
}

/// What the file holds: the settings, the connection's non-secret metadata (`UserDefaultsMetadataStore`, key
/// `connection.metadata`), and any other key, such as the per-connection image models, kept as it is.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PreferencesFile {
    #[serde(flatten)]
    preferences: Preferences,
    #[serde(
        rename = "connection.metadata",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    connection: Option<ConnectionMetadata>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

/// Why the preferences file couldn't be written.
#[derive(Debug, thiserror::Error)]
pub enum PreferencesError {
    #[error("Can't save the settings: {0}")]
    Io(#[from] std::io::Error),
    #[error("Can't save the settings: {0}")]
    Encoding(#[from] serde_json::Error),
}

/// `preferences.json` in the app data dir. Reads come from memory; every write replaces the file atomically (a
/// temporary file renamed over it) and bumps the revision `subscribe` hands out.
///
/// note: writes are synchronous; the file is a few kilobytes at most. A larger store would move them to
/// `spawn_blocking`.
#[derive(Debug)]
pub struct PreferencesStore {
    path: PathBuf,
    file: Mutex<PreferencesFile>,
    changes: watch::Sender<u64>,
}

impl PreferencesStore {
    pub const FILE_NAME: &'static str = "preferences.json";

    /// Loads `preferences.json` from `folder` and migrates it. A file that is missing or can't be read gives the
    /// defaults; one that can't be read is replaced on the next write.
    pub fn open(folder: &Path) -> Self {
        let path = folder.join(Self::FILE_NAME);
        let mut file = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<PreferencesFile>(&bytes).unwrap_or_else(|error| {
                tracing::error!(%error, "can't read the settings; using the defaults");
                PreferencesFile::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => PreferencesFile::default(),
            Err(error) => {
                tracing::error!(%error, "can't open the settings; using the defaults");
                PreferencesFile::default()
            }
        };
        let migrated = file.preferences.migrate();
        let store = Self {
            path,
            file: Mutex::new(file),
            changes: watch::channel(0).0,
        };
        if migrated && let Err(error) = store.save() {
            tracing::error!(%error, "can't save the migrated settings");
        }
        store
    }

    /// The settings as they are now.
    pub fn get(&self) -> Preferences {
        self.locked().preferences.clone()
    }

    /// Writes every field of `patch` that is set, saves the file and announces the change.
    pub fn update(&self, patch: Preferences) -> Result<Preferences, PreferencesError> {
        let preferences = {
            let mut file = self.locked();
            file.preferences.apply(patch);
            file.preferences.clone()
        };
        self.save()?;
        Ok(preferences)
    }

    /// Changes the settings with `edit`, which may also clear a value (`update` only sets them), saves the file and
    /// announces the change.
    pub fn edit(&self, edit: impl FnOnce(&mut Preferences)) -> Result<Preferences, PreferencesError> {
        let preferences = {
            let mut file = self.locked();
            edit(&mut file.preferences);
            file.preferences.clone()
        };
        self.save()?;
        Ok(preferences)
    }

    /// A value under a key `Preferences` doesn't model, such as `Preferences::image_models_key`.
    pub fn value(&self, key: &str) -> Option<Value> {
        self.locked().other.get(key).cloned()
    }

    pub fn set_value(&self, key: &str, value: Option<Value>) -> Result<(), PreferencesError> {
        {
            let mut file = self.locked();
            match value {
                Some(value) => file.other.insert(key.to_owned(), value),
                None => file.other.remove(key),
            };
        }
        self.save()
    }

    /// A revision that goes up after every write.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, PreferencesFile> {
        self.file.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn save(&self) -> Result<(), PreferencesError> {
        let bytes = serde_json::to_vec_pretty(&*self.locked())?;
        write_atomically(&self.path, &bytes)?;
        self.changes.send_modify(|revision| *revision += 1);
        Ok(())
    }
}

impl ConnectionMetadataStore for PreferencesStore {
    fn load(&self) -> Option<ConnectionMetadata> {
        self.locked().connection.clone()
    }

    fn save(&self, metadata: &ConnectionMetadata) {
        self.locked().connection = Some(metadata.clone());
        if let Err(error) = PreferencesStore::save(self) {
            tracing::error!(%error, "can't save the connection's metadata");
        }
    }

    fn clear(&self) {
        self.locked().connection = None;
        if let Err(error) = PreferencesStore::save(self) {
            tracing::error!(%error, "can't clear the connection's metadata");
        }
    }
}

/// Writes `bytes` next to `path` and renames the copy over it, so a crash leaves the old file or the new one.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stored(json: Value) -> Preferences {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn empty_defaults_give_the_documented_values() {
        let defaults = Preferences::default();
        assert_eq!(defaults.default_resolution(), Resolution::P480);
        assert_eq!(defaults.max_concurrency(Some(ProviderKind::ApiKey)), 2);
        assert_eq!(defaults.max_concurrency(Some(ProviderKind::Account)), 8);
        assert_eq!(defaults.max_concurrency(None), 2);
        assert_eq!(
            defaults.spend_cap(Some(ProviderKind::ApiKey)),
            Some(SpendCap::usd(Decimal::from(75)))
        );
        assert_eq!(defaults.spend_cap(Some(ProviderKind::Account)), None);
        assert_eq!(defaults.spend_cap(None), None);
        assert_eq!(
            defaults.notification_preferences(),
            NotificationPreferences::default()
        );
        assert_eq!(defaults.output_folder, None);
        assert_eq!(defaults.draft_resolution(), Resolution::P480);
        assert_eq!(defaults.default_mode(), Mode::MotionTransfer);
        assert_eq!(defaults.output_name_pattern(), OutputNamePattern::standard());
        assert!(!defaults.runs_in_menu_bar());
        assert!(!defaults.uses_unlimited());
        assert_eq!(defaults.low_credit_threshold(), 50);
    }

    #[test]
    fn output_name_pattern_falls_back_when_invalid() {
        let mut preferences = stored(serde_json::json!({ "settings.outputNamePattern": "{res}_{id}" }));
        assert_eq!(preferences.output_name_pattern().pattern(), "{res}_{id}");
        preferences.output_name_pattern = Some("{resolution}".to_owned());
        assert_eq!(preferences.output_name_pattern(), OutputNamePattern::standard());
    }

    #[test]
    fn stored_values_are_read() {
        let preferences = stored(serde_json::json!({
            "settings.defaultResolution": "1080p",
            "settings.spendCapUSD": 42.5,
            "settings.notifyOnFinish": false,
            "settings.notificationSound": false,
            "settings.outputFolderBookmark": "/tmp/Renders",
            "settings.draftResolution": "720p",
            "settings.defaultMode": "restyle",
        }));
        assert_eq!(preferences.default_resolution(), Resolution::P1080);
        assert_eq!(
            preferences.spend_cap(Some(ProviderKind::ApiKey)),
            Some(SpendCap::usd(Decimal::new(425, 1)))
        );
        assert_eq!(
            preferences.notification_preferences(),
            NotificationPreferences {
                on_done: false,
                on_problems: true,
                plays_sound: false
            }
        );
        assert_eq!(preferences.output_folder.as_deref(), Some("/tmp/Renders"));
        assert_eq!(preferences.draft_resolution(), Resolution::P720);
        assert_eq!(preferences.default_mode(), Mode::Restyle);
    }

    #[test]
    fn unknown_values_fall_back_to_the_defaults() {
        let preferences = stored(serde_json::json!({
            "settings.defaultResolution": "4k",
            "settings.draftResolution": "4k",
            "settings.defaultMode": "dance",
        }));
        assert_eq!(preferences.default_resolution(), Resolution::P480);
        assert_eq!(preferences.draft_resolution(), Resolution::P480);
        assert_eq!(preferences.default_mode(), Mode::MotionTransfer);
    }

    #[test]
    fn disabled_cap_is_none() {
        let mut preferences = stored(serde_json::json!({ "settings.spendCapEnabled": false }));
        assert_eq!(preferences.spend_cap(Some(ProviderKind::ApiKey)), None);
        preferences.spend_cap_enabled = Some(true);
        preferences.spend_cap_usd = Some(-3.0);
        assert_eq!(
            preferences.spend_cap(Some(ProviderKind::ApiKey)),
            Some(SpendCap::usd(Decimal::from(75)))
        );
    }

    #[test]
    fn account_cap_is_in_credits() {
        let mut preferences = stored(serde_json::json!({ "settings.spendCapCreditsEnabled": true }));
        let account = Some(ProviderKind::Account);
        assert_eq!(preferences.spend_cap(account), Some(SpendCap::credits(1_000)));
        preferences.spend_cap_credits = Some(250);
        assert_eq!(preferences.spend_cap(account), Some(SpendCap::credits(250)));
        preferences.spend_cap_credits = Some(0);
        assert_eq!(preferences.spend_cap(account), Some(SpendCap::credits(1_000)));
        preferences.spend_cap_enabled = Some(false);
        assert_eq!(preferences.spend_cap(account), Some(SpendCap::credits(1_000)));
    }

    #[test]
    fn turning_off_the_old_notify_switch_keeps_problems_quiet() {
        let mut preferences = stored(serde_json::json!({ "settings.notifyOnFinish": false }));
        assert!(preferences.migrate());
        assert_eq!(
            preferences.notification_preferences(),
            NotificationPreferences {
                on_done: false,
                on_problems: false,
                plays_sound: true
            }
        );
        preferences.notify_on_problems = Some(true);
        assert!(!preferences.migrate());
        assert!(preferences.notification_preferences().on_problems);
    }

    #[test]
    fn migrating_fresh_defaults_writes_nothing() {
        let mut preferences = Preferences::default();
        assert!(!preferences.migrate());
        assert_eq!(preferences.notify_on_problems, None);
    }

    #[test]
    fn concurrency_is_clamped() {
        for (value, expected) in [(-5, 1), (0, 1), (1, 1), (2, 2), (40, 40), (41, 40), (1_000, 40)] {
            let preferences = Preferences {
                max_concurrency: Some(value),
                ..Preferences::default()
            };
            assert_eq!(preferences.max_concurrency(Some(ProviderKind::Account)), expected);
            assert_eq!(preferences.max_concurrency(Some(ProviderKind::ApiKey)), expected);
        }
    }

    /// A cap stored before the upper bound is read without trapping.
    #[test]
    fn a_huge_stored_cap_is_read() {
        let preferences = stored(serde_json::json!({ "settings.spendCapUSD": 1e30 }));
        assert!(preferences.spend_cap(Some(ProviderKind::ApiKey)).is_some());
    }

    #[test]
    fn image_models_keys_name_the_connection() {
        assert_eq!(
            Preferences::image_models_key(ProviderKind::Account, Some("sub")),
            "images.models.account.sub"
        );
        assert_eq!(
            Preferences::image_models_key(ProviderKind::ApiKey, None),
            "images.models.apiKey."
        );
    }

    #[test]
    fn the_store_writes_the_file_atomically_and_announces_changes() {
        let folder = tempfile::tempdir().unwrap();
        let store = PreferencesStore::open(folder.path());
        let mut changes = store.subscribe();
        assert!(!changes.has_changed().unwrap());

        let saved = store
            .update(Preferences {
                use_unlimited: Some(true),
                ..Preferences::default()
            })
            .unwrap();
        assert!(saved.uses_unlimited());
        assert!(changes.has_changed().unwrap());
        changes.mark_unchanged();

        store
            .update(Preferences {
                low_credit_threshold: Some(10),
                ..Preferences::default()
            })
            .unwrap();
        assert!(changes.has_changed().unwrap());

        let text = std::fs::read_to_string(folder.path().join(PreferencesStore::FILE_NAME)).unwrap();
        let json: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["settings.useUnlimited"], Value::Bool(true));
        assert_eq!(json["settings.lowCreditThreshold"], Value::from(10));
        assert!(!folder.path().join("preferences.json.tmp").exists());

        let reopened = PreferencesStore::open(folder.path());
        assert_eq!(reopened.get().low_credit_threshold(), 10);
        assert!(reopened.get().uses_unlimited());
    }

    #[test]
    fn metadata_round_trips_and_clears_and_other_keys_are_kept() {
        let folder = tempfile::tempdir().unwrap();
        let store = PreferencesStore::open(folder.path());
        let metadata = ConnectionMetadata {
            kind: ProviderKind::Account,
            display_name: Some("me@example.com".into()),
            plan: Some("Pro".into()),
            name: None,
        };
        ConnectionMetadataStore::save(&store, &metadata);
        let key = Preferences::image_models_key(ProviderKind::Account, Some("me"));
        store.set_value(&key, Some(serde_json::json!(["nano"]))).unwrap();

        let reopened = PreferencesStore::open(folder.path());
        assert_eq!(reopened.load(), Some(metadata));
        assert_eq!(reopened.value(&key), Some(serde_json::json!(["nano"])));
        let text = std::fs::read_to_string(folder.path().join(PreferencesStore::FILE_NAME)).unwrap();
        assert!(text.contains("\"connection.metadata\""));
        assert!(text.contains("\"displayName\": \"me@example.com\""));

        reopened.clear();
        assert_eq!(PreferencesStore::open(folder.path()).load(), None);
    }

    #[test]
    fn an_unreadable_file_gives_the_defaults() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join(PreferencesStore::FILE_NAME), "{ not json").unwrap();
        assert_eq!(
            PreferencesStore::open(folder.path()).get(),
            Preferences::default()
        );
    }

    #[test]
    fn opening_migrates_and_saves() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(
            folder.path().join(PreferencesStore::FILE_NAME),
            r#"{"settings.notifyOnFinish": false}"#,
        )
        .unwrap();
        PreferencesStore::open(folder.path());
        let text = std::fs::read_to_string(folder.path().join(PreferencesStore::FILE_NAME)).unwrap();
        assert!(text.contains("\"settings.notifyOnProblems\": false"));
    }
}
