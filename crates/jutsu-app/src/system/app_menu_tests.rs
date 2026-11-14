use std::collections::HashMap;

use uuid::Uuid;

use super::*;

fn defaults() -> HashMap<ShortcutAction, Shortcut> {
    ShortcutAction::ALL
        .into_iter()
        .map(|action| (action, action.default_shortcut()))
        .collect()
}

fn menu(page_commands: &[PageCommandState], templates: &[MenuTemplate]) -> Vec<AppMenuSection> {
    app_menu(&AppMenuInput {
        page_commands,
        shortcuts: &defaults(),
        can_pause_all: true,
        can_resume_all: false,
        can_check_for_updates: false,
        templates,
    })
}

/// (title, enabled, accelerator) of every item of the menu titled `section`, separators as "—".
fn entries(sections: &[AppMenuSection], section: &str) -> Vec<(String, bool, Option<String>)> {
    sections
        .iter()
        .find(|candidate| candidate.title == section)
        .map(|found| {
            found
                .items
                .iter()
                .map(|item| match item {
                    AppMenuItem::Item {
                        title,
                        is_enabled,
                        accelerator,
                        ..
                    } => (title.clone(), *is_enabled, accelerator.clone()),
                    AppMenuItem::Label(text) => (text.clone(), false, None),
                    AppMenuItem::Separator => ("—".to_owned(), false, None),
                    AppMenuItem::Submenu { title, .. } => (format!("{title} ▸"), true, None),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn entry(title: &str, enabled: bool, accelerator: Option<&str>) -> (String, bool, Option<String>) {
    (title.to_owned(), enabled, accelerator.map(str::to_owned))
}

fn separator() -> (String, bool, Option<String>) {
    entry("—", false, None)
}

#[test]
fn the_menus_follow_the_macos_order() {
    let sections = menu(&[], &[]);
    let titles: Vec<&str> = sections.iter().map(|section| section.title.as_str()).collect();
    assert_eq!(titles, ["Jutsu Studio", "File", "View", "Library", "Queue"]);
    assert_eq!(
        entries(&sections, "Jutsu Studio"),
        [
            entry("About Jutsu Studio", true, None),
            entry("Check for Updates…", false, None),
            separator(),
            entry("Settings…", false, Some("Ctrl+,")),
            separator(),
            entry("Quit Jutsu Studio", true, Some("Ctrl+Q")),
        ]
    );
    assert_eq!(
        entries(&sections, "File"),
        [
            entry("Open…", false, Some("Ctrl+O")),
            entry("New from Template ▸", true, None),
            separator(),
            entry("Generate", false, Some("Ctrl+Enter")),
        ]
    );
    assert_eq!(
        entries(&sections, "View"),
        [
            entry("Create", false, Some("Ctrl+1")),
            entry("Queue", false, Some("Ctrl+2")),
            entry("Library", false, Some("Ctrl+3")),
            separator(),
            entry("Toggle Sidebar", false, Some("Ctrl+Shift+S")),
        ]
    );
    assert_eq!(
        entries(&sections, "Library"),
        [
            entry("Star", false, Some("Ctrl+L")),
            entry("Render Final…", false, None),
            separator(),
            entry("Delete from Library…", false, Some("Ctrl+Backspace")),
        ]
    );
    assert_eq!(
        entries(&sections, "Queue"),
        [
            entry("Pause All", true, None),
            entry("Resume All", false, None),
            separator(),
            entry("Cancel Job", false, Some("Ctrl+.")),
            entry("Retry Job", false, Some("Ctrl+Shift+R")),
        ]
    );
}

#[test]
fn a_published_command_is_enabled_with_its_pages_title() {
    let published = [
        PageCommandState {
            command: PageCommand::Star,
            title: Some("Unstar".to_owned()),
        },
        PageCommandState {
            command: PageCommand::DeleteResult,
            title: None,
        },
        PageCommandState {
            command: PageCommand::RetryJob,
            title: Some("Retry Download".to_owned()),
        },
    ];
    let sections = menu(&published, &[]);
    assert_eq!(
        entries(&sections, "Library"),
        [
            entry("Unstar", true, Some("Ctrl+L")),
            entry("Render Final…", false, None),
            separator(),
            entry("Delete from Library…", true, Some("Ctrl+Backspace")),
        ]
    );
    assert_eq!(
        entries(&sections, "Queue")[4],
        entry("Retry Download", true, Some("Ctrl+Shift+R"))
    );
}

#[test]
fn a_removed_shortcut_leaves_the_item_without_one() {
    let mut shortcuts = defaults();
    shortcuts.remove(&ShortcutAction::Star);
    shortcuts.insert(
        ShortcutAction::Open,
        Shortcut::character('k', Modifiers::CTRL_ALT),
    );
    let sections = app_menu(&AppMenuInput {
        page_commands: &[],
        shortcuts: &shortcuts,
        can_pause_all: false,
        can_resume_all: false,
        can_check_for_updates: true,
        templates: &[],
    });
    assert_eq!(entries(&sections, "Library")[0], entry("Star", false, None));
    assert_eq!(
        entries(&sections, "File")[0],
        entry("Open…", false, Some("Ctrl+Alt+K"))
    );
    assert_eq!(
        entries(&sections, "Jutsu Studio")[1],
        entry("Check for Updates…", true, None)
    );
}

#[test]
fn new_from_template_lists_the_templates_or_says_there_are_none() {
    let none = menu(&[], &[]);
    let AppMenuItem::Submenu { items, .. } = &none[1].items[1] else {
        panic!("New from Template is a submenu");
    };
    assert_eq!(items, &[AppMenuItem::Label("No templates yet".to_owned())]);

    let id = Uuid::from_u128(7);
    let some = menu(
        &[],
        &[MenuTemplate {
            id,
            name: "Dance".to_owned(),
        }],
    );
    let AppMenuItem::Submenu { items, .. } = &some[1].items[1] else {
        panic!("New from Template is a submenu");
    };
    assert_eq!(
        items,
        &[AppMenuItem::Item {
            action: AppMenuAction::Template(id),
            title: "Dance".to_owned(),
            is_enabled: true,
            accelerator: None,
        }]
    );
    assert!(!same_layout(&none, &some));
}

#[test]
fn titles_and_states_change_in_place() {
    let before = menu(&[], &[]);
    let after = menu(
        &[PageCommandState {
            command: PageCommand::Star,
            title: Some("Unstar".to_owned()),
        }],
        &[],
    );
    assert_ne!(before, after);
    assert!(same_layout(&before, &after));
}

#[test]
fn item_ids_round_trip() {
    let mut actions = vec![
        AppMenuAction::About,
        AppMenuAction::CheckForUpdates,
        AppMenuAction::Quit,
        AppMenuAction::PauseAll,
        AppMenuAction::ResumeAll,
        AppMenuAction::Template(Uuid::from_u128(42)),
    ];
    actions.extend(PageCommand::ALL.into_iter().map(AppMenuAction::Page));
    for action in actions {
        assert_eq!(
            AppMenuAction::from_id(&action.id()),
            Some(action.clone()),
            "{action:?}"
        );
    }
    assert_eq!(
        AppMenuAction::Page(PageCommand::DeleteResult).id(),
        "app.page.deleteResult"
    );
    for id in [
        "main",
        "tray.panel.0",
        "app.page.nothing",
        "app.template.x",
        "app.unknown",
    ] {
        assert_eq!(AppMenuAction::from_id(id), None, "{id}");
    }
}
