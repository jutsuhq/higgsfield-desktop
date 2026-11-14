//! `AgentLibraryDetailTests`: `library_get` over catalog entries as the store holds them.

use jutsu_core::{
    Culling, MaterialGeneration, MaterialInput, MaterialKind, MaterialOrigin, MaterialRemote, Mode,
    ProviderKind,
};

use super::*;
use crate::library::fixture::{ItemFixture, catalog_image, now};

fn character() -> Material {
    catalog_image("A knight", None, 20.0, Culling::Unreviewed, false)
}

fn generation(prompt: &str, model: &str, parameters: Option<Vec<u8>>) -> MaterialGeneration {
    MaterialGeneration {
        prompt: prompt.to_owned(),
        model: model.to_owned(),
        parameters,
        title: None,
    }
}

/// A motion transfer of two references, the first of them the Library's knight, its prompt longer than a row's.
fn swap(character: &Material) -> Material {
    let item = ItemFixture {
        mode: Mode::MotionTransfer,
        is_draft: false,
        prompt: "dance ".repeat(60).leak(),
        references: 2,
        ..ItemFixture::default()
    }
    .item();
    let job = item.job;
    let mut version = MaterialVersion::new(job.id, now());
    version.file_url.clone_from(&job.output_file);
    version.generation = Some(generation(&job.spec.prompt, job.spec.model.raw_value(), None));
    version.inputs = vec![MaterialInput {
        source: character.reference(),
        role: MaterialInputRole::Reference,
        position: 0,
    }];
    version.culling = Culling::Rejected;
    let (id, created) = (job.id, job.created_at);
    version.video = Some(Box::new(job));
    Material::new(id, MaterialKind::Video, created, version)
}

#[test]
fn a_video_in_full_has_the_whole_prompt_its_inputs_in_order_and_its_settings() {
    let character = character();
    let swap = swap(&character);
    let (details, missing) = details(&[swap.id], &[swap.clone(), character.clone()]);
    assert_eq!(missing.len(), 0);
    let detail = &details[0];
    // Rejected, yet named by ID, so it is there; the prompt isn't cut.
    assert_eq!(detail.hit.culling, Culling::Rejected);
    assert_eq!(detail.hit.job_id, Some(swap.id));
    assert_eq!(
        Some(&detail.hit.prompt),
        swap.current.video.as_ref().map(|job| &job.spec.prompt)
    );
    assert!(!detail.hit.is_prompt_truncated);
    assert_eq!(
        detail.inputs.iter().map(|input| input.role).collect::<Vec<_>>(),
        [
            AgentInputRole::Source,
            AgentInputRole::Reference,
            AgentInputRole::Reference
        ]
    );
    assert_eq!(
        detail
            .inputs
            .iter()
            .map(|input| input.position)
            .collect::<Vec<_>>(),
        [None, Some(1), Some(2)]
    );
    assert_eq!(detail.inputs[0].range, Some((4.0, 10.0)));
    assert_eq!(
        detail.inputs[0]
            .file
            .as_ref()
            .and_then(|file| file.file_name())
            .and_then(|name| name.to_str()),
        Some("rooftop.mov")
    );
    assert_eq!(detail.inputs[1].item, Some(character.id));
    assert_eq!(detail.inputs[2].item, None);
    assert_eq!(detail.settings.resolution.as_deref(), Some("480p"));
    assert_eq!(detail.settings.draft, Some(false));
    assert_eq!(detail.settings.audio.as_deref(), Some("original"));
    assert_eq!(detail.settings.operation, None);
}

