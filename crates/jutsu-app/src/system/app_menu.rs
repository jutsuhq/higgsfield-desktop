//! The window's menu bar (`AppCommands.swift`, `JutsuStudioApp.swift`): the app menu with About, Check for Updates,
//! Settings and Quit; File › Open…, New from Template and Generate; View with the pages and Toggle Sidebar; and the
//! Library and Queue menus. Their shortcuts are the ones set in Settings › Shortcuts.
//!
//! A page publishes the commands that apply on it (`@FocusedValue` on macOS) as `PageCommandState`s: a command that
//! no page publishes is disabled and keeps its placeholder title. Choosing one of them runs the handler the page's
//! keyboard shortcut runs.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::{Key, Modifiers, Shortcut, ShortcutAction};

/// A command a page carries out: its keyboard shortcut and its menu item run the same handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PageCommand {
    Create,
    Queue,
    Library,
    Settings,
    Open,
    Generate,
    Star,
    RenderFinal,
    DeleteResult,
    CancelJob,
    RetryJob,
    ToggleSidebar,
}

impl PageCommand {
    pub const ALL: [Self; 12] = [
        Self::Create,
        Self::Queue,
        Self::Library,
        Self::Settings,
        Self::Open,
        Self::Generate,
        Self::Star,
        Self::RenderFinal,
        Self::DeleteResult,
        Self::CancelJob,
        Self::RetryJob,
        Self::ToggleSidebar,
    ];

    /// The shortcut Settings › Shortcuts sets for it; `None` for the fixed ones.
    pub fn shortcut_action(self) -> Option<ShortcutAction> {
        match self {
            Self::Create => Some(ShortcutAction::Create),
            Self::Queue => Some(ShortcutAction::Queue),
            Self::Library => Some(ShortcutAction::Library),
            Self::Open => Some(ShortcutAction::Open),
            Self::Generate => Some(ShortcutAction::Generate),
            Self::Star => Some(ShortcutAction::Star),
            Self::DeleteResult => Some(ShortcutAction::DeleteResult),
            Self::CancelJob => Some(ShortcutAction::CancelJob),
            Self::RetryJob => Some(ShortcutAction::RetryJob),
            Self::Settings | Self::RenderFinal | Self::ToggleSidebar => None,
        }
    }

    /// The fixed shortcuts: Ctrl+, opens Settings (⌘, on macOS), Ctrl+Shift+S toggles the sidebar (⌃⌘S there).
    pub fn fixed_shortcut(self) -> Option<Shortcut> {
        match self {
            Self::Settings => Some(Shortcut::character(',', Modifiers::CTRL)),
            Self::ToggleSidebar => Some(Shortcut::character('s', Modifiers::CTRL_SHIFT)),
            _ => None,
        }
    }

    /// The item's title while no page publishes one of its own.
    pub fn title(self) -> &'static str {
        match self {
            Self::Create => "Create",
            Self::Queue => "Queue",
            Self::Library => "Library",
            Self::Settings => "Settings…",
            Self::Open => "Open…",
            Self::Generate => "Generate",
            Self::Star => "Star",
            Self::RenderFinal => "Render Final…",
            Self::DeleteResult => "Delete from Library…",
            Self::CancelJob => "Cancel Job",
            Self::RetryJob => "Retry Job",
            Self::ToggleSidebar => "Toggle Sidebar",
        }
    }
}

/// A command that applies on the page on screen, with the title the page gives it (Star or Unstar, Retry Download).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PageCommandState {
    pub command: PageCommand,
    /// `None` keeps the item's own title.
    pub title: Option<String>,
}

/// A template New from Template lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuTemplate {
    pub id: Uuid,
    pub name: String,
}

/// What the menu is made from.
#[derive(Debug, Clone, Copy)]
pub struct AppMenuInput<'a> {
    pub page_commands: &'a [PageCommandState],
    /// Settings › Shortcuts; a removed shortcut is missing.
    pub shortcuts: &'a HashMap<ShortcutAction, Shortcut>,
    /// As the Queue header's buttons are enabled.
    pub can_pause_all: bool,
    pub can_resume_all: bool,
    /// Dimmed in builds without an updater and while a check runs.
    pub can_check_for_updates: bool,
    /// The templates of the side Create shows, as Create's Templates menu sorts them.
    pub templates: &'a [MenuTemplate],
}

/// What a menu item does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMenuAction {
    About,
    CheckForUpdates,
    Quit,
    PauseAll,
    ResumeAll,
    /// Runs on the page that published it.
    Page(PageCommand),
    /// New from Template: applies the template in Create.
    Template(Uuid),
}

impl AppMenuAction {
    const PREFIX: &'static str = "app.";

    /// The menu item's ID.
    pub fn id(&self) -> String {
        let name = match self {
            Self::About => "about".to_owned(),
            Self::CheckForUpdates => "checkForUpdates".to_owned(),
            Self::Quit => "quit".to_owned(),
            Self::PauseAll => "pauseAll".to_owned(),
            Self::ResumeAll => "resumeAll".to_owned(),
            Self::Page(command) => format!("page.{}", page_name(*command)),
            Self::Template(id) => format!("template.{id}"),
        };
        format!("{}{name}", Self::PREFIX)
    }

