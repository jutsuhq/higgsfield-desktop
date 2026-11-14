//! `TemplateLibraryTests` and `TemplateNamingTests`, and the summary cases of `CreateFormTemplateTests`, ported.

use std::collections::BTreeSet;
use std::sync::Mutex;

use futures::future::BoxFuture;
use jutsu_core::{AudioOption, JobTemplate, LocalImage, Mode, Resolution, VideoModel};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use super::*;

#[derive(Default)]
struct FakeRecords {
    stored: Mutex<Vec<JobTemplate>>,
    fails_write: Mutex<bool>,
    unreadable: Mutex<bool>,
    folders: Mutex<BTreeSet<Uuid>>,
}

impl FakeRecords {
    fn with(templates: Vec<JobTemplate>) -> Self {
        Self {
            stored: Mutex::new(templates),
            ..Self::default()
        }
    }
    fn fails(&self) -> bool {
        *self.fails_write.lock().unwrap()
    }
    fn stored(&self) -> Vec<JobTemplate> {
        self.stored.lock().unwrap().clone()
    }
    fn folders(&self) -> BTreeSet<Uuid> {
        self.folders.lock().unwrap().clone()
    }
}

const OUT_OF_SPACE: &str = "The disk is full.";

impl TemplateStore for FakeRecords {
    fn names(&self, excluding: Option<Uuid>) -> BoxFuture<'_, Result<Vec<String>, String>> {
        let names = self
            .stored()
            .into_iter()
            .filter(|template| Some(template.id) != excluding)
            .map(|template| template.name)
            .collect();
        Box::pin(async move { Ok(names) })
    }
    fn add<'a>(&'a self, template: &'a JobTemplate) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if self.fails() {
                return Err(OUT_OF_SPACE.to_owned());
            }
            self.stored.lock().unwrap().push(template.clone());
            Ok(())
        })
    }
    fn rename<'a>(&'a self, id: Uuid, name: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if self.fails() {
                return Err(OUT_OF_SPACE.to_owned());
            }
            for template in self.stored.lock().unwrap().iter_mut() {
                if template.id == id {
                    template.name = name.to_owned();
                }
            }
            Ok(())
        })
    }
    fn delete(&self, id: Uuid) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if self.fails() {
                return Err(OUT_OF_SPACE.to_owned());
            }
            self.stored.lock().unwrap().retain(|template| template.id != id);
            Ok(())
        })
    }
    fn copy_images(&self, references: &[LocalImage], id: Uuid) -> Result<Vec<Url>, String> {
        if *self.unreadable.lock().unwrap() {
            return Err("no permission".to_owned());
        }
        self.folders.lock().unwrap().insert(id);
        Ok(references
            .iter()
            .map(|image| {
                let name = image
                    .file_url
                    .path_segments()
                    .unwrap()
                    .next_back()
                    .unwrap()
                    .to_owned();
                Url::from_file_path(format!("/templates/{id}/{name}")).unwrap()
            })
            .collect())
    }
    fn remove_images(&self, id: Uuid) {
        self.folders.lock().unwrap().remove(&id);
    }
}

fn file(path: &str) -> Url {
    Url::from_file_path(path).unwrap()
}

fn draft(name: &str) -> JobTemplate {
    let mut template = JobTemplate::new(
        Uuid::new_v4(),
        name,
        OffsetDateTime::UNIX_EPOCH,
        Mode::MotionTransfer,
        VideoModel::GENJUTSU,
        Resolution::P720,
    );
    let mut second = LocalImage::new(file("/home/me/b.png"));
    second.prompt = Some("neon cat".to_owned());
    template.references = vec![LocalImage::new(file("/home/me/a.png")), second];
    template
}

#[tokio::test]
async fn a_saved_template_gets_a_free_name_and_its_own_copies() {
    let records = FakeRecords::with(vec![draft("Dance")]);
    let saved = TemplateLibrary { store: &records }
        .save(&draft("dance"))
        .await
        .unwrap();

    assert_eq!(saved.name, "dance 2");
    let paths: Vec<Url> = saved
        .references
        .iter()
        .map(|image| image.file_url.clone())
        .collect();
    assert_eq!(
        paths,
        ["a.png", "b.png"].map(|name| file(&format!("/templates/{}/{name}", saved.id)))
    );
    let prompts: Vec<Option<String>> = saved
        .references
        .iter()
        .map(|image| image.prompt.clone())
        .collect();
    assert_eq!(prompts, vec![None, Some("neon cat".to_owned())]);
    assert_eq!(records.stored().last(), Some(&saved));
}

#[tokio::test]
async fn an_unreadable_image_saves_nothing_and_says_which() {
    let records = FakeRecords::default();
    *records.unreadable.lock().unwrap() = true;

    let error = TemplateLibrary { store: &records }
        .save(&draft("Dance"))
        .await
        .unwrap_err();

    assert_eq!(error, TemplateError::UnreadableImage);
    assert_eq!(records.stored().len(), 0);
    let notice = not_saved(&error);
    assert_eq!(notice.title, "Can't save the template");
    assert_eq!(
        notice.detail,
        "A reference image can't be read. Add it again, then save the template."
    );
}

#[tokio::test]
async fn a_failed_write_removes_the_copies_again() {
    let records = FakeRecords::default();
    *records.fails_write.lock().unwrap() = true;

    let error = TemplateLibrary { store: &records }
        .save(&draft("Dance"))
        .await
        .unwrap_err();

    assert_eq!(error, TemplateError::Store(OUT_OF_SPACE.to_owned()));
    assert!(records.folders().is_empty());
    assert_eq!(not_saved(&error).title, "Can't save the template");
}

