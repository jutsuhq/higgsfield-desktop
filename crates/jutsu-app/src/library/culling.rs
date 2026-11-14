//! Keep and reject (`LibraryCulling.swift`): what each key does, the catalog facts the grid needs, and this
//! session's steps for Undo.

use std::collections::HashMap;

use jutsu_core::{Culling, Material, MaterialReference};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a culling key or button does to the versions it acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CullingAction {
    Keep,
    Reject,
    Clear,
}

impl CullingAction {
    pub fn result(self) -> Culling {
        match self {
            Self::Keep => Culling::Kept,
            Self::Reject => Culling::Rejected,
            Self::Clear => Culling::Unreviewed,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Keep => "Keep",
            Self::Reject => "Reject",
            Self::Clear => "Clear Keep or Reject",
        }
    }

    /// Keep or Reject on results that all have that verdict already takes it back, like a toggle.
    pub fn toggled(self, current: &[Culling]) -> Self {
        if self == Self::Clear
            || current.is_empty()
            || current.iter().any(|culling| *culling != self.result())
        {
            return self;
        }
        Self::Clear
    }
}

/// One material's facts the grid needs beyond its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndexEntry {
    pub reference: MaterialReference,
    pub culling: Culling,
    pub variant_group_id: Option<Uuid>,
    pub is_variant_primary: bool,
}

/// The current version's culling, the variant group and the chosen face of every material, by ID.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogIndex {
    pub entries: HashMap<Uuid, IndexEntry>,
}

impl CatalogIndex {
    pub fn new(materials: &[Material]) -> Self {
        let mut entries = HashMap::with_capacity(materials.len());
        for material in materials {
            entries.entry(material.id).or_insert(IndexEntry {
                reference: material.reference(),
                culling: material.current.culling,
                variant_group_id: material.variant_group_id,
                is_variant_primary: material.is_variant_primary,
            });
        }
        Self { entries }
    }

    pub fn culling(&self, id: Uuid) -> Culling {
        self.entries
            .get(&id)
            .map_or(Culling::Unreviewed, |entry| entry.culling)
    }

    pub fn is_rejected(&self, id: Uuid) -> bool {
        self.culling(id) == Culling::Rejected
    }
}

/// What a step writes.
pub type CullingWrites = HashMap<MaterialReference, Culling>;

/// One undoable culling step: what each version was before and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CullingStep {
    pub before: CullingWrites,
    pub after: CullingWrites,
}

/// This session's culling changes, newest last, so Undo takes back the last one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CullingHistory {
    steps: Vec<CullingStep>,
}

impl CullingHistory {
    /// Older steps drop off past this.
    pub const LIMIT: usize = 100;

    pub fn can_undo(&self) -> bool {
        !self.steps.is_empty()
    }

    /// Records `action` on `ids` and returns what to write; `None` when it changes nothing, so a repeated key
    /// doesn't take an Undo of its own. IDs the index doesn't know are skipped.
    pub fn apply(
        &mut self,
        action: CullingAction,
        ids: &[Uuid],
        index: &CatalogIndex,
    ) -> Option<CullingWrites> {
        let mut step = CullingStep {
            before: HashMap::new(),
            after: HashMap::new(),
        };
        for id in ids {
            let Some(entry) = index.entries.get(id) else {
                continue;
            };
            if entry.culling == action.result() {
                continue;
            }
            step.before.insert(entry.reference, entry.culling);
            step.after.insert(entry.reference, action.result());
        }
        if step.after.is_empty() {
            return None;
        }
        let writes = step.after.clone();
        self.steps.push(step);
        if self.steps.len() > Self::LIMIT {
            let excess = self.steps.len() - Self::LIMIT;
            self.steps.drain(..excess);
        }
        Some(writes)
    }

    /// Takes back the last step and returns what to write to restore it; `None` with nothing to undo.
    pub fn undo(&mut self) -> Option<CullingWrites> {
        self.steps.pop().map(|step| step.before)
    }
}

/// Where the cursor goes once `acted` is culled (`LibraryCursor.advanced`): the next card, so a series is culled key
/// after key. At the end it stays, unless the card leaves the grid (`leaves`), when it goes back one; `None` when
/// no card is left.
pub fn advanced_cursor<T: PartialEq + Clone>(acted: &T, ids: &[T], leaves: bool) -> Option<T> {
    let Some(index) = ids.iter().position(|id| id == acted) else {
        return Some(acted.clone());
    };
    if let Some(next) = ids.get(index + 1) {
        return Some(next.clone());
    }
    if !leaves {
        return Some(acted.clone());
    }
    index
        .checked_sub(1)
        .and_then(|previous| ids.get(previous))
        .cloned()
}

/// The cursor after a keep, reject or clear of the current card: Clear stays, a verdict moves on; a reject leaves
/// the grid unless rejected results show.
pub fn cursor_after(
    action: CullingAction,
    acted: &str,
    order: &[String],
    shows_rejected: bool,
) -> Option<String> {
    if action == CullingAction::Clear {
        return Some(acted.to_owned());
    }
    let leaves = action == CullingAction::Reject && !shows_rejected;
    advanced_cursor(&acted.to_owned(), order, leaves)
}

#[cfg(test)]
#[path = "culling_tests.rs"]
mod tests;
