//! `CreateService` over a real store and engine, with a fake provider and fake media.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use jutsu_core::{
    Balance, Capabilities, Connectivity, Cost, CostUnit, Credential, GenerationProvider, JobSpec,
    MediaPreparing, Mode, PixelSize, PortError, PreparedFile, Progress, ProviderKind, RemoteAsset,
    RemoteInputs, RemoteJob, RemoteStatus, Resolution, StylePreset,
};
use jutsu_engine::EnqueueOptions;
use jutsu_media::MediaInfo;
use rust_decimal::Decimal;
use time::UtcOffset;
use uuid::Uuid;

use super::CreateError;
use super::edit::CreateEdit;
use super::media::SourceMedia;
use super::service::{CreateDeps, CreateService, SubmitOutcome, final_spec, quoted};
use crate::connection::{ApiKeyForm, MemoryVault};
use crate::preferences::Preferences;
use crate::provider::{PersistTokens, ProviderFactory, UnlimitedOptIn};
use crate::queue::MissingFfmpeg;
use crate::testing::{BalanceProvider, FakeServices, eventually};
use crate::{AppDirectories, AppEnvironment, PlatformServices};

struct AlwaysOnline;

impl Connectivity for AlwaysOnline {
    fn updates(&self) -> BoxStream<'static, bool> {
        futures::stream::once(async { true })
            .chain(futures::stream::pending())
            .boxed()
    }
}

/// Quotes $1.00 a job and counts the quotes.
struct QuotingProvider {
    inner: BalanceProvider,
    quotes: Arc<AtomicUsize>,
}

#[async_trait]
impl GenerationProvider for QuotingProvider {
    fn kind(&self) -> ProviderKind {
        self.inner.kind()
    }
    fn owner(&self) -> Option<String> {
        self.inner.owner()
    }
    async fn capabilities(&self) -> Result<Capabilities, PortError> {
        self.inner.capabilities().await
    }
    async fn presets(&self) -> Result<Vec<StylePreset>, PortError> {
        self.inner.presets().await
    }
    async fn estimate(&self, _spec: &JobSpec) -> Result<Cost, PortError> {
        self.quotes.fetch_add(1, Ordering::SeqCst);
        Ok(Cost::usd(Decimal::ONE, false))
    }
    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError> {
        self.inner.upload(file).await
    }
    async fn submit(&self, spec: &JobSpec, inputs: &RemoteInputs, key: Uuid) -> Result<RemoteJob, PortError> {
        self.inner.submit(spec, inputs, key).await
    }
    async fn submit_is_idempotent(&self, spec: &JobSpec) -> Result<bool, PortError> {
        self.inner.submit_is_idempotent(spec).await
    }
    async fn status(&self, job: &RemoteJob) -> Result<RemoteStatus, PortError> {
        self.inner.status(job).await
    }
    async fn cancel(&self, job: &RemoteJob) -> Result<(), PortError> {
        self.inner.cancel(job).await
    }
    async fn balance(&self) -> Result<Option<Balance>, PortError> {
        self.inner.balance().await
    }
}

#[derive(Default)]
struct Factory {
    quotes: Arc<AtomicUsize>,
}

impl ProviderFactory for Factory {
    fn provider(
        &self,
        _credential: Credential,
        _persist: PersistTokens,
        _unlimited_opt_in: UnlimitedOptIn,
    ) -> Arc<dyn GenerationProvider> {
        let mut inner = BalanceProvider::credits(0, None);
        inner.kind = ProviderKind::ApiKey;
        inner.capabilities = Ok(Capabilities::new(
            BTreeSet::from([Mode::MotionTransfer, Mode::Restyle, Mode::ObjectSwap]),
            Resolution::ALL.iter().copied().collect(),
            CostUnit::Usd,
            None,
        ));
        Arc::new(QuotingProvider {
            inner,
            quotes: self.quotes.clone(),
        })
    }
}

/// Probes any file as a 14.2 s 1080p clip, HEVC for `.mov`; writes small files for the rest.
struct FakeMedia;

