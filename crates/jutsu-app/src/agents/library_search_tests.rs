//! `AgentLibraryTests`: `library_search`, `library_mark` and `library_download`'s plan over catalog entries as the store
//! holds them.

use jutsu_core::{MaterialGeneration, MaterialKind, MaterialRemote, MaterialVersion, ProviderKind};
use time::Duration;
use url::Url;

use super::*;
use crate::library::fixture::{catalog_image, catalog_video, now};

struct Fixture {
    fox: Material,
    kept: Material,
    rejected: Material,
    image: Material,
}

impl Fixture {
    fn new() -> Self {
        Self {
            fox: catalog_video("A fox runs", None, 1.0, Culling::Unreviewed, false),
            kept: catalog_video("A kept owl", None, 2.0, Culling::Kept, false),
            rejected: catalog_video("A rejected fox", None, 3.0, Culling::Rejected, false),
            image: catalog_image("A fox portrait", None, 4.0, Culling::Unreviewed, false),
        }
    }

    fn materials(&self) -> Vec<Material> {
        vec![
            self.fox.clone(),
            self.kept.clone(),
            self.rejected.clone(),
            self.image.clone(),
        ]
    }

    fn ids(&self, query: &AgentLibraryQuery) -> Vec<Uuid> {
        search(&self.materials(), query)
            .0
            .iter()
            .map(|hit| hit.id)
            .collect()
    }
}

fn query() -> AgentLibraryQuery {
    AgentLibraryQuery {
        limit: 20,
        ..AgentLibraryQuery::default()
    }
}

/// An image of the account's Higgsfield history: no file on this computer, only its link.
fn cloud() -> Material {
    let mut version = MaterialVersion::new(Uuid::new_v4(), now() - Duration::seconds(30));
    version.remote_url = Some(Url::parse("https://cdn.example.com/cloud.png").unwrap());
    version.generation = Some(MaterialGeneration {
        prompt: "A cloud fox".to_owned(),
        model: "soul".to_owned(),
        parameters: None,
        title: None,
    });
    let mut material = Material::new(
        Uuid::new_v4(),
        MaterialKind::Image,
        now() - Duration::seconds(30),
        version,
    );
    material.remote = Some(MaterialRemote {
        provider: ProviderKind::Account,
        owner: None,
        id: "gen-1".to_owned(),
        output: 0,
    });
    material
}

#[test]
fn rejected_results_stay_out_unless_asked_for() {
    let f = Fixture::new();
    assert_eq!(f.ids(&query()), [f.fox.id, f.kept.id, f.image.id]);
    let with_rejected = AgentLibraryQuery {
        include_rejected: true,
        ..query()
    };
    assert_eq!(
        f.ids(&with_rejected),
        [f.fox.id, f.kept.id, f.rejected.id, f.image.id]
    );
    let rejected = AgentLibraryQuery {
        verdict: Some(Culling::Rejected),
        ..query()
    };
    assert_eq!(f.ids(&rejected), [f.rejected.id]);
    let kept = AgentLibraryQuery {
        verdict: Some(Culling::Kept),
        ..query()
    };
    assert_eq!(f.ids(&kept), [f.kept.id]);
}

#[test]
fn the_filters_are_the_librarys_and_their_own() {
    let f = Fixture::new();
    let with = |change: fn(&mut AgentLibraryQuery)| {
        let mut asked = query();
        change(&mut asked);
        f.ids(&asked)
    };
    assert_eq!(with(|q| q.text = "fox".to_owned()), [f.fox.id, f.image.id]);
    assert_eq!(with(|q| q.kind = LibraryKind::Image), [f.image.id]);
    assert_eq!(with(|q| q.kind = LibraryKind::Video), [f.fox.id, f.kept.id]);
    // A mode lists videos only; an image model's name, images only.
    assert_eq!(with(|q| q.mode = Some(Mode::Restyle)), [f.fox.id, f.kept.id]);
    assert_eq!(with(|q| q.model = Some("soul".to_owned())), [f.image.id]);
    assert_eq!(with(|q| q.starred = Some(true)).len(), 0);
    assert_eq!(
        with(|q| q.starred = Some(false)),
        [f.fox.id, f.kept.id, f.image.id]
    );
    let cutoff = now() - Duration::seconds(150);
    let from = AgentLibraryQuery {
        from: Some(cutoff),
        ..query()
    };
    assert!(f.ids(&from).contains(&f.fox.id));
    let to = AgentLibraryQuery {
        to: Some(cutoff),
        ..query()
    };
    assert!(!f.ids(&to).contains(&f.fox.id));
}

