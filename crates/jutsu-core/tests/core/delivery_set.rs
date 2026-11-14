//! `DeliverySetTests.swift`.

use std::collections::HashSet;
use std::sync::LazyLock;

use jutsu_core::*;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::common::file_url;

fn image_output(
    name: &str,
    format: ImageFormat,
    long_edge: Option<i64>,
    aspect: Option<CropAspect>,
) -> DeliveryOutput {
    let mut preset = ImageExportPreset::new(format);
    preset.long_edge = long_edge;
    let mut output = DeliveryOutput::new(DeliveryPreset::Image(preset));
    name.clone_into(&mut output.name);
    output.aspect = aspect;
    output
}

static WEB: LazyLock<DeliveryOutput> =
    LazyLock::new(|| image_output("web", ImageFormat::Jpeg, Some(2048), Some(CropAspect::new(4, 5))));
static MASTER: LazyLock<DeliveryOutput> =
    LazyLock::new(|| image_output("master", ImageFormat::Png, None, None));
static PHONE: LazyLock<DeliveryOutput> =
    LazyLock::new(|| image_output("", ImageFormat::Heic, Some(1080), None));
static EDIT: LazyLock<DeliveryOutput> = LazyLock::new(|| {
    let mut output = DeliveryOutput::new(DeliveryPreset::Video(ExportPreset::EditingH264));
    output.aspect = Some(CropAspect::new(9, 16));
    output
});

fn material(kind: MaterialKind, path: &str) -> Material {
    let mut version = MaterialVersion::new(Uuid::new_v4(), OffsetDateTime::now_utc());
    version.file_url = Some(file_url(path));
    Material::new(Uuid::new_v4(), kind, OffsetDateTime::now_utc(), version)
}

fn image(name: &str) -> Material {
    material(MaterialKind::Image, &format!("/library/{name}.png"))
}

fn new_set(name: &str, items: Vec<DeliveryItem>, outputs: Vec<DeliveryOutput>) -> DeliverySet {
    DeliverySet::new(Uuid::new_v4(), name, OffsetDateTime::now_utc(), items, outputs)
}

fn set(images: usize, outputs: Option<Vec<DeliveryOutput>>) -> DeliverySet {
    let materials: Vec<Material> = (1..=images).map(|n| image(&format!("shot{n:02}"))).collect();
    let outputs = outputs.unwrap_or_else(|| vec![WEB.clone(), MASTER.clone(), PHONE.clone()]);
    new_set("Spring", DeliverySet::items_from(&materials), outputs)
}

fn folder() -> DeliveryFolder {
    DeliveryFolder {
        url: file_url("/out"),
    }
}

fn all_done(set: &mut DeliverySet) {
    let tasks = DeliveryPlan::tasks(set, DeliveryScope::All, |_| false);
    set.record(
        tasks
            .iter()
            .map(|task| task.result(DeliveryResultStatus::Done, None))
            .collect(),
        folder(),
        OffsetDateTime::now_utc(),
    );
}

fn names(tasks: &[DeliveryTask]) -> Vec<&str> {
    tasks.iter().map(|task| task.file_name.as_str()).collect()
}

// Naming

#[test]
fn twenty_images_in_three_formats_give_sixty_distinct_names() {
    let tasks = DeliveryPlan::tasks(&set(20, None), DeliveryScope::All, |_| false);

    assert_eq!(tasks.len(), 60);
    assert_eq!(
        tasks
            .iter()
            .map(|task| task.file_name.to_lowercase())
            .collect::<HashSet<_>>()
            .len(),
        60
    );
    assert_eq!(tasks[0].file_name, "Spring_01_shot01_web.jpg");
    assert_eq!(tasks[1].file_name, "Spring_01_shot01_master.png");
    assert_eq!(tasks[2].file_name, "Spring_01_shot01_heic-1080.heic");
    assert_eq!(tasks[59].file_name, "Spring_20_shot20_heic-1080.heic");
}

#[test]
fn positions_are_padded_to_the_set_size() {
    assert_eq!(
        ExportNaming::delivery_base_name("S", 7, 120, "a", "web"),
        "S_007_a_web"
    );
    assert_eq!(
        ExportNaming::delivery_base_name("S", 7, 9, "a", "web"),
        "S_07_a_web"
    );
}