#[async_trait]
impl SourceMedia for FakeMedia {
    async fn probe(&self, path: &Path) -> Result<MediaInfo, CreateError> {
        if !path.is_file() {
            return Err(CreateError::Media(
                "This video's format can't be read.".to_owned(),
            ));
        }
        let is_mov = path.extension().is_some_and(|extension| extension == "mov");
        Ok(MediaInfo {
            duration: 14.2,
            pixel_size: PixelSize::new(1920, 1080),
            codec: if is_mov { "hvc1" } else { "avc1" }.to_owned(),
            frame_rate: 30.0,
            file_size: 38_000_000,
            has_audio: true,
        })
    }

    async fn thumbnails(
        &self,
        _path: &Path,
        count: usize,
        folder: &Path,
    ) -> Result<Vec<PathBuf>, CreateError> {
        std::fs::create_dir_all(folder)?;
        (0..count)
            .map(|index| {
                let file = folder.join(format!("{index}.png"));
                std::fs::write(&file, b"png")?;
                Ok(file)
            })
            .collect()
    }

    async fn proxy(
        &self,
        _path: &Path,
        output: &Path,
        _info: &MediaInfo,
        progress: Progress<f64>,
    ) -> Result<(), CreateError> {
        progress(0.5);
        std::fs::create_dir_all(output.parent().unwrap())?;
        std::fs::write(output, b"mp4")?;
        progress(1.0);
        Ok(())
    }

    async fn image_thumbnail(&self, _path: &Path, output: &Path) -> bool {
        std::fs::create_dir_all(output.parent().unwrap()).is_ok() && std::fs::write(output, b"png").is_ok()
    }

    /// A 12.5 s recording, for any file.
    async fn audio_duration(&self, path: &Path) -> Option<f64> {
        path.is_file().then_some(12.5)
    }
}

struct Fixture {
    folder: tempfile::TempDir,
    environment: Arc<AppEnvironment>,
    create: Arc<CreateService>,
    quotes: Arc<AtomicUsize>,
}

impl Fixture {
    fn file(&self, name: &str) -> PathBuf {
        let path = self.folder.path().join(name);
        std::fs::write(&path, b"file").unwrap();
        path
    }

    fn service(&self) -> Arc<CreateService> {
        CreateService::new(self.deps())
    }

    fn deps(&self) -> CreateDeps {
        let environment = &self.environment;
        CreateDeps {
            layout: environment.layout.clone(),
            store: environment.store.clone(),
            preferences: environment.preferences.clone(),
            connection: environment.connection.clone(),
            provider: environment.provider.clone(),
            engine: environment.engine.clone(),
            media: Arc::new(FakeMedia),
            utc_offset: UtcOffset::UTC,
            image_spend: environment.images.generator().spend().clone(),
            preparer: Arc::new(crate::queue::MissingFfmpeg),
        }
    }
}

async fn fixture() -> Fixture {
    let folder = tempfile::tempdir().unwrap();
    let factory = Arc::new(Factory::default());
    let quotes = factory.quotes.clone();
    let media: Arc<dyn MediaPreparing> = Arc::new(MissingFfmpeg);
    let environment = AppEnvironment::open(
        AppDirectories {
            data_dir: folder.path().join("data"),
            default_output_dir: folder.path().join("Videos"),
        },
        PlatformServices {
            vault: Arc::new(MemoryVault::new()),
            connection: Arc::new(FakeServices::default()),
            providers: factory,
            connectivity: Arc::new(AlwaysOnline),
            media: Some(media),
            utc_offset: UtcOffset::UTC,
        },
    )
    .await
    .unwrap();
    environment.start().await;
    environment
        .connection
        .connect_api_key(&ApiKeyForm::new("kid", "secret"))
        .await;
    let mut fixture = Fixture {
        folder,
        create: environment.create.clone(),
        environment,
        quotes,
    };
    fixture.create = fixture.service();
    fixture
}

/// A source and one reference: submittable.
async fn ready(fixture: &Fixture) {
    let clip = fixture.file("clip.mp4");
    fixture.create.open_source(&clip).await.unwrap();
    let reference = fixture.file("ref.png");
    fixture.create.add_references(&[reference]).unwrap();
}

