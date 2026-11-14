//! "Made from" and "Used in" (`MaterialRelations.swift`): the images a video was made from and the videos made from
//! an image, read from one snapshot of the catalog. A source removed or whose file is gone stays, marked unavailable.

use std::collections::HashMap;
use std::path::Path;

use jutsu_core::{Material, MaterialInputRole, MaterialKind, MaterialReference, MaterialVersion};
use serde::Serialize;
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

/// Whether a linked version can still be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum LinkAvailability {
    /// The Library lists it: a file here, or an account result.
    Listed,
    /// The file is here, but the Library doesn't list it.
    NotListed,
    /// Removed or deleted, its file gone or on a drive that isn't connected.
    Unavailable,
}

/// One end of a "made from" relation.
#[derive(Debug, Clone, PartialEq)]
pub struct MaterialLink {
    pub reference: MaterialReference,
    pub kind: MaterialKind,
    pub role: MaterialInputRole,
    /// 0-based among the job's images.
    pub position: i64,
    /// The file here, else where the account keeps it.
    pub preview_url: Option<Url>,
    pub prompt: String,
    pub created_at: Option<OffsetDateTime>,
    pub availability: LinkAvailability,
}

impl MaterialLink {
    /// "Start frame", "End frame", "Reference 2".
    pub fn role_title(&self) -> String {
        match self.role {
            MaterialInputRole::StartFrame => "Start frame".to_owned(),
            MaterialInputRole::EndFrame => "End frame".to_owned(),
            MaterialInputRole::Reference => format!("Reference {}", self.position + 1),
            MaterialInputRole::SourceVideo => "Source video".to_owned(),
            MaterialInputRole::Parent => "Edited from".to_owned(),
        }
    }

    /// Shown under an unavailable or unlisted source.
    pub fn availability_note(&self) -> Option<&'static str> {
        match self.availability {
            LinkAvailability::Listed => None,
            LinkAvailability::NotListed => Some("Not in the library"),
            LinkAvailability::Unavailable => Some("Unavailable"),
        }
    }
}

/// The roles an image plays in a video.
fn is_image_role(role: MaterialInputRole) -> bool {
    matches!(
        role,
        MaterialInputRole::StartFrame | MaterialInputRole::EndFrame | MaterialInputRole::Reference
    )
}

pub struct MaterialRelations<'a, F: Fn(&Path) -> bool> {
    materials: &'a [Material],
    by_id: HashMap<Uuid, &'a Material>,
    file_exists: F,
}

impl<'a, F: Fn(&Path) -> bool> MaterialRelations<'a, F> {
    pub fn new(materials: &'a [Material], file_exists: F) -> Self {
        let mut by_id = HashMap::with_capacity(materials.len());
        for material in materials {
            by_id.entry(material.id).or_insert(material);
        }
        Self {
            materials,
            by_id,
            file_exists,
        }
    }