#[test]
fn repeated_output_names_are_numbered() {
    let first = image_output("web", ImageFormat::Jpeg, None, None);
    let second = image_output("Web", ImageFormat::Png, None, None);
    let labels = ExportNaming::output_labels(&[first.clone(), second.clone()]);

    assert_eq!(labels[&first.id], "web");
    assert_eq!(labels[&second.id], "Web-2");
}

#[test]
fn a_taken_name_is_never_reused() {
    let tasks = DeliveryPlan::tasks(&set(1, Some(vec![MASTER.clone()])), DeliveryScope::All, |name| {
        name == "Spring_01_shot01_master.png"
    });

    assert_eq!(names(&tasks), ["Spring_01_shot01_master-2.png"]);
    assert_eq!(
        tasks
            .iter()
            .map(|task| task.replaces_existing)
            .collect::<Vec<_>>(),
        [false]
    );
}

#[test]
fn a_file_the_set_wrote_is_written_again_under_its_name() {
    let mut delivery = set(1, Some(vec![MASTER.clone()]));
    all_done(&mut delivery);

    let again = DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| true);

    assert_eq!(names(&again), ["Spring_01_shot01_master.png"]);
    assert_eq!(
        again
            .iter()
            .map(|task| task.replaces_existing)
            .collect::<Vec<_>>(),
        [true]
    );
}

#[test]
fn image_outputs_take_images_and_video_outputs_videos() {
    let video = material(MaterialKind::Video, "/library/clip.mp4");
    let delivery = new_set(
        "Mixed",
        DeliverySet::items_from(&[image("still"), video]),
        vec![MASTER.clone(), EDIT.clone()],
    );

    let tasks = DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| false);

    assert_eq!(
        names(&tasks),
        ["Mixed_01_still_master.png", "Mixed_02_clip_editingH264-9x16.mp4"]
    );
}

// Crop

fn frame(zoom: f64, center_x: f64, center_y: f64) -> CropFrame {
    CropFrame {
        zoom,
        center_x,
        center_y,
    }
}

#[test]
fn the_default_crop_is_the_largest_centred_frame() {
    let square = PixelSize::new(1000, 1000);

    assert_eq!(
        CropFrame::CENTERED.rect(CropAspect::new(4, 5), square),
        PixelRect {
            x: 100,
            y: 0,
            width: 800,
            height: 1000
        }
    );
    assert_eq!(
        CropFrame::CENTERED.rect(CropAspect::new(16, 9), PixelSize::new(1600, 1600)),
        PixelRect {
            x: 0,
            y: 350,
            width: 1600,
            height: 900
        }
    );
}

#[test]
fn a_moved_or_zoomed_crop_stays_inside_the_picture() {
    let size = PixelSize::new(1000, 1000);
    let aspect = CropAspect::new(4, 5);

    assert_eq!(
        frame(1.0, 0.0, 0.5).rect(aspect, size),
        PixelRect {
            x: 0,
            y: 0,
            width: 800,
            height: 1000
        }
    );
    assert_eq!(
        frame(1.0, 1.0, 0.5).rect(aspect, size),
        PixelRect {
            x: 200,
            y: 0,
            width: 800,
            height: 1000
        }
    );
    assert_eq!(
        frame(2.0, 1.0, 1.0).rect(aspect, size),
        PixelRect {
            x: 600,
            y: 500,
            width: 400,
            height: 500
        }
    );
}

#[test]
fn fixing_one_crop_makes_only_that_items_cropped_files_pending() {
    let mut delivery = set(20, None);
    all_done(&mut delivery);
    assert!(DeliveryPlan::tasks(&delivery, DeliveryScope::Pending, |_| true).is_empty());
    let fixed = delivery.items()[4].clone();

    delivery.set_frame(frame(1.0, 0.5, 0.2), fixed.reference, CropAspect::new(4, 5));
    let pending = DeliveryPlan::tasks(&delivery, DeliveryScope::Pending, |_| true);

    assert_eq!(
        pending.iter().map(|task| task.item.reference).collect::<Vec<_>>(),
        [fixed.reference]
    );
    assert_eq!(
        pending.iter().map(|task| task.output.id).collect::<Vec<_>>(),
        [WEB.id]
    );
    assert_eq!(
        pending
            .iter()
            .map(|task| task.replaces_existing)
            .collect::<Vec<_>>(),
        [true]
    );
    assert_eq!(
        pending.first().and_then(|task| task.frame),
        Some(frame(1.0, 0.5, 0.2))
    );
}

