//! `TemplateRecordTests`.

use jutsu_core::{AudioOption, JobState, JobTemplate, LocalImage, Mode, Resolution, VideoModel};
use jutsu_store::{FileLayout, Store};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::common::{at, file_url, job};

fn template(name: &str, created_at: OffsetDateTime) -> JobTemplate {
    let mut template = JobTemplate::new(
        Uuid::new_v4(),
        name,
        created_at,
        Mode::Restyle,
        VideoModel::genjutsu(),
        Resolution::P720,
    );
    template.references = vec![LocalImage::new(file_url("/tmp/templates/1-face.png"))];
    template.preset_id = Some(Uuid::new_v4());
    "neon".clone_into(&mut template.prompt);
    template.audio = AudioOption::Mute;
    template.normalize();
    template
}

fn names(templates: &[JobTemplate]) -> Vec<&str> {
    templates.iter().map(|template| template.name.as_str()).collect()
}

#[tokio::test]
async fn stores_and_renames_templates() {
    let store = Store::open_in_memory().await.unwrap();
    let neon = template("Neon", at(1_000));
    store.add_template(&neon).await.unwrap();
    store.add_template(&template("Clay", at(1_000))).await.unwrap();

    let templates = store.templates().await.unwrap();
    assert_eq!(names(&templates), ["Clay", "Neon"]);
    assert_eq!(templates[1], neon);

    store.rename_template(neon.id, "Alpha").await.unwrap();
    assert_eq!(names(&store.templates().await.unwrap()), ["Alpha", "Clay"]);
    let renamed = store.template(neon.id).await.unwrap().unwrap();
    assert_eq!(renamed.name, "Alpha");
    assert_eq!(renamed.references, neon.references);
    assert_eq!(store.template_names(Some(neon.id)).await.unwrap(), ["Clay"]);
    store.delete_template(neon.id).await.unwrap();
    assert_eq!(store.template_names(None).await.unwrap(), ["Clay"]);
}

/// A store written before templates were added keeps opening, with its jobs. Swift opens a store of an older
/// schema; here the schema's migrations run on a store reopened from disk.
#[tokio::test]
async fn store_written_earlier_opens_with_the_current_schema() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FileLayout::new(temp.path(), temp.path().join("Movies"));
    layout.prepare_directories().unwrap();
    let done = job(JobState::Done);
    {
        let store = Store::open(&layout.store_path()).await.unwrap();
        store.save_job(&done).await.unwrap();
        store.pool().close().await;
    }

    let store = Store::open(&layout.store_path()).await.unwrap();
    let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(store.pool())
        .await
        .unwrap();
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!((journal.as_str(), foreign_keys), ("wal", 1));
    assert_eq!(store.finished_jobs().await.unwrap(), [done]);
    store.add_template(&template("Neon", at(1_000))).await.unwrap();
    assert_eq!(store.templates().await.unwrap().len(), 1);
}
