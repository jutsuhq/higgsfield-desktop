//! `CreateFormGroupsTests`, ported: the draft's model picks, frames and batch are groups of their own, written flat
//! into the draft as before (`CreateFormCodingTests`, `FramePairTests`, `CreateBatchTests`).

use jutsu_core::{CharacterOrientation, CinemaControl, LocalImage, Mode, ProviderKind, SeedanceBitrate};
use serde_json::{Value, json};
use url::Url;

use super::elements::SavedElement;
use super::form::{CreateBatch, CreateForm, FramePair, FrameSlot, SavedSet};
use super::form_tests::{image, images};

fn element(owner: Option<&str>) -> SavedElement {
    SavedElement {
        id: "el".to_owned(),
        owner: owner.map(str::to_owned),
    }
}

/// The groups' fields sit at the draft's top level, next to the form's own, and read back and write out the same.
#[test]
fn a_draft_saved_before_the_groups_decodes_and_is_written_the_same() {
    let mut flat = serde_json::to_value(CreateForm::default()).unwrap();
    let file = |index: usize| serde_json::to_value(image(index)).unwrap();
    let groups = json!({
        "seedanceBitrate": "standard",
        "klingOrientation": "image",
        "cinemaPicks": {"genre": "noir"},
        "variantCount": 3,
        "startFrame": file(3),
        "endFrame": file(4),
        "isBatch": true,
        "batchPresetIDs": ["22222222-2222-2222-2222-222222222222"],
        "characterSets": [[file(5)]],
        "savedCharacterSets": [{"files": [image(5).file_url], "element": {"id": "el1", "owner": "me"}}],
    });
    let (Value::Object(fields), Value::Object(added)) = (&mut flat, groups) else {
        panic!("a form is written as an object");
    };
    fields.extend(added);

    let form: CreateForm = serde_json::from_value(flat.clone()).unwrap();
    assert_eq!(
        form.bitrate(Some(ProviderKind::ApiKey)),
        SeedanceBitrate::Standard
    );
    assert_eq!(form.character_orientation(), CharacterOrientation::Image);
    assert_eq!(form.cinema_controls().get(CinemaControl::Genre), Some("noir"));
    assert_eq!(form.variants(), 3);
    assert_eq!(form.frame(FrameSlot::Start), Some(&image(3)));
    assert_eq!(form.frame(FrameSlot::End), Some(&image(4)));
    assert!(form.is_batch());
    assert_eq!(form.character_sets(), [images([5])]);
    assert_eq!(
        form.saved_element(0, Some("me")),
        Some(&SavedElement {
            id: "el1".to_owned(),
            owner: Some("me".to_owned())
        })
    );

    assert_eq!(serde_json::to_value(&form).unwrap(), flat);
}

#[test]
fn a_jobs_images_fill_the_slots_in_order() {
    let images = images([1, 2, 3]);
    let mut frames = FramePair::from_images(&images);
    assert_eq!(frames.get(FrameSlot::Start), Some(&images[0]));
    assert_eq!(frames.get(FrameSlot::End), Some(&images[1]));
    assert_eq!(frames.images(), images[..2]);
    frames.set(FrameSlot::Start, None);
    assert_eq!(frames.images(), [images[1].clone()]);
    assert!(!frames.is_empty());
    frames.set(FrameSlot::End, None);
    assert!(frames.is_empty() && FramePair::default().is_empty());
}

#[test]
fn a_published_set_stays_published_while_an_equal_set_is_left() {
    let images = images([1, 2]);
    let mut batch = CreateBatch {
        character_sets: vec![images.clone(), images.clone()],
        ..CreateBatch::default()
    };
    batch.set_saved_element(element(Some("me")), &images);
    assert_eq!(batch.saved_element(1, Some("me")), Some(&element(Some("me"))));
    assert_eq!(batch.saved_element(1, Some("someone else")), None);
    batch.remove_character_set(0);
    assert_eq!(batch.saved_element(0, Some("me")), Some(&element(Some("me"))));
    batch.remove_character_set(0);
    assert_eq!(batch.saved_sets, Some(Vec::new()));
    assert_eq!(batch.count(Mode::MotionTransfer), 0);
}

/// Swift's `reopeningSets` can move a file; the port's keeps or drops each file, which is what reopening a draft
/// does (`reopening_files`).
#[test]
fn reopening_keeps_the_published_mark_only_for_whole_sets() {
    let images = images([1, 2]);
    let mut batch = CreateBatch {
        character_sets: vec![images.clone(), vec![images[0].clone()]],
        ..CreateBatch::default()
    };
    batch.set_saved_element(element(None), &images);
    batch.set_saved_element(element(None), &images[..1]);
    let kept = images[0].file_url.clone();

    let reopened = batch.reopening_sets(&|file: &Url| *file == kept);

    // The first set lost a file, so only the second is still the one published.
    let first: Vec<LocalImage> = vec![images[0].clone()];
    assert_eq!(reopened.character_sets, [first.clone(), first]);
    assert_eq!(
        reopened.saved_sets,
        Some(vec![SavedSet {
            files: vec![kept],
            element: element(None)
        }])
    );
    let lost = CreateBatch {
        character_sets: vec![images],
        ..CreateBatch::default()
    };
    assert_eq!(
        lost.reopening_sets(&|_| false).character_sets,
        Vec::<Vec<LocalImage>>::new()
    );
}