#[tokio::test]
async fn an_opened_source_plays_through_a_link_and_gets_its_strip() {
    let fixture = fixture().await;
    let clip = fixture.file("clip.mp4");
    fixture.create.open_source(&clip).await.unwrap();
    let create = &fixture.create;
    for _ in 0..500 {
        let view = create.snapshot().await;
        let source = view.source.unwrap();
        if source.playback.path.is_some() && source.thumbnails.len() == 12 {
            assert!(source.playback.path.unwrap().ends_with("source.mp4"));
            assert_eq!(source.file_text, "38 MB · H.264");
            assert_eq!(view.trim.unwrap().ranges.len(), 1);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the stage never got its media");
}

#[tokio::test]
async fn hevc_gets_a_proxy_and_a_conversion_pill() {
    let fixture = fixture().await;
    let clip = fixture.file("IMG.mov");
    fixture.create.open_source(&clip).await.unwrap();
    let mut path = None;
    for _ in 0..500 {
        let source = fixture.create.snapshot().await.source.unwrap();
        assert!(source.pills.contains(&"Will convert to MP4 · H.264".to_owned()));
        if let Some(found) = source.playback.path {
            path = Some(found);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(path.unwrap().ends_with("proxy.mp4"));
}

#[tokio::test]
async fn an_unreadable_file_is_refused() {
    let fixture = fixture().await;
    let missing = fixture.folder.path().join("gone.mp4");
    assert!(matches!(
        fixture.create.open_source(&missing).await,
        Err(CreateError::Media(_))
    ));
    assert!(fixture.create.form().source().is_none());
}

#[tokio::test]
async fn dropped_files_split_into_the_source_and_references() {
    let fixture = fixture().await;
    let files = [
        fixture.file("clip.mp4"),
        fixture.file("a.png"),
        fixture.file("b.jpg"),
        fixture.file("notes.txt"),
    ];
    let outcome = fixture.create.add_files(&files).await.unwrap();
    assert_eq!(outcome.notice, None);
    let form = fixture.create.form();
    assert!(form.source().is_some());
    assert_eq!(form.references().len(), 2);
}

#[tokio::test]
async fn a_generated_image_is_added_from_its_saved_copy_until_the_references_are_full() {
    let fixture = fixture().await;
    fixture
        .create
        .edit(CreateEdit::SetTab {
            mode: "restyle".to_owned(),
        })
        .await
        .unwrap();
    let remote = url::Url::parse("https://cdn.example.com/made.png").unwrap();
    let mut generated =
        crate::images::generator::GeneratedImage::new(remote.clone(), "A red fox", "Soul 2", None);
    generated.file = Some(fixture.file("made.png"));
    assert_eq!(fixture.create.reference_room(), 5);
    assert_eq!(fixture.create.add_generated(&generated).await.unwrap(), None);
    let form = fixture.create.form();
    let added = &form.references()[0];
    assert!(added.file_url.path().ends_with("/made.png"));
    assert_eq!(added.remote_origin.as_ref(), Some(&remote));
    assert_eq!(added.prompt.as_deref(), Some("A red fox"));
    assert_eq!(fixture.create.reference_room(), 4);

    let files: Vec<PathBuf> = (0..4)
        .map(|index| fixture.file(&format!("{index}.png")))
        .collect();
    fixture.create.add_references(&files).unwrap();
    let mut another = generated.clone();
    another.file = Some(fixture.file("another.png"));
    assert_eq!(
        fixture.create.add_generated(&another).await.unwrap().as_deref(),
        Some("1 image wasn't added: Restyle takes up to 5 reference images.")
    );
}

/// `CreatePanel.recheckRoles`: a model change that cuts references says so and offers them back with Undo.
#[tokio::test]
async fn a_model_change_that_cuts_references_says_so_and_can_be_undone() {
    let fixture = fixture().await;
    let files: Vec<PathBuf> = (0..7)
        .map(|index| fixture.file(&format!("{index}.png")))
        .collect();
    fixture.create.add_references(&files).unwrap();
    let outcome = fixture
        .create
        .change_roles(CreateEdit::SetTab {
            mode: "restyle".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(
        outcome.note.as_deref(),
        Some("Genjutsu takes up to 5 reference images: 2 images were removed.")
    );
    assert!(outcome.can_undo);
    assert_eq!(fixture.create.form().references().len(), 5);
    fixture.create.undo_replacement();
    assert_eq!(fixture.create.form().references().len(), 7);
    let unchanged = fixture
        .create
        .change_roles(CreateEdit::SetModel {
            model: fixture.create.form().selected_model().raw_value().to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(unchanged.note, None);
    assert!(!unchanged.can_undo);
}

#[tokio::test]
async fn a_pasted_image_is_saved_under_pasted_and_added() {
    let fixture = fixture().await;
    fixture
        .create
        .paste_image(b"\x89PNG\r\n\x1a\nrest".to_vec())
        .await
        .unwrap();
    let form = fixture.create.form();
    let path = form.references()[0].file_url.to_file_path().unwrap();
    assert_eq!(
        path.parent().unwrap(),
        fixture.environment.layout.pasted_images_folder()
    );
    assert!(path.file_name().unwrap().to_string_lossy().starts_with("Pasted "));
    assert!(fixture.create.paste_image(b"text".to_vec()).await.is_err());
}

#[tokio::test]
async fn quotes_wait_for_the_form_to_settle_and_ignore_the_prompt() {
    let fixture = fixture().await;
    ready(&fixture).await;
    fixture.create.snapshot().await;
    eventually(|| fixture.quotes.load(Ordering::SeqCst) >= 2).await;
    let asked = fixture.quotes.load(Ordering::SeqCst);
    let spec = fixture.create.form().make_specs(false).remove(0);
    assert_eq!(
        quoted(&fixture.create, &spec),
        Some(Cost::usd(Decimal::ONE, false))
    );
    fixture
        .create
        .edit(CreateEdit::SetPrompt {
            text: "Typing reuses the price".to_owned(),
        })
        .await
        .unwrap();
    tokio::time::sleep(super::service::QUOTE_DEBOUNCE * 2).await;
    assert_eq!(fixture.quotes.load(Ordering::SeqCst), asked);
    let view = fixture.create.snapshot().await;
    assert_eq!(view.submit.generate.price.as_deref(), Some("$1.00"));
}

#[tokio::test]
async fn generate_queues_and_keeps_the_form_and_the_cap_asks_first() {
    let fixture = fixture().await;
    ready(&fixture).await;
    let SubmitOutcome::Queued { notice } = fixture.create.submit(false, false).await else {
        panic!("expected the job to be queued");
    };
    assert!(notice.starts_with("Final queued"), "{notice}");
    assert!(fixture.create.form().source().is_some());

    fixture
        .environment
        .preferences
        .update(Preferences {
            spend_cap_enabled: Some(true),
            spend_cap_usd: Some(0.5),
            ..Preferences::default()
        })
        .unwrap();
    let SubmitOutcome::Confirm { confirmation } = fixture.create.submit(true, false).await else {
        panic!("over the cap asks first");
    };
    assert_eq!(confirmation.title, "Over today's spend cap");
    let SubmitOutcome::Queued { notice } = fixture.create.submit(true, true).await else {
        panic!("approved, it starts");
    };
    assert!(notice.starts_with("Draft queued"), "{notice}");
}

#[tokio::test]
async fn edit_inputs_replaces_the_draft_with_undo() {
    let fixture = fixture().await;
    ready(&fixture).await;
    let mut spec = fixture.create.form().make_specs(false).remove(0);
    spec.prompt = "From the queue".to_owned();
    let job = fixture
        .environment
        .engine
        .enqueue(spec, EnqueueOptions::default())
        .await
        .unwrap();
    fixture
        .create
        .edit(CreateEdit::SetPrompt {
            text: "My own words".to_owned(),
        })
        .await
        .unwrap();

    let outcome = fixture.create.load_job(job.id).await.unwrap();
    assert!(outcome.can_undo);
    assert_eq!(outcome.notice, None);
    assert_eq!(fixture.create.form().prompt, "From the queue");

    fixture.create.undo_replacement();
    assert_eq!(fixture.create.form().prompt, "My own words");

    // The source file is gone: the inputs load without it.
    std::fs::remove_file(fixture.folder.path().join("clip.mp4")).unwrap();
    let outcome = fixture.create.load_job(job.id).await.unwrap();
    assert_eq!(outcome.notice.unwrap().title, "Can't open the source video");
    assert!(fixture.create.form().source().is_none());
}

#[tokio::test]
async fn render_final_queues_the_draft_as_a_final() {
    let fixture = fixture().await;
    ready(&fixture).await;
    let draft = fixture.create.form().make_specs(true).remove(0);
    assert!(draft.is_draft);
    let job = fixture
        .environment
        .engine
        .enqueue(draft.clone(), EnqueueOptions::default())
        .await
        .unwrap();
    let final_job = final_spec(&draft, Resolution::P480);
    assert!(!final_job.is_draft);
    assert_eq!(final_job.resolution, Resolution::P720);
    let SubmitOutcome::Queued { notice } = fixture.create.render_final(job.id, false).await else {
        panic!("expected the final to be queued");
    };
    assert!(notice.starts_with("Final queued"), "{notice}");
}

#[tokio::test]
async fn the_draft_survives_a_relaunch() {
    let fixture = fixture().await;
    ready(&fixture).await;
    fixture
        .create
        .edit(CreateEdit::SetPrompt {
            text: "Kept".to_owned(),
        })
        .await
        .unwrap();
    fixture.create.save_now().await;
    let reopened = fixture.service();
    assert_eq!(reopened.form().prompt, "Kept");
    assert!(reopened.form().source().is_some());
}

fn capped_at_50_cents(fixture: &Fixture) {
    fixture
        .environment
        .preferences
        .update(Preferences {
            spend_cap_enabled: Some(true),
            spend_cap_usd: Some(0.5),
            ..Preferences::default()
        })
        .unwrap();
}

#[tokio::test]
async fn under_a_cap_an_unreadable_spend_stops_the_submit() {
    let fixture = fixture().await;
    ready(&fixture).await;
    capped_at_50_cents(&fixture);
    let broken = fixture.folder.path().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join(crate::images::ImageSpendLog::FILE_NAME), b"not json").unwrap();
    let environment = &fixture.environment;
    let create = CreateService::new(CreateDeps {
        image_spend: Arc::new(crate::images::ImageSpendLog::open(
            Some(&broken),
            time::OffsetDateTime::now_utc(),
            UtcOffset::UTC,
        )),
        ..fixture.deps()
    });
    let clip = fixture.file("clip.mp4");
    create.open_source(&clip).await.unwrap();
    create.add_references(&[fixture.file("ref.png")]).unwrap();

    assert_eq!(create.submit(false, true).await, SubmitOutcome::Failed);
    let alert = environment.connection.alert().unwrap();
    assert_eq!(alert.title, "Can't read today's spending");
    assert_eq!(environment.store.active_jobs().await.unwrap().len(), 0);
}

#[tokio::test]
async fn a_submit_right_after_an_edit_decides_on_the_forms_own_price() {
    let fixture = fixture().await;
    capped_at_50_cents(&fixture);
    ready(&fixture).await;
    // No snapshot asked for quotes: the submit asks itself, so the cap compares the real $1.00.
    let SubmitOutcome::Confirm { confirmation } = fixture.create.submit(false, false).await else {
        panic!("over the cap asks first");
    };
    assert_eq!(confirmation.title, "Over today's spend cap");
    assert!(fixture.quotes.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn the_first_recording_dropped_becomes_the_speech() {
    let fixture = fixture().await;
    let image = fixture.file("face.png");
    assert_eq!(fixture.create.add_speech(&[image]).await.unwrap(), None);
    assert!(fixture.create.form().speech().is_none());

    let missing = fixture.folder.path().join("gone.wav");
    let notice = fixture.create.add_speech(&[missing]).await.unwrap();
    assert_eq!(
        notice.map(|notice| notice.title).as_deref(),
        Some("Can't read this audio file")
    );

    let voice = fixture.file("voice.M4A");
    assert_eq!(fixture.create.add_speech(&[voice]).await.unwrap(), None);
    let form = fixture.create.form();
    let speech = form.speech().unwrap();
    assert_eq!(speech.name(), "voice.M4A");
    assert!((speech.duration - 12.5).abs() < f64::EPSILON);
    fixture.create.edit(CreateEdit::RemoveSpeech).await.unwrap();
    assert!(fixture.create.form().speech().is_none());
}

/// `DuplicateJob.duplicate` → `submitDecision`: today's spend counts the image charges, and an unreadable spend
/// blocks the duplicate.
#[tokio::test]
async fn duplicating_counts_the_image_charges_and_an_unreadable_spend_blocks_it() {
    use crate::queue::service::{DuplicateOutcome, QueueService};
    let fixture = fixture().await;
    let environment = &fixture.environment;
    environment
        .preferences
        .update(Preferences {
            spend_cap_enabled: Some(true),
            spend_cap_usd: Some(1.5),
            ..Preferences::default()
        })
        .unwrap();
    // Finished long ago, so only the image charge counts today.
    let old = crate::queue::fixture::finished(jutsu_core::JobState::Done, 10.0, 5.0);
    environment.store.save_job(&old).await.unwrap();
    let now = time::OffsetDateTime::now_utc();
    environment
        .images
        .generator()
        .spend()
        .add(jutsu_core::ImageCharge::new(
            Uuid::new_v4(),
            now,
            Cost::usd(Decimal::ONE, false),
            ProviderKind::ApiKey,
            "Nano Banana Pro",
            1,
            None,
        ))
        .unwrap();
    // $1.00 of images plus the $1.00 duplicate is over the $1.50 cap.
    let DuplicateOutcome::Confirm { .. } = environment.queue.duplicate(old.id, false, now).await else {
        panic!("the image charge counts toward the cap");
    };

    let broken = fixture.folder.path().join("broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join(crate::images::ImageSpendLog::FILE_NAME), b"not json").unwrap();
    let queue = QueueService::new(
        environment.engine.clone(),
        environment.store.clone(),
        environment.preferences.clone(),
        environment.connection.clone(),
        environment.provider.clone(),
        UtcOffset::UTC,
        Arc::new(crate::images::ImageSpendLog::open(
            Some(&broken),
            now,
            UtcOffset::UTC,
        )),
    );
    assert_eq!(
        queue.duplicate(old.id, false, now).await,
        DuplicateOutcome::Failed
    );
    let alert = environment.connection.alert().unwrap();
    assert_eq!(alert.title, "Can't duplicate this job");
    assert_eq!(
        alert.message,
        jutsu_core::BudgetError::SpendingUnavailable.to_string()
    );
}

/// `SpeechSection.isAudio`: any audio type, not only MP3, WAV and M4A; an unreadable one says so.
#[tokio::test]
async fn any_audio_type_becomes_the_speech_and_an_unreadable_one_says_so() {
    let fixture = fixture().await;
    for name in [
        "book.m4b",
        "old.au",
        "talk.wma",
        "web.weba",
        "tape.aifc",
        "radio.mp2",
    ] {
        let file = fixture.file(name);
        assert_eq!(fixture.create.add_speech(&[file]).await.unwrap(), None, "{name}");
        assert!(fixture.create.form().speech().unwrap().name().ends_with(name));
    }
    let missing = fixture.folder.path().join("gone.wma");
    let notice = fixture.create.add_speech(&[missing]).await.unwrap();
    assert_eq!(
        notice.map(|notice| notice.title).as_deref(),
        Some("Can't read this audio file")
    );
}

/// `CharacterSavingTests.thereIsNoSaveWithoutElementsThatPublishOrAnUpload`: an API key has no elements, so a batch
/// character offers no Save to Higgsfield and saving it records nothing.
#[tokio::test]
async fn an_api_key_has_no_save_to_higgsfield() {
    let fixture = fixture().await;
    let create = &fixture.create;
    create.edit(CreateEdit::SetBatch { on: true }).await.unwrap();
    let reference = fixture.file("a.png");
    assert_eq!(create.add_character_set(&[reference]).unwrap(), None);
    let sets = create.snapshot().await.character_sets.unwrap().sets;
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].save, super::view::SaveState::Hidden);

    assert_eq!(create.save_character_set(0, "Mara").await.unwrap(), None);
    assert_eq!(create.form().saved_element(0, None), None);
}