#[tokio::test]
async fn a_rename_ignores_the_templates_own_name() {
    let dance = draft("Dance");
    let records = FakeRecords::with(vec![dance.clone(), draft("Spin")]);
    let library = TemplateLibrary { store: &records };

    assert_eq!(library.rename(dance.id, "DANCE ").await.unwrap(), "DANCE");
    assert_eq!(library.rename(dance.id, "spin").await.unwrap(), "spin 2");
    let names: Vec<String> = records
        .stored()
        .into_iter()
        .map(|template| template.name)
        .collect();
    assert_eq!(names, ["spin 2", "Spin"]);
}

#[tokio::test]
async fn images_go_only_with_their_template() {
    let records = FakeRecords::default();
    let library = TemplateLibrary { store: &records };
    let saved = library.save(&draft("Dance")).await.unwrap();

    *records.fails_write.lock().unwrap() = true;
    assert!(library.delete(saved.id).await.is_err());
    assert_eq!(records.folders(), BTreeSet::from([saved.id]));

    *records.fails_write.lock().unwrap() = false;
    library.delete(saved.id).await.unwrap();
    assert!(records.folders().is_empty() && records.stored().is_empty());
}

// MARK: Naming

#[test]
fn suggested_name_uses_mode_and_first_prompt_line() {
    assert_eq!(
        suggested_name(mode_short_title(Mode::Restyle), "  rainy neon\nsecond line"),
        "Restyle · rainy neon"
    );
    assert_eq!(suggested_name(mode_short_title(Mode::ObjectSwap), ""), "Swap");
}

#[test]
fn long_names_are_cut() {
    let name = suggested_name(mode_short_title(Mode::MotionTransfer), &"a".repeat(200));
    assert_eq!(name.chars().count(), MAXIMUM_LENGTH);
    assert!(name.ends_with('…'));
}

#[test]
fn unique_name_numbers_taken_names() {
    assert_eq!(unique_name("Neon", &[]), "Neon");
    assert_eq!(unique_name(" Neon ", &["neon".to_owned()]), "Neon 2");
    assert_eq!(
        unique_name("Neon", &["Neon".to_owned(), "Neon 2".to_owned()]),
        "Neon 3"
    );
}

#[test]
fn empty_name_falls_back() {
    assert_eq!(unique_name("  ", &[]), "Untitled template");
    assert_eq!(
        unique_name("", &["Untitled template".to_owned()]),
        "Untitled template 2"
    );
}

fn plain(name: &str, mode: Mode, model: VideoModel, resolution: Resolution) -> JobTemplate {
    JobTemplate::new(
        Uuid::new_v4(),
        name,
        OffsetDateTime::UNIX_EPOCH,
        mode,
        model,
        resolution,
    )
}

#[test]
fn summary_lists_the_inputs() {
    let mut motion = plain("A", Mode::MotionTransfer, VideoModel::GENJUTSU, Resolution::P720);
    motion.references = vec![
        LocalImage::new(file("/tmp/1.png")),
        LocalImage::new(file("/tmp/2.png")),
    ];
    motion.audio = AudioOption::Mute;
    assert_eq!(summary(&motion), "Motion · 2 images · 720p · Mute");
    assert_eq!(
        summary(&plain("B", Mode::Restyle, VideoModel::GENJUTSU, Resolution::P480)),
        "Restyle · 480p"
    );
}

#[test]
fn summary_names_a_model_other_than_genjutsu() {
    let seedance = plain(
        "A",
        Mode::MotionTransfer,
        VideoModel::SEEDANCE25,
        Resolution::P720,
    );
    assert_eq!(summary(&seedance), "Motion · Seedance 2.5 · 720p");
    let genjutsu = plain("B", Mode::MotionTransfer, VideoModel::GENJUTSU, Resolution::P720);
    assert_eq!(summary(&genjutsu), "Motion · 720p");
}

// MARK: Image templates (`ReferenceGeneratorTests`)

fn image_template(
    model: &str,
    title: Option<&str>,
    ratio: &str,
    count: i64,
    references: Vec<LocalImage>,
) -> JobTemplate {
    JobTemplate::image(
        Uuid::new_v4(),
        "Knight",
        OffsetDateTime::UNIX_EPOCH,
        Some(model.to_owned()),
        title.map(str::to_owned),
        Some(ratio.to_owned()),
        count,
        "knight",
        references,
    )
}

#[test]
fn image_template_summary_names_the_model_and_the_frame() {
    let knight = image_template("soul", Some("Soul 2"), "16:9", 4, Vec::new());
    assert_eq!(summary(&knight), "Image · Soul 2 · 16:9 · 4 variants");
    let edit = image_template(
        "qwen",
        None,
        "1:1",
        1,
        vec![LocalImage::new(file("/tmp/knight.png"))],
    );
    assert_eq!(summary(&edit), "Image · 1:1 · 1 image");
}

#[test]
fn image_template_names_use_the_kind_and_the_prompt() {
    let image = crate::images::form::kind_title(jutsu_core::TemplateKind::Image);
    assert_eq!(suggested_name(image, "  a knight\nsecond"), "Image · a knight");
    assert_eq!(suggested_name(image, ""), "Image");
}
