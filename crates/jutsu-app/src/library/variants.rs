//! Variant groups on screen (`LibraryVariantGroups.swift`): the variants of one generation folded into one card
//! that shows its face, the variant made primary or else the first in grid order.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::culling::CatalogIndex;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VariantGroups {
    /// Each group with two or more rows on screen, its members in grid order.
    pub members: HashMap<Uuid, Vec<Uuid>>,
    /// The face of each group.
    pub faces: HashMap<Uuid, Uuid>,
    group_of: HashMap<Uuid, Uuid>,
}

impl VariantGroups {
    /// `ids`: the rows on screen in grid order.
    pub fn new(ids: &[Uuid], index: &CatalogIndex) -> Self {
        let mut found: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for id in ids {
            if let Some(group) = index.entries.get(id).and_then(|entry| entry.variant_group_id) {
                found.entry(group).or_default().push(*id);
            }
        }
        let mut groups = Self::default();
        for (group, ids) in found {
            if ids.len() < 2 {
                continue;
            }
            let face = ids
                .iter()
                .find(|id| {
                    index
                        .entries
                        .get(id)
                        .is_some_and(|entry| entry.is_variant_primary)
                })
                .unwrap_or(&ids[0]);
            groups.faces.insert(group, *face);
            for id in &ids {
                groups.group_of.insert(*id, group);
            }
            groups.members.insert(group, ids);
        }
        groups
    }

    /// The group a row on screen belongs to; `None` for a row alone.
    pub fn group(&self, id: Uuid) -> Option<Uuid> {
        self.group_of.get(&id).copied()
    }

    /// Whether `id` shows: not a member of a collapsed group other than its face.
    pub fn shows(&self, id: Uuid, expanded: &HashSet<Uuid>) -> bool {
        match self.group_of.get(&id) {
            Some(group) if !expanded.contains(group) => self.faces.get(group) == Some(&id),
            _ => true,
        }
    }

    /// How many variants a collapsed group's face stands for; `None` for any other card.
    pub fn collapsed_count(&self, id: Uuid, expanded: &HashSet<Uuid>) -> Option<usize> {
        let group = self.group_of.get(&id)?;
        if expanded.contains(group) || self.faces.get(group) != Some(&id) {
            return None;
        }
        self.members.get(group).map(Vec::len)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use jutsu_core::Culling;
    use uuid::Uuid;

    use super::VariantGroups;
    use crate::library::culling::CatalogIndex;
    use crate::library::filter::LibraryFilter;
    use crate::library::fixture::{catalog_image, catalog_video, ids, image, rows, video};
    use crate::library::media::LibraryKind;

    fn collapse(groups: &VariantGroups, ids: &[Uuid], expanded: &HashSet<Uuid>) -> Vec<Uuid> {
        ids.iter()
            .copied()
            .filter(|id| groups.shows(*id, expanded))
            .collect()
    }

    #[test]
    fn video_variants_fold_into_one_card() {
        let group = Uuid::new_v4();
        let materials = vec![
            catalog_video("", Some(group), 1.0, Culling::Unreviewed, false),
            catalog_video("", Some(group), 2.0, Culling::Unreviewed, false),
            catalog_video("", Some(group), 3.0, Culling::Unreviewed, false),
            video(4.0),
        ];
        let shown = ids(&rows(&materials));
        let groups = VariantGroups::new(&shown, &CatalogIndex::new(&materials));
        let none = HashSet::new();
        assert_eq!(groups.members[&group], shown[..3]);
        assert_eq!(groups.faces[&group], materials[0].id);
        assert_eq!(
            collapse(&groups, &shown, &none),
            [materials[0].id, materials[3].id]
        );
        assert_eq!(groups.collapsed_count(materials[0].id, &none), Some(3));
        assert_eq!(groups.collapsed_count(materials[3].id, &none), None);
        let expanded = HashSet::from([group]);
        assert_eq!(collapse(&groups, &shown, &expanded).len(), 4);
        assert_eq!(groups.collapsed_count(materials[0].id, &expanded), None);
    }

    #[test]
    fn image_variants_show_their_primary() {
        let group = Uuid::new_v4();
        let materials = vec![
            catalog_image("A knight", Some(group), 1.0, Culling::Unreviewed, false),
            catalog_image("A knight", Some(group), 2.0, Culling::Unreviewed, true),
            catalog_image("A knight", Some(group), 3.0, Culling::Unreviewed, false),
        ];
        let shown = ids(&rows(&materials));
        let groups = VariantGroups::new(&shown, &CatalogIndex::new(&materials));
        assert_eq!(groups.faces[&group], materials[1].id);
        assert_eq!(collapse(&groups, &shown, &HashSet::new()), [materials[1].id]);
        assert_eq!(groups.collapsed_count(materials[1].id, &HashSet::new()), Some(3));
    }

    #[test]
    fn a_mixed_grid_groups_only_what_is_shown() {
        let (images, videos) = (Uuid::new_v4(), Uuid::new_v4());
        let materials = vec![
            catalog_image("A knight", Some(images), 1.0, Culling::Unreviewed, false),
            catalog_video("", Some(videos), 2.0, Culling::Unreviewed, false),
            catalog_image("A knight", Some(images), 3.0, Culling::Rejected, true),
            catalog_video("", Some(videos), 4.0, Culling::Unreviewed, false),
            catalog_image("A knight", Some(images), 5.0, Culling::Unreviewed, false),
            image(6.0),
        ];
        let index = CatalogIndex::new(&materials);
        let filter = LibraryFilter::default();
        let shown = ids(&filter.apply(&filter.culled(rows(&materials), &index), LibraryKind::All));
        let groups = VariantGroups::new(&shown, &index);
        // The rejected primary is hidden, so the first variant shown stands in for it.
        assert_eq!(groups.members[&images], [materials[0].id, materials[4].id]);
        assert_eq!(groups.faces[&images], materials[0].id);
        assert_eq!(groups.members[&videos], [materials[1].id, materials[3].id]);
        assert_eq!(
            collapse(&groups, &shown, &HashSet::new()),
            [materials[0].id, materials[1].id, materials[5].id]
        );
        // A search that leaves one variant shows it as a card of its own.
        let one = VariantGroups::new(&shown[..1], &index);
        assert!(one.members.is_empty());
        assert_eq!(one.group(materials[0].id), None);
    }
}