    /// The action of the item `id`; `None` for an ID this menu didn't make.
    pub fn from_id(id: &str) -> Option<Self> {
        let name = id.strip_prefix(Self::PREFIX)?;
        if let Some(page) = name.strip_prefix("page.") {
            return PageCommand::ALL
                .into_iter()
                .find(|command| page_name(*command) == page)
                .map(Self::Page);
        }
        if let Some(template) = name.strip_prefix("template.") {
            return Uuid::parse_str(template).ok().map(Self::Template);
        }
        match name {
            "about" => Some(Self::About),
            "checkForUpdates" => Some(Self::CheckForUpdates),
            "quit" => Some(Self::Quit),
            "pauseAll" => Some(Self::PauseAll),
            "resumeAll" => Some(Self::ResumeAll),
            _ => None,
        }
    }
}

fn page_name(command: PageCommand) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// One entry of a menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMenuItem {
    Item {
        action: AppMenuAction,
        title: String,
        is_enabled: bool,
        /// `Ctrl+Shift+R`; `None` without a shortcut.
        accelerator: Option<String>,
    },
    /// A line that can't be chosen, such as "No templates yet".
    Label(String),
    Separator,
    Submenu {
        title: String,
        items: Vec<AppMenuItem>,
    },
}

/// A menu of the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMenuSection {
    pub title: String,
    pub items: Vec<AppMenuItem>,
}

/// Ctrl+Q, as ⌘Q quits on macOS.
const QUIT_SHORTCUT: Shortcut = Shortcut {
    key: Key::Character('q'),
    modifiers: Modifiers::CTRL,
};

/// The menu bar as things stand.
pub fn app_menu(input: &AppMenuInput) -> Vec<AppMenuSection> {
    let published: HashMap<PageCommand, Option<&str>> = input
        .page_commands
        .iter()
        .map(|state| (state.command, state.title.as_deref()))
        .collect();
    let page = |command: PageCommand| {
        let shortcut = match command.shortcut_action() {
            Some(action) => input.shortcuts.get(&action).copied(),
            None => command.fixed_shortcut(),
        };
        let title = published
            .get(&command)
            .copied()
            .flatten()
            .unwrap_or(command.title());
        AppMenuItem::Item {
            action: AppMenuAction::Page(command),
            title: title.to_owned(),
            is_enabled: published.contains_key(&command),
            accelerator: shortcut.as_ref().and_then(Shortcut::accelerator),
        }
    };
    let item = |action: AppMenuAction, title: &str, is_enabled: bool| AppMenuItem::Item {
        action,
        title: title.to_owned(),
        is_enabled,
        accelerator: None,
    };

    let mut templates: Vec<AppMenuItem> = input
        .templates
        .iter()
        .map(|template| item(AppMenuAction::Template(template.id), &template.name, true))
        .collect();
    if templates.is_empty() {
        templates.push(AppMenuItem::Label("No templates yet".to_owned()));
    }

    vec![
        AppMenuSection {
            title: "Jutsu Studio".to_owned(),
            items: vec![
                item(AppMenuAction::About, "About Jutsu Studio", true),
                item(
                    AppMenuAction::CheckForUpdates,
                    "Check for Updates…",
                    input.can_check_for_updates,
                ),
                AppMenuItem::Separator,
                page(PageCommand::Settings),
                AppMenuItem::Separator,
                AppMenuItem::Item {
                    action: AppMenuAction::Quit,
                    title: "Quit Jutsu Studio".to_owned(),
                    is_enabled: true,
                    accelerator: QUIT_SHORTCUT.accelerator(),
                },
            ],
        },
        AppMenuSection {
            title: "File".to_owned(),
            items: vec![
                page(PageCommand::Open),
                AppMenuItem::Submenu {
                    title: "New from Template".to_owned(),
                    items: templates,
                },
                AppMenuItem::Separator,
                page(PageCommand::Generate),
            ],
        },
        AppMenuSection {
            title: "View".to_owned(),
            items: vec![
                page(PageCommand::Create),
                page(PageCommand::Queue),
                page(PageCommand::Library),
                AppMenuItem::Separator,
                page(PageCommand::ToggleSidebar),
            ],
        },
        AppMenuSection {
            title: "Library".to_owned(),
            items: vec![
                page(PageCommand::Star),
                page(PageCommand::RenderFinal),
                AppMenuItem::Separator,
                page(PageCommand::DeleteResult),
            ],
        },
        AppMenuSection {
            title: "Queue".to_owned(),
            items: vec![
                item(AppMenuAction::PauseAll, "Pause All", input.can_pause_all),
                item(AppMenuAction::ResumeAll, "Resume All", input.can_resume_all),
                AppMenuItem::Separator,
                page(PageCommand::CancelJob),
                page(PageCommand::RetryJob),
            ],
        },
    ]
}

/// The same menus with the same items in the same places: only titles, enabled states and shortcuts differ, so the
/// shown menu can be changed in place instead of made again.
pub fn same_layout(a: &[AppMenuSection], b: &[AppMenuSection]) -> bool {
    fn items_match(a: &[AppMenuItem], b: &[AppMenuItem]) -> bool {
        a.len() == b.len()
            && a.iter().zip(b).all(|pair| match pair {
                (AppMenuItem::Item { action: x, .. }, AppMenuItem::Item { action: y, .. }) => x == y,
                (AppMenuItem::Label(x), AppMenuItem::Label(y)) => x == y,
                (AppMenuItem::Separator, AppMenuItem::Separator) => true,
                (
                    AppMenuItem::Submenu { title: x, items: xs },
                    AppMenuItem::Submenu { title: y, items: ys },
                ) => x == y && items_match(xs, ys),
                _ => false,
            })
    }
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.title == y.title && items_match(&x.items, &y.items))
}

#[cfg(test)]
#[path = "app_menu_tests.rs"]
mod tests;