#[test]
fn an_image_lists_what_it_edited_and_its_request() {
    let character = character();
    let request = ImageRequest {
        model: "soul".to_owned(),
        prompt: "A knight, side view".to_owned(),
        aspect_ratio: "3:4".to_owned(),
        count: 4,
        input_images: Vec::new(),
    };
    let mut version = MaterialVersion::new(Uuid::new_v4(), now());
    version.file_url = Some(crate::testing::file_url("/tmp/side.png"));
    version.generation = Some(generation(
        &request.prompt,
        "soul",
        Some(serde_json::to_vec(&request).unwrap()),
    ));
    version.inputs = vec![MaterialInput {
        source: character.reference(),
        role: MaterialInputRole::Reference,
        position: 0,
    }];
    version.aspect_ratio = Some("3:4".to_owned());
    let side = Material::new(version.id, MaterialKind::Image, now(), version);
    let (found, _) = details(&[side.id], &[side.clone(), character.clone()]);
    let detail = &found[0];
    assert_eq!(
        detail.inputs.iter().map(|input| input.item).collect::<Vec<_>>(),
        [Some(character.id)]
    );
    assert_eq!(detail.inputs[0].position, Some(1));
    assert_eq!(
        detail.inputs[0].file,
        character
            .current
            .file_url
            .as_ref()
            .and_then(|url| url.to_file_path().ok())
    );
    assert_eq!(detail.settings.count, Some(4));
    assert_eq!(detail.settings.aspect_ratio.as_deref(), Some("3:4"));
}

/// A version made from an earlier one of the same item names it, so an agent can compare or go back.
#[test]
fn a_new_version_names_the_one_it_was_made_from() {
    let mut first = MaterialVersion::new(Uuid::new_v4(), now());
    first.file_url = Some(crate::testing::file_url("/tmp/a.png"));
    let mut image = Material::new(first.id, MaterialKind::Image, now(), first);
    let original = image.reference();
    let mut upscaled = MaterialVersion::new(Uuid::new_v4(), now());
    upscaled.file_url = Some(crate::testing::file_url("/tmp/a 4x.png"));
    upscaled.inputs = vec![MaterialInput {
        source: original,
        role: MaterialInputRole::Parent,
        position: 0,
    }];
    image.add_version(upscaled).unwrap();

    let (found, _) = details(&[image.id], std::slice::from_ref(&image));
    let detail = &found[0];

    let made = detail.made_from.as_ref().unwrap();
    assert_eq!(made.reference, original);
    assert_eq!(made.version, 1);
    assert_eq!(
        made.file
            .as_ref()
            .and_then(|file| file.file_name())
            .and_then(|name| name.to_str()),
        Some("a.png")
    );
    assert_eq!(detail.hit.version, 2);
    let character = character();
    assert!(
        details(&[character.id], std::slice::from_ref(&character)).0[0]
            .made_from
            .is_none()
    );
}

/// A video of the account's Higgsfield history: no file on this computer, only its link and what the history says.
fn history_video() -> Material {
    let item = ItemFixture {
        mode: Mode::MotionTransfer,
        is_draft: false,
        prompt: "a long history prompt",
        ..ItemFixture::default()
    }
    .item();
    let mut job = item.job;
    job.output_file = None;
    let mut version = MaterialVersion::new(job.id, now());
    version.remote_url = Some(Url::parse("https://cdn.example.com/clip.mp4").unwrap());
    version.generation = Some(generation(&job.spec.prompt, job.spec.model.raw_value(), None));
    let (id, created) = (job.id, job.created_at);
    version.video = Some(Box::new(job));
    let mut material = Material::new(id, MaterialKind::Video, created, version);
    material.origin = MaterialOrigin::Higgsfield;
    material.remote = Some(MaterialRemote {
        provider: ProviderKind::Account,
        owner: None,
        id: "gen-2".to_owned(),
        output: 0,
    });
    material
}

/// One only on Higgsfield is answered from the history, with no file; an unknown ID fails alone.
#[test]
fn a_history_result_is_in_full_without_a_file_and_an_unknown_id_is_missing_alone() {
    let character = character();
    let history = history_video();
    let unknown = Uuid::new_v4();
    let (found, missing) = details(
        &[character.id, unknown, history.id],
        &[character.clone(), history.clone()],
    );
    assert_eq!(
        found.iter().map(|detail| detail.hit.id).collect::<Vec<_>>(),
        [character.id, history.id]
    );
    assert_eq!(missing, [unknown]);
    let detail = &found[1];
    assert!(detail.hit.is_on_higgsfield && detail.hit.file.is_none());
    assert_eq!(detail.hit.job_id, None);
    assert_eq!(detail.hit.prompt, "a long history prompt");
    assert_eq!(detail.hit.mode.as_deref(), Some("motion_transfer"));
    assert_eq!(detail.settings.resolution.as_deref(), Some("480p"));
    assert_eq!(
        agent_details(detail).next.as_deref(),
        Some(AgentLibraryDetails::DOWNLOAD_NEXT)
    );
}