    /// The version `reference` names.
    pub fn version(&self, reference: MaterialReference) -> Option<&'a MaterialVersion> {
        self.by_id
            .get(&reference.material_id)?
            .versions()
            .into_iter()
            .find(|version| version.id == reference.version_id)
    }

    /// The images the version was made from, in the order its job sent them.
    pub fn made_from(&self, reference: MaterialReference) -> Vec<MaterialLink> {
        self.version(reference)
            .map(|version| version.inputs.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|input| is_image_role(input.role))
            .map(|input| self.link(input.source, input.role, input.position, MaterialKind::Image))
            .collect()
    }

    /// The videos made from any version of the material, newest first.
    pub fn used_in(&self, material_id: Uuid) -> Vec<MaterialLink> {
        let mut links: Vec<MaterialLink> = self
            .materials
            .iter()
            .filter(|material| material.kind == MaterialKind::Video)
            .flat_map(|video| {
                video.versions().into_iter().flat_map(move |version| {
                    version
                        .inputs
                        .iter()
                        .filter(move |input| {
                            input.source.material_id == material_id && is_image_role(input.role)
                        })
                        .map(move |input| (MaterialReference::new(video.id, version.id), input))
                })
            })
            .map(|(reference, input)| self.link(reference, input.role, input.position, MaterialKind::Video))
            .collect();
        links.sort_by_key(|link| std::cmp::Reverse(link.created_at));
        links
    }

    /// The version as a link; `kind` stands in while its material is gone.
    pub fn link(
        &self,
        reference: MaterialReference,
        role: MaterialInputRole,
        position: i64,
        kind: MaterialKind,
    ) -> MaterialLink {
        let material = self.by_id.get(&reference.material_id).copied();
        let version = self.version(reference);
        MaterialLink {
            reference,
            kind: material.map_or(kind, |material| material.kind),
            role,
            position,
            preview_url: version
                .and_then(|version| version.file_url.clone().or_else(|| version.remote_url.clone())),
            prompt: version
                .and_then(|version| version.generation.as_ref())
                .map(|generation| generation.prompt.clone())
                .unwrap_or_default(),
            created_at: version.map(|version| version.created_at),
            availability: self.availability(material, version),
        }
    }

    fn availability(
        &self,
        material: Option<&Material>,
        version: Option<&MaterialVersion>,
    ) -> LinkAvailability {
        let (Some(material), Some(version)) = (material, version) else {
            return LinkAvailability::Unavailable;
        };
        if let Some(file) = &version.file_url {
            let exists = file.to_file_path().is_ok_and(|path| (self.file_exists)(&path));
            if !exists {
                return LinkAvailability::Unavailable;
            }
        } else if version.remote_url.is_none() {
            return LinkAvailability::Unavailable;
        }
        if material.is_hidden {
            LinkAvailability::NotListed
        } else {
            LinkAvailability::Listed
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use jutsu_core::{
        Material, MaterialInput, MaterialInputRole, MaterialKind, MaterialReference, MaterialVersion,
    };
    use uuid::Uuid;

    use super::{LinkAvailability, MaterialRelations};
    use crate::library::fixture::{catalog_image, file, generation, minutes_ago, video};

    const MISSING: &str = "/tmp/missing.png";

    fn input(source: MaterialReference, role: MaterialInputRole, position: i64) -> MaterialInput {
        MaterialInput {
            source,
            role,
            position,
        }
    }

    fn video_from(inputs: Vec<MaterialInput>, minutes: f64, hidden: bool) -> Material {
        let mut video = video(minutes);
        video.current.inputs = inputs;
        video.is_hidden = hidden;
        video
    }

    fn image(prompt: &str) -> Material {
        let mut image = catalog_image("", None, 10.0, jutsu_core::Culling::Unreviewed, false);
        image.current.generation = Some(generation(prompt, "soul"));
        image
    }

    fn edit(image: &mut Material) {
        let mut version = MaterialVersion::new(Uuid::new_v4(), minutes_ago(1.0));
        version.file_url = Some(file("/tmp/edit.png"));
        image.add_version(version).unwrap();
    }

    fn exists(path: &Path) -> bool {
        path != Path::new(MISSING)
    }

    #[test]
    fn made_from_names_the_exact_version_and_its_role() {
        let mut knight = image("A knight at dawn");
        let first = knight.reference();
        edit(&mut knight);
        let reference = image("Armor");
        let video = video_from(
            vec![
                input(first, MaterialInputRole::StartFrame, 0),
                input(reference.reference(), MaterialInputRole::Reference, 1),
                input(first, MaterialInputRole::SourceVideo, 0),
            ],
            1.0,
            false,
        );
        let materials = [knight.clone(), reference.clone(), video.clone()];
        let links = MaterialRelations::new(&materials, exists).made_from(video.reference());
        assert_eq!(
            links.iter().map(|link| link.reference).collect::<Vec<_>>(),
            [first, reference.reference()]
        );
        assert_eq!(
            links
                .iter()
                .map(super::MaterialLink::role_title)
                .collect::<Vec<_>>(),
            ["Start frame", "Reference 2"]
        );
        // The version the job sent, not the edit made since.
        assert_eq!(links[0].prompt, "A knight at dawn");
        assert_eq!(links[0].preview_url, knight.previous_versions()[0].file_url);
        assert!(
            links
                .iter()
                .all(|link| link.availability == LinkAvailability::Listed)
        );
    }

    #[test]
    fn a_removed_or_missing_source_stays_marked_unavailable() {
        let mut removed = image("A knight");
        removed.current.file_url = Some(file(MISSING));
        removed.is_hidden = true;
        let mut picked = image("A knight");
        picked.is_hidden = true;
        let deleted = MaterialReference::new(Uuid::new_v4(), Uuid::new_v4());
        let video = video_from(
            vec![
                input(removed.reference(), MaterialInputRole::StartFrame, 0),
                input(picked.reference(), MaterialInputRole::EndFrame, 1),
                input(deleted, MaterialInputRole::Reference, 2),
            ],
            1.0,
            false,
        );
        let materials = [removed, picked, video.clone()];
        let links = MaterialRelations::new(&materials, exists).made_from(video.reference());
        assert_eq!(
            links.iter().map(|link| link.availability).collect::<Vec<_>>(),
            [
                LinkAvailability::Unavailable,
                LinkAvailability::NotListed,
                LinkAvailability::Unavailable
            ]
        );
        assert_eq!(
            links
                .iter()
                .map(super::MaterialLink::availability_note)
                .collect::<Vec<_>>(),
            [
                Some("Unavailable"),
                Some("Not in the library"),
                Some("Unavailable")
            ]
        );
        assert_eq!(links[2].kind, MaterialKind::Image);
        assert_eq!(links[2].preview_url, None);
    }

    #[test]
    fn used_in_lists_every_video_of_any_version_newest_first() {
        let mut knight = image("A knight");
        let first = knight.reference();
        edit(&mut knight);
        let older = video_from(vec![input(first, MaterialInputRole::Reference, 0)], 30.0, false);
        let newer = video_from(
            vec![input(knight.reference(), MaterialInputRole::EndFrame, 0)],
            2.0,
            false,
        );
        let mut gone = video_from(
            vec![input(knight.reference(), MaterialInputRole::StartFrame, 0)],
            1.0,
            true,
        );
        gone.current.file_url = Some(file(MISSING));
        let unrelated = video_from(Vec::new(), 1.0, false);
        // An edit of the image is made from it too, but it isn't a video.
        let mut edit = image("A knight");
        edit.current.inputs = vec![input(first, MaterialInputRole::Parent, 0)];
        let materials = [
            knight.clone(),
            older.clone(),
            newer.clone(),
            gone.clone(),
            unrelated,
            edit,
        ];
        let links = MaterialRelations::new(&materials, exists).used_in(knight.id);
        assert_eq!(
            links
                .iter()
                .map(|link| link.reference.material_id)
                .collect::<Vec<_>>(),
            [gone.id, newer.id, older.id]
        );
        assert_eq!(
            links.iter().map(|link| link.role).collect::<Vec<_>>(),
            [
                MaterialInputRole::StartFrame,
                MaterialInputRole::EndFrame,
                MaterialInputRole::Reference
            ]
        );
        assert!(links.iter().all(|link| link.kind == MaterialKind::Video));
        assert_eq!(
            links.iter().map(|link| link.availability).collect::<Vec<_>>(),
            [
                LinkAvailability::Unavailable,
                LinkAvailability::Listed,
                LinkAvailability::Listed
            ]
        );
    }
}