#[test]
fn changing_an_output_makes_its_files_pending() {
    let mut delivery = set(2, None);
    all_done(&mut delivery);

    let mut preset = ImageExportPreset::new(ImageFormat::Png);
    preset.long_edge = Some(512);
    delivery.outputs[1].preset = DeliveryPreset::Image(preset);

    assert_eq!(
        DeliveryPlan::tasks(&delivery, DeliveryScope::Pending, |_| true).len(),
        2
    );
}

// Freezing

#[test]
fn an_exported_set_keeps_its_versions() {
    let mut delivery = set(2, None);
    all_done(&mut delivery);

    assert!(delivery.is_frozen());
    assert_eq!(delivery.set_items(Vec::new()), Err(DeliverySetError::Frozen));
    assert_eq!(delivery.items().len(), 2);
}

#[test]
fn a_run_with_nothing_written_doesnt_freeze_the_set() {
    let mut delivery = set(1, Some(vec![MASTER.clone()]));
    let tasks = DeliveryPlan::tasks(&delivery, DeliveryScope::All, |_| false);

    delivery.record(
        tasks
            .iter()
            .map(|task| task.result(DeliveryResultStatus::Failed, Some("disk full".into())))
            .collect(),
        folder(),
        OffsetDateTime::now_utc(),
    );

    assert!(!delivery.is_frozen());
    assert_eq!(delivery.status(), DeliverySetStatus::NotExported);
    assert_eq!(
        DeliveryPlan::tasks(&delivery, DeliveryScope::Failed, |_| false).len(),
        1
    );
}

#[test]
fn the_next_round_takes_the_newest_versions_and_starts_unexported() {
    let mut first = image("hero");
    let mut delivery = new_set(
        "Round 1",
        DeliverySet::items_from(std::slice::from_ref(&first)),
        DeliverySet::default_outputs(),
    );
    all_done(&mut delivery);
    let held = delivery.items()[0].reference;
    let mut edited = MaterialVersion::new(Uuid::new_v4(), OffsetDateTime::now_utc());
    edited.file_url = Some(file_url("/library/hero edited.png"));
    first.add_version(edited).unwrap();

    let next = delivery.next_round("Round 2", OffsetDateTime::now_utc(), |id| {
        (id == first.id).then(|| first.reference())
    });

    assert_eq!(delivery.items()[0].reference, held);
    assert_eq!(next.items()[0].reference, first.reference());
    assert_ne!(next.items()[0].reference, held);
    assert!(!next.is_frozen());
    assert!(next.results().is_empty());
    assert_eq!(next.outputs, delivery.outputs);
}

#[test]
fn a_set_decodes_back_with_its_results() {
    let mut delivery = set(1, None);
    all_done(&mut delivery);
    delivery.note = "Second pass".into();
    delivery.review_link = "https://review.example/abc".into();

    let decoded: DeliverySet = serde_json::from_str(&serde_json::to_string(&delivery).unwrap()).unwrap();

    assert_eq!(decoded, delivery);
}

#[test]
fn the_manifest_lists_every_file_with_its_version() {
    let mut delivery = set(2, Some(vec![MASTER.clone()]));
    all_done(&mut delivery);

    let manifest = DeliveryManifest::new(&delivery, OffsetDateTime::now_utc());
    let json = manifest.encoded().unwrap();

    assert_eq!(
        manifest
            .files
            .iter()
            .map(|file| file.file_name.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("Spring_01_shot01_master.png"),
            Some("Spring_02_shot02_master.png")
        ]
    );
    assert_eq!(
        manifest
            .files
            .iter()
            .map(|file| file.version_id)
            .collect::<Vec<_>>(),
        delivery
            .items()
            .iter()
            .map(|item| item.reference.version_id)
            .collect::<Vec<_>>()
    );
    assert!(!json.contains("/library"));
}