#[test]
fn the_higgsfield_history_is_listed_only_when_asked_for() {
    let f = Fixture::new();
    let cloud = cloud();
    let mut all = f.materials();
    all.push(cloud.clone());
    let fox = AgentLibraryQuery {
        text: "fox".to_owned(),
        ..query()
    };
    let local: Vec<Uuid> = search(&all, &fox).0.iter().map(|hit| hit.id).collect();
    assert_eq!(local, [f.fox.id, f.image.id]);
    let hits = search(
        &all,
        &AgentLibraryQuery {
            include_higgsfield: true,
            ..fox
        },
    )
    .0;
    assert_eq!(
        hits.iter().map(|hit| hit.id).collect::<Vec<_>>(),
        [cloud.id, f.fox.id, f.image.id]
    );
    let first = &hits[0];
    assert!(first.is_on_higgsfield && first.file.is_none() && first.model == "soul");
    assert!(
        hits[1..]
            .iter()
            .all(|hit| !hit.is_on_higgsfield && hit.file.is_some())
    );
    // A mode lists videos only, the history's images too.
    let videos = search(
        &all,
        &AgentLibraryQuery {
            mode: Some(Mode::Restyle),
            include_higgsfield: true,
            ..query()
        },
    )
    .0;
    assert!(videos.iter().all(|hit| hit.is_video));
}

#[test]
fn a_download_is_planned_from_what_each_id_is() {
    let f = Fixture::new();
    let cloud = cloud();
    let mut all = f.materials();
    all.push(cloud.clone());
    let plan = AgentLibraryDownload::plan(&[cloud.id, f.image.id, Uuid::new_v4()], &all);
    let AgentLibraryDownload::Image(remote) = &plan[0] else {
        panic!("not the history image: {:?}", plan[0]);
    };
    assert!(remote.job_id == cloud.id && remote.id == "gen-1");
    let file = f.image.current.file_url.as_ref().unwrap().to_file_path().unwrap();
    assert_eq!(plan[1], AgentLibraryDownload::Local(file));
    assert_eq!(plan[2], AgentLibraryDownload::Missing);
}

#[test]
fn pages_follow_the_cursor_without_repeats_or_gaps() {
    let f = Fixture::new();
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let (hits, next) = search(
            &f.materials(),
            &AgentLibraryQuery {
                include_rejected: true,
                limit: 1,
                cursor,
                ..query()
            },
        );
        seen.extend(hits.iter().map(|hit| hit.id));
        cursor = next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, [f.fox.id, f.kept.id, f.rejected.id, f.image.id]);
}

#[test]
fn prompts_are_cut_and_say_so() {
    let long = catalog_image(&"a".repeat(300), None, 0.0, Culling::Unreviewed, false);
    let (hits, _) = search(&[long], &query());
    let hit = &hits[0];
    assert_eq!(hit.prompt.chars().count(), PROMPT_LENGTH);
    assert!(hit.is_prompt_truncated);
    let f = Fixture::new();
    let (videos, _) = search(&[f.fox], &query());
    let video = &videos[0];
    assert!(!video.is_prompt_truncated);
    assert_eq!(video.mode.as_deref(), Some("restyle"));
    assert!(video.version == 1 && video.version_count == 1);
    assert_eq!(
        video
            .file
            .as_ref()
            .and_then(|file| file.file_name())
            .and_then(|name| name.to_str()),
        Some("out.mp4")
    );
}

/// An item says its current version's frame, so an agent checks it before scaling a result to a delivery size.
#[test]
fn an_item_says_its_frame() {
    let mut video = Fixture::new().fox;
    video.current.pixel_size = Some(PixelSize::new(554, 740));
    let (hits, _) = search(&[video], &query());
    assert_eq!(hits[0].pixel_size, Some(PixelSize::new(554, 740)));
    let item = agent_item(&hits[0]);
    assert!(item.width == Some(554) && item.height == Some(740));
}

#[test]
fn marking_again_changes_nothing() {
    let f = Fixture::new();
    let first = AgentLibraryMarks::new(
        &[f.fox.id, f.kept.id],
        Some(CullingAction::Keep),
        None,
        &f.materials(),
    );
    assert_eq!(first.culling.len(), 1);
    assert_eq!(
        first
            .results
            .iter()
            .map(|marked| marked.culling)
            .collect::<Vec<_>>(),
        [Culling::Kept, Culling::Kept]
    );
    let mut marked = f.materials();
    marked[0].current.culling = Culling::Kept;
    let again = AgentLibraryMarks::new(&[f.fox.id, f.kept.id], Some(CullingAction::Keep), None, &marked);
    assert_eq!(again.changes(), 0);
}

#[test]
fn the_star_is_apart_from_the_verdict() {
    let f = Fixture::new();
    let marks = AgentLibraryMarks::new(&[f.fox.id, f.image.id], None, Some(true), &f.materials());
    assert_eq!(marks.culling.len(), 0);
    assert_eq!(
        marks.videos.iter().map(LibraryItem::id).collect::<Vec<_>>(),
        [f.fox.id]
    );
    assert_eq!(marks.images, HashSet::from([f.image.id]));
    assert!(marks.results.iter().all(|marked| marked.is_starred));
    assert!(
        marks
            .results
            .iter()
            .all(|marked| marked.culling == Culling::Unreviewed)
    );
}

#[test]
fn an_unknown_id_changes_nothing() {
    let f = Fixture::new();
    let marks = AgentLibraryMarks::new(
        &[f.fox.id, Uuid::new_v4()],
        Some(CullingAction::Reject),
        Some(true),
        &f.materials(),
    );
    assert_eq!(marks.missing.len(), 1);
    assert_eq!(marks.changes(), 0);
}
