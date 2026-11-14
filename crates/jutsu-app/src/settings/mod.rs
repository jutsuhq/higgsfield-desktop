//! The Settings tabs without their views (`JutsuStudio/Settings/*SettingsView.swift` with the rules of
//! `Presentation`): what each tab shows, computed from the preferences and the disk, and the changes it makes.
//! `src-tauri` adapts them to commands; Connection lives in `connection`, Spending and Agents come later.

mod bytes;
mod general;
mod notifications;
mod output;
mod output_folder;
mod shortcuts;
pub mod spending;
mod storage;

pub use bytes::format_bytes;
pub use general::{
    GeneralSettings, LoginItemStatus, ModeOption, UpdatesState, clear_prompt_history, general_settings,
    shows_tray_icon, version_text,
};
pub use notifications::{
    BlockedBySystem, NotificationSettings, WINDOWS_ALL_APPS_KEY, WINDOWS_ALL_APPS_VALUE, WINDOWS_APP_VALUE,
    blocked_by_system, notification_settings, windows_app_key, windows_blocks_notifications,
};
pub use output::{
    OutputNamePreview, OutputNameTemplate, OutputSettings, display_path, name_footer, output_name_example,
    output_name_preview, output_name_templates, output_settings, token_help,
};
pub use output_folder::{
    OutputFolderError, OutputFolderHistory, current_output_folder, folder_contains, folder_key,
    use_output_folder,
};
pub use shortcuts::{
    Key, KeyStroke, Modifiers, Shortcut, ShortcutAction, ShortcutBinding, ShortcutKeyPress,
    ShortcutRecording, ShortcutRefusal, ShortcutRow, ShortcutSettings, ShortcutStore,
};
pub use storage::{
    Confirmation, DEFAULT_DRAFT_DAYS, DRAFT_DAY_OPTIONS, OldDraft, OldDrafts, StorageError, StorageSummary,
    StorageUsage, cache_size_text, clear_cache, old_drafts, old_drafts_confirmation, old_drafts_summary,
    select_old_drafts, storage_summary, trash_old_drafts,
};
