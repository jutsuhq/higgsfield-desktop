//! The main window's sidebar (`RootView.swift`): shown or hidden by its toggle and View › Toggle Sidebar, dragged
//! between 228 and 320 wide. Both are kept across launches, as the macOS window keeps them (`@SceneStorage` and the
//! split view's own width).

use serde::Serialize;
use serde_json::Value;

use crate::preferences::{PreferencesError, PreferencesStore};

/// `.navigationSplitViewColumnWidth(min: 228, ideal: 228, max: 320)`.
pub const MIN_WIDTH: i32 = 228;
pub const MAX_WIDTH: i32 = 320;
pub const DEFAULT_WIDTH: i32 = MIN_WIDTH;

const COLLAPSED_KEY: &str = "sidebar.collapsed";
const WIDTH_KEY: &str = "sidebar.width";

/// The sidebar as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SidebarLayout {
    pub is_collapsed: bool,
    pub width: i32,
    pub min_width: i32,
    pub max_width: i32,
    /// "Hide sidebar (Ctrl+Shift+S)", the toggle's hover hint.
    pub toggle_hint: String,
    /// "Hide Sidebar", the toggle's accessibility label.
    pub toggle_label: String,
}

/// A width the sidebar can take.
pub fn clamped_width(width: i32) -> i32 {
    width.clamp(MIN_WIDTH, MAX_WIDTH)
}

impl SidebarLayout {
    pub fn new(is_collapsed: bool, width: i32) -> Self {
        let (hint, label) = if is_collapsed {
            ("Show sidebar (Ctrl+Shift+S)", "Show Sidebar")
        } else {
            ("Hide sidebar (Ctrl+Shift+S)", "Hide Sidebar")
        };
        Self {
            is_collapsed,
            width: clamped_width(width),
            min_width: MIN_WIDTH,
            max_width: MAX_WIDTH,
            toggle_hint: hint.to_owned(),
            toggle_label: label.to_owned(),
        }
    }
}

/// The sidebar in the preferences.
pub struct SidebarStore<'a>(pub &'a PreferencesStore);

impl SidebarStore<'_> {
    pub fn layout(&self) -> SidebarLayout {
        let is_collapsed = matches!(self.0.value(COLLAPSED_KEY), Some(Value::Bool(true)));
        let width = self
            .0
            .value(WIDTH_KEY)
            .and_then(|value| value.as_i64())
            .and_then(|width| i32::try_from(width).ok())
            .unwrap_or(DEFAULT_WIDTH);
        SidebarLayout::new(is_collapsed, width)
    }

    /// Stores what changed; a width outside the range is kept at its edge.
    pub fn set(
        &self,
        is_collapsed: Option<bool>,
        width: Option<i32>,
    ) -> Result<SidebarLayout, PreferencesError> {
        if let Some(is_collapsed) = is_collapsed {
            self.0
                .set_value(COLLAPSED_KEY, is_collapsed.then_some(Value::Bool(true)))?;
        }
        if let Some(width) = width {
            let width = clamped_width(width);
            self.0
                .set_value(WIDTH_KEY, (width != DEFAULT_WIDTH).then(|| Value::from(width)))?;
        }
        Ok(self.layout())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_stay_between_the_split_views_bounds() {
        assert_eq!(clamped_width(100), 228);
        assert_eq!(clamped_width(250), 250);
        assert_eq!(clamped_width(900), 320);
    }

    #[test]
    fn the_toggle_says_what_it_does() {
        let shown = SidebarLayout::new(false, 228);
        assert_eq!(shown.toggle_hint, "Hide sidebar (Ctrl+Shift+S)");
        assert_eq!(shown.toggle_label, "Hide Sidebar");
        let hidden = SidebarLayout::new(true, 228);
        assert_eq!(hidden.toggle_hint, "Show sidebar (Ctrl+Shift+S)");
        assert_eq!(hidden.toggle_label, "Show Sidebar");
    }

    #[test]
    fn the_store_keeps_both_across_launches() {
        let folder = tempfile::tempdir().unwrap();
        let preferences = PreferencesStore::open(folder.path());
        let store = SidebarStore(&preferences);
        assert_eq!(store.layout(), SidebarLayout::new(false, DEFAULT_WIDTH));

        let layout = store.set(Some(true), Some(400)).unwrap();
        assert!(layout.is_collapsed);
        assert_eq!(layout.width, 320);

        let reopened = PreferencesStore::open(folder.path());
        assert_eq!(SidebarStore(&reopened).layout(), SidebarLayout::new(true, 320));

        let layout = store.set(Some(false), None).unwrap();
        assert!(!layout.is_collapsed);
        assert_eq!(layout.width, 320);
    }
}
