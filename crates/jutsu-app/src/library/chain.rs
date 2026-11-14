//! The draft-to-final chain (`LibraryChain.swift`), e.g. "draft 480p → final 720p → final 1080p": the oldest known
//! ancestor first, then everything made from it, oldest first. A deleted parent stays in as a "deleted" step.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

use super::item::{LibraryItem, post_action_tag};

/// One step of a chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ChainStep {
    pub id: String,
    /// "draft 480p", "final 1080p" or "deleted".
    pub title: String,
    pub is_deleted: bool,
    pub is_current: bool,
}

/// The chain item `id` belongs to; fewer than two steps when it has no parent or child. `items`: the library.
pub fn chain(id: Uuid, items: &[LibraryItem]) -> Vec<ChainStep> {
    let by_id: HashMap<Uuid, &LibraryItem> = items.iter().map(|item| (item.id(), item)).collect();
    let Some(item) = by_id.get(&id) else {
        return Vec::new();
    };
    let mut children: HashMap<Uuid, Vec<&LibraryItem>> = HashMap::new();
    for item in items {
        if let Some(parent) = item.job.parent_id {
            children.entry(parent).or_default().push(item);
        }
    }

    let mut root = *item;
    let mut visited = HashSet::from([root.id()]);
    while let Some(parent_id) = root.job.parent_id
        && let Some(parent) = by_id.get(&parent_id)
        && visited.insert(parent_id)
    {
        root = parent;
    }
    let mut steps = Vec::new();
    let mut start = root.id();
    if let Some(missing) = root.job.parent_id
        && !by_id.contains_key(&missing)
    {
        steps.push(ChainStep {
            id: missing.to_string(),
            title: "deleted".to_owned(),
            is_deleted: true,
            is_current: false,
        });
        start = missing;
    }

    let mut descendants: Vec<&LibraryItem> = Vec::new();
    let mut pending = vec![start];
    let mut seen = HashSet::from([start]);
    while let Some(next) = pending.pop() {
        if let Some(node) = by_id.get(&next) {
            descendants.push(node);
        }
        for child in children.get(&next).into_iter().flatten() {
            if seen.insert(child.id()) {
                pending.push(child.id());
            }
        }
    }
    descendants.sort_by_key(|item| (item.job.created_at, item.id().to_string()));
    steps.extend(descendants.into_iter().map(|step| ChainStep {
        id: step.id().to_string(),
        title: title(step),
        is_deleted: false,
        is_current: step.id() == id,
    }));
    steps
}

fn title(item: &LibraryItem) -> String {
    if let Some(operation) = &item.spec().operation {
        return post_action_tag(&operation.action).to_lowercase();
    }
    let kind = if item.spec().is_draft { "draft" } else { "final" };
    format!("{kind} {}", item.resolution_title())
}

#[cfg(test)]
mod tests {
    use jutsu_core::Resolution;
    use time::Duration;
    use uuid::Uuid;

    use super::{ChainStep, chain};
    use crate::library::fixture::{ItemFixture, now};
    use crate::library::item::LibraryItem;

    fn item(resolution: Resolution, is_draft: bool, parent: Option<Uuid>, minutes: i64) -> LibraryItem {
        let mut item = ItemFixture {
            resolution,
            is_draft,
            ..ItemFixture::default()
        }
        .item();
        item.job.parent_id = parent;
        item.job.created_at = now() + Duration::minutes(minutes);
        item
    }

    fn titles(steps: &[ChainStep]) -> Vec<&str> {
        steps.iter().map(|step| step.title.as_str()).collect()
    }

    fn ids(steps: &[ChainStep]) -> Vec<String> {
        steps.iter().map(|step| step.id.clone()).collect()
    }

    #[test]
    fn a_final_shows_the_draft_it_came_from_and_its_siblings_in_order() {
        let draft = item(Resolution::P480, true, None, 0);
        let final720 = item(Resolution::P720, false, Some(draft.id()), 1);
        let final1080 = item(Resolution::P1080, false, Some(draft.id()), 2);
        let unrelated = item(Resolution::P720, false, None, 3);
        let library = vec![unrelated, final1080.clone(), final720.clone(), draft.clone()];

        let steps = chain(final1080.id(), &library);
        assert_eq!(titles(&steps), ["draft 480p", "final 720p", "final 1080p"]);
        assert_eq!(
            ids(&steps),
            [draft.id(), final720.id(), final1080.id()].map(|id| id.to_string())
        );
        assert_eq!(
            steps.iter().map(|step| step.is_current).collect::<Vec<_>>(),
            [false, false, true]
        );
        assert_eq!(ids(&chain(draft.id(), &library)), ids(&steps));
    }

    #[test]
    fn a_rerun_of_a_final_extends_the_chain() {
        let draft = item(Resolution::P480, true, None, 0);
        let final720 = item(Resolution::P720, false, Some(draft.id()), 1);
        let rerun = item(Resolution::P1080, false, Some(final720.id()), 2);
        let steps = chain(draft.id(), &[rerun, final720, draft.clone()]);
        assert_eq!(titles(&steps), ["draft 480p", "final 720p", "final 1080p"]);
        assert!(steps[0].is_current);
    }

    #[test]
    fn a_deleted_parent_stays_as_a_deleted_step_and_keeps_its_children() {
        let deleted = Uuid::new_v4();
        let final720 = item(Resolution::P720, false, Some(deleted), 1);
        let final1080 = item(Resolution::P1080, false, Some(deleted), 2);
        let steps = chain(final720.id(), &[final1080, final720.clone()]);
        assert_eq!(titles(&steps), ["deleted", "final 720p", "final 1080p"]);
        assert_eq!(
            steps.iter().map(|step| step.is_deleted).collect::<Vec<_>>(),
            [true, false, false]
        );
        assert_eq!(steps[0].id, deleted.to_string());
    }

    #[test]
    fn an_item_without_parent_or_children_has_no_chain() {
        let alone = item(Resolution::P720, false, None, 0);
        assert!(chain(alone.id(), std::slice::from_ref(&alone)).len() < 2);
        assert_eq!(chain(Uuid::new_v4(), &[alone]).len(), 0);
    }

    #[test]
    fn a_parent_cycle_still_ends() {
        let mut first = item(Resolution::P480, true, None, 0);
        let mut second = item(Resolution::P720, false, Some(first.id()), 1);
        first.job.parent_id = Some(second.id());
        second.job.parent_id = Some(first.id());
        assert_eq!(chain(second.id(), &[first, second.clone()]).len(), 2);
    }
}
