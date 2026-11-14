//! Settings › Agents (`AgentSettingsView.swift`): Allow agents and whether the app listens, the daily budget of each
//! connection, the unlimited allowance, and how to connect an agent to the bundled `jutsu` helper.

use std::path::{Path, PathBuf};

use jutsu_core::ProviderKind;
use serde::Serialize;

use super::preferences::AgentPreferences;
use crate::preferences::PreferencesError;

/// Whether the app listens for the helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    /// Not started yet.
    Idle,
    Listening(PathBuf),
    /// No place for the socket on this computer: the path would be too long, or there is no runtime folder.
    NoEndpoint,
    Failed(String),
}

impl AgentStatus {
    pub fn text(&self) -> String {
        match self {
            Self::Idle => "Not started".to_owned(),
            Self::Listening(_) => "Ready for the jutsu helper".to_owned(),
            Self::NoEndpoint => "Unavailable: this computer has no folder for the agent socket".to_owned(),
            Self::Failed(reason) => format!("Can't listen for agents: {reason}"),
        }
    }
}

/// One agent the person can connect, and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentClientSetup {
    pub name: String,
    /// What to do with `snippet`.
    pub instructions: String,
    /// A command to run, or the JSON to add to the client's config file.
    pub snippet: String,
    /// Where the config file is on this computer; `None` for a command.
    pub config_file: Option<String>,
}

/// What Settings › Agents shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettingsView {
    pub is_allowed: bool,
    /// "Ready for the jutsu helper", or why not.
    pub status: String,
    pub allows_unlimited: bool,
    /// The API key budget as the field shows it, in dollars: "12.50".
    pub cents_text: String,
    /// The account budget, in credits: "1000".
    pub credits_text: String,
    /// The bundled helper; `None` when this build has none next to the app.
    pub helper_path: Option<String>,
    pub clients: Vec<AgentClientSetup>,
}

/// The tab as it is now.
pub fn view(
    preferences: &AgentPreferences,
    status: &AgentStatus,
    helper: Option<&Path>,
) -> AgentSettingsView {
    let helper_path = helper.map(|path| path.display().to_string());
    AgentSettingsView {
        is_allowed: preferences.is_allowed(),
        status: status.text(),
        allows_unlimited: preferences.allows_unlimited(),
        cents_text: budget_text(preferences, ProviderKind::ApiKey),
        credits_text: budget_text(preferences, ProviderKind::Account),
        clients: helper_path.as_deref().map(clients).unwrap_or_default(),
        helper_path,
    }
}

fn budget_text(preferences: &AgentPreferences, kind: ProviderKind) -> String {
    AgentPreferences::budget_text(preferences.daily_budget(kind), kind)
}

/// The budget field on commit: saves what parses, keeps the last valid budget otherwise, and returns what the field
/// shows next.
pub fn commit_budget(
    preferences: &AgentPreferences,
    kind: ProviderKind,
    text: &str,
) -> Result<String, PreferencesError> {
    if let Some(amount) = AgentPreferences::parse_budget(text, kind) {
        preferences.set_daily_budget(amount, kind)?;
    }
    Ok(budget_text(preferences, kind))
}

/// The one client Settings › Agents sets up, as Swift's does: the command that adds `helper` to Claude Code
/// (`AgentService.claudeCommand`).
pub fn clients(helper: &str) -> Vec<AgentClientSetup> {
    vec![AgentClientSetup {
        name: "Claude Code".to_owned(),
        instructions: "Run this once in a terminal to add Jutsu Studio's tools to Claude Code.".to_owned(),
        snippet: format!("claude mcp add jutsu -- \"{helper}\" mcp"),
        config_file: None,
    }]
}

/// The `jutsu` helper the app ships next to its own executable, when it is there.
pub fn bundled_helper(executable: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "jutsu.exe" } else { "jutsu" };
    let helper = executable.parent()?.join(name);
    helper.is_file().then_some(helper)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::preferences::PreferencesStore;

    fn preferences() -> (tempfile::TempDir, AgentPreferences) {
        let folder = tempfile::tempdir().unwrap();
        let store = Arc::new(PreferencesStore::open(folder.path()));
        (folder, AgentPreferences::new(store))
    }

    #[test]
    fn the_tab_shows_what_is_saved_and_whether_the_app_listens() {
        let (_folder, preferences) = preferences();
        let off = view(&preferences, &AgentStatus::Idle, None);
        assert!(!off.is_allowed && !off.allows_unlimited);
        assert_eq!(off.status, "Not started");
        assert_eq!(
            (off.cents_text.as_str(), off.credits_text.as_str()),
            ("0.00", "0")
        );
        assert!(off.clients.is_empty() && off.helper_path.is_none());

        preferences.set_allowed(true).unwrap();
        preferences.set_allows_unlimited(true).unwrap();
        let on = view(
            &preferences,
            &AgentStatus::Listening(PathBuf::from("/tmp/agent.sock")),
            Some(Path::new("/opt/Jutsu Studio/jutsu")),
        );
        assert!(on.is_allowed && on.allows_unlimited);
        assert_eq!(on.status, "Ready for the jutsu helper");
        // Only Claude Code's command, as Swift's tab offers; a path with spaces stays one argument.
        assert_eq!(on.clients.len(), 1);
        assert_eq!(on.clients[0].name, "Claude Code");
        assert_eq!(
            on.clients[0].snippet,
            "claude mcp add jutsu -- \"/opt/Jutsu Studio/jutsu\" mcp"
        );
        assert!(on.clients[0].config_file.is_none());
        assert_eq!(
            AgentStatus::Failed("in use".to_owned()).text(),
            "Can't listen for agents: in use"
        );
    }

    /// A budget that doesn't parse keeps the last valid one, as the Swift field does.
    #[test]
    fn a_budget_commits_only_what_parses() {
        let (_folder, preferences) = preferences();
        assert_eq!(
            commit_budget(&preferences, ProviderKind::ApiKey, "$12.5").unwrap(),
            "12.50"
        );
        assert_eq!(preferences.daily_budget(ProviderKind::ApiKey), 1250);
        assert_eq!(
            commit_budget(&preferences, ProviderKind::ApiKey, "lots").unwrap(),
            "12.50"
        );
        assert_eq!(
            commit_budget(&preferences, ProviderKind::Account, "1,000 credits").unwrap(),
            "1000"
        );
        assert_eq!(preferences.daily_budget(ProviderKind::Account), 1000);
    }
}
