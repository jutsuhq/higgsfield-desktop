//! `ReferenceGeneratorTests`: the runs against a scripted image provider.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use jutsu_core::{CostUnit, ImageModel, ImageModelInput, PreparedFile, Rejection, SpendCap};

use super::*;
use crate::testing::{Gate, eventually};

/// An image provider whose answers the test scripts: `statuses` are handed out in order, the last one repeated.
struct FakeImageProvider {
    kind: ProviderKind,
    owner: Option<String>,
    media_id: String,
    script: Mutex<Script>,
    submitted: Mutex<Vec<ImageRequest>>,
    polls: AtomicUsize,
    cancels: AtomicUsize,
    uploads: AtomicUsize,
}

struct Script {
    estimate: Result<Cost, ProviderError>,
    submit_error: Option<ProviderError>,
    statuses: Vec<ImageJobStatus>,
    poll_error: Option<ProviderError>,
    cancel_error: Option<ProviderError>,
    submit_gate: Option<Arc<Gate<()>>>,
    upload_gate: Option<Arc<Gate<()>>>,
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn usd(text: &str, is_estimate: bool) -> Cost {
    Cost::usd(text.parse().unwrap(), is_estimate)
}

impl FakeImageProvider {
    fn new(owner: &str, media_id: &str) -> Arc<Self> {
        Arc::new(Self {
            kind: ProviderKind::ApiKey,
            owner: Some(owner.to_owned()),
            media_id: media_id.to_owned(),
            script: Mutex::new(Script {
                estimate: Ok(usd("0.0128", false)),
                submit_error: None,
                statuses: vec![ImageJobStatus::Completed(vec![url(
                    "https://cdn.example.com/1.png",
                )])],
                poll_error: None,
                cancel_error: None,
                submit_gate: None,
                upload_gate: None,
            }),
            submitted: Mutex::new(Vec::new()),
            polls: AtomicUsize::new(0),
            cancels: AtomicUsize::new(0),
            uploads: AtomicUsize::new(0),
        })
    }

    fn default() -> Arc<Self> {
        Self::new("key-1", "media-1")
    }

    fn script(&self, change: impl FnOnce(&mut Script)) {
        change(&mut self.script.lock().unwrap());
    }

    fn submitted(&self) -> Vec<ImageRequest> {
        self.submitted.lock().unwrap().clone()
    }

    fn polls(&self) -> usize {
        self.polls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ImageProvider for FakeImageProvider {
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    fn owner(&self) -> Option<String> {
        self.owner.clone()
    }
    async fn image_models(&self) -> Result<Vec<ImageModel>, PortError> {
        Ok(Vec::new())
    }
    async fn image_estimate(&self, _request: &ImageRequest) -> Result<Cost, PortError> {
        self.script
            .lock()
            .unwrap()
            .estimate
            .clone()
            .map_err(PortError::from)
    }
    async fn upload(&self, _file: PreparedFile) -> Result<RemoteAsset, PortError> {
        self.uploads.fetch_add(1, Ordering::SeqCst);
        let gate = self.script.lock().unwrap().upload_gate.clone();
        if let Some(gate) = gate {
            gate.wait().await;
        }
        let mut asset = RemoteAsset::new(url("https://cdn.example.com/in.png"));
        asset.id = Some(self.media_id.clone());
        Ok(asset)
    }
    async fn submit_images(&self, request: &ImageRequest, _key: Uuid) -> Result<Vec<RemoteJob>, PortError> {
        self.submitted.lock().unwrap().push(request.clone());
        let gate = self.script.lock().unwrap().submit_gate.clone();
        if let Some(gate) = gate {
            gate.wait().await;
        }
        if let Some(error) = self.script.lock().unwrap().submit_error.clone() {
            return Err(error.into());
        }
        Ok(vec![RemoteJob::new("req-1")])
    }
    async fn image_status(&self, _job: &RemoteJob) -> Result<ImageJobStatus, PortError> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        let mut script = self.script.lock().unwrap();
        if let Some(error) = script.poll_error.clone() {
            return Err(error.into());
        }
        Ok(if script.statuses.len() > 1 {
            script.statuses.remove(0)
        } else {
            script.statuses[0].clone()
        })
    }
    async fn cancel_images(&self, _job: &RemoteJob) -> Result<(), PortError> {
        self.cancels.fetch_add(1, Ordering::SeqCst);
        match self.script.lock().unwrap().cancel_error.clone() {
            Some(error) => Err(error.into()),
            None => Ok(()),
        }
    }
}

fn soul() -> ImageModel {
    let mut model = ImageModel::new(
        "soul",
        "Soul 2",
        ["1:1", "3:4", "9:16", "16:9"].map(str::to_owned).to_vec(),
    )
    .with_variant_counts(vec![1, 4]);
    model.list_price = Some(usd("0.0032", true));
    model
}

fn qwen() -> ImageModel {
    let mut model = ImageModel::new(
        "qwen",
        "Qwen Image 3 Edit",
        vec!["1:1".to_owned(), "21:9".to_owned()],
    );
    model.input = ImageModelInput::Required;
    model.list_price = Some(usd("0.04", true));
    model
}

fn input() -> LocalImage {
    LocalImage::new(crate::testing::file_url("/tmp/knight.png"))
}

const LANDSCAPE: PixelSize = PixelSize {
    width: 1920,
    height: 1080,
};

fn yielding() -> SleepFn {
    Arc::new(|_| Box::pin(tokio::task::yield_now()))
}

fn no_cap() -> SpendStatusReader {
    Arc::new(|_| Box::pin(async { Ok(None) }))
}

struct Setup {
    spend: Arc<ImageSpendLog>,
    jobs: Arc<ImageJobLog>,
}

impl Setup {
    fn memory() -> Self {
        Self {
            spend: Arc::new(ImageSpendLog::open(
                None,
                OffsetDateTime::now_utc(),
                time::UtcOffset::UTC,
            )),
            jobs: Arc::new(ImageJobLog::open(None)),
        }
    }

    fn generator(&self) -> Arc<ReferenceGenerator> {
        let generator = ReferenceGenerator::new(self.spend.clone(), self.jobs.clone(), no_cap(), yielding());
        generator.edit_form(|form| {
            form.use_models(vec![soul(), qwen()], None);
            form.prompt = "a knight in silver armor".to_owned();
        });
        generator
    }
}

fn upload(provider: &Arc<FakeImageProvider>) -> UploadFn {
    let provider = provider.clone();
    Arc::new(move |_| {
        let provider = provider.clone();
        Box::pin(async move {
            provider
                .upload(PreparedFile {
                    file_url: crate::testing::file_url("/tmp/knight.png"),
                    content_type: "image/png".to_owned(),
                    byte_count: 1,
                    content_hash: "h".to_owned(),
                })
                .await
        })
    })
}

fn image_provider(provider: &Arc<FakeImageProvider>) -> Arc<dyn ImageProvider> {
    provider.clone()
}

fn first_state(generator: &ReferenceGenerator) -> Option<RunState> {
    generator.runs().first().map(|run| run.state.clone())
}

async fn generate(
    generator: &Arc<ReferenceGenerator>,
    provider: &Arc<FakeImageProvider>,
    price: Option<Cost>,
) -> PaidStart {
    generator
        .generate(
            image_provider(provider),
            None,
            Mode::MotionTransfer,
            price,
            false,
            upload(provider),
        )
        .await
}

#[tokio::test]
async fn the_price_is_the_quote_or_the_list_price_when_there_is_none() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    let generator = setup.generator();
    generator.edit_form(|form| form.count = 4);
    generator
        .refresh_price(
            &image_provider(&provider),
            Some(LANDSCAPE),
            Mode::MotionTransfer,
            &upload(&provider),
        )
        .await;
    assert_eq!(generator.price(), Some(usd("0.0128", false)));

    provider.script(|script| {
        script.estimate = Err(ProviderError::Validation {
            field: None,
            message: "Nope".to_owned(),
        });
    });
    generator
        .refresh_price(
            &image_provider(&provider),
            Some(LANDSCAPE),
            Mode::MotionTransfer,
            &upload(&provider),
        )
        .await;
    assert_eq!(generator.price(), Some(usd("0.0128", true)));
    assert!(generator.price_failure().is_some());
}

#[tokio::test]
async fn the_input_is_uploaded_once_for_the_price_and_the_run() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    let generator = setup.generator();
    generator.edit_form(|form| {
        form.select_model("qwen");
        form.input_image = Some(input());
    });
    generator
        .refresh_price(
            &image_provider(&provider),
            None,
            Mode::ObjectSwap,
            &upload(&provider),
        )
        .await;
    let price = generator.price();
    generator
        .generate(
            image_provider(&provider),
            None,
            Mode::ObjectSwap,
            price,
            false,
            upload(&provider),
        )
        .await;
    eventually(|| first_state(&generator) == Some(RunState::Finished)).await;
    assert_eq!(provider.uploads.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider.submitted()[0]
            .input_images
            .iter()
            .filter_map(|asset| asset.id.clone())
            .collect::<Vec<_>>(),
        vec!["media-1".to_owned()]
    );
}

#[tokio::test]
async fn a_finished_run_adds_its_images_and_counts_its_price() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    provider.script(|script| {
        script.statuses = vec![
            ImageJobStatus::Pending,
            ImageJobStatus::Pending,
            ImageJobStatus::Completed(vec![
                url("https://cdn.example.com/1.png"),
                url("https://cdn.example.com/2.png"),
            ]),
        ];
    });
    let generator = setup.generator();
    generator.edit_form(|form| form.count = 4);
    let price = usd("0.0128", false);

    generator
        .generate(
            image_provider(&provider),
            Some(LANDSCAPE),
            Mode::MotionTransfer,
            Some(price.clone()),
            false,
            upload(&provider),
        )
        .await;
    assert!(generator.is_running());
    eventually(|| first_state(&generator) == Some(RunState::Finished)).await;

    let images = generator.images();
    assert_eq!(
        images
            .iter()
            .map(|image| image.url.path().to_owned())
            .collect::<Vec<_>>(),
        vec!["/1.png", "/2.png"]
    );
    assert!(
        images
            .iter()
            .all(|image| image.prompt == "a knight in silver armor" && image.model == "Soul 2")
    );
    assert_eq!(
        images
            .iter()
            .map(|image| image.remote.as_ref().map(|remote| remote.output))
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1)]
    );
    let run = generator.runs()[0].id;
    assert!(images.iter().all(|image| {
        image.remote.as_ref().and_then(|remote| remote.owner.as_deref()) == Some("key-1")
            && image.variant_group_id == Some(run)
    }));
    let parameters: ImageRequest = serde_json::from_slice(images[0].parameters.as_deref().unwrap()).unwrap();
    assert_eq!(parameters, provider.submitted()[0]);
    assert_eq!(provider.submitted()[0].count, 4);
    assert_eq!(provider.submitted()[0].aspect_ratio, "16:9");
    let charges = setup.spend.charges();
    assert_eq!(
        charges
            .iter()
            .map(|charge| charge.cost.clone())
            .collect::<Vec<_>>(),
        vec![price]
    );
    assert!(charges.iter().all(|charge| !charge.is_pending));
    assert_eq!(provider.polls(), 3);
}

#[tokio::test]
async fn a_run_without_images_is_not_charged() {
    for status in [
        ImageJobStatus::Failed {
            message: Some("Bad prompt".to_owned()),
        },
        ImageJobStatus::Rejected(Rejection::ExplicitContent),
    ] {
        let setup = Setup::memory();
        let provider = FakeImageProvider::default();
        provider.script(|script| script.statuses = vec![status]);
        let generator = setup.generator();
        generate(&generator, &provider, soul().list_price).await;
        eventually(|| first_state(&generator) != Some(RunState::Running)).await;
        assert!(matches!(first_state(&generator), Some(RunState::Failed(_))));
        assert_eq!(setup.spend.charges().len(), 0);
        assert_eq!(generator.images().len(), 0);
    }
}

#[tokio::test]
async fn a_refused_submit_is_not_charged() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    provider.script(|script| script.submit_error = Some(ProviderError::InsufficientFunds));
    let generator = setup.generator();
    generate(&generator, &provider, soul().list_price).await;
    eventually(|| first_state(&generator) != Some(RunState::Running)).await;
    assert_eq!(setup.spend.charges().len(), 0);
}

#[tokio::test]
async fn an_unanswered_submit_keeps_counting() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    provider.script(|script| script.submit_error = Some(ProviderError::Network));
    let generator = setup.generator();
    generate(&generator, &provider, soul().list_price).await;
    eventually(|| first_state(&generator) != Some(RunState::Running)).await;
    assert_eq!(
        setup
            .spend
            .charges()
            .iter()
            .map(|charge| charge.is_pending)
            .collect::<Vec<_>>(),
        vec![false]
    );
}

#[tokio::test]
async fn cancelling_a_queued_run_drops_its_price_unless_it_cant_be_cancelled() {
    for cancel_error in [
        None,
        Some(ProviderError::Validation {
            field: None,
            message: "Already running".to_owned(),
        }),
    ] {
        let setup = Setup::memory();
        let provider = FakeImageProvider::default();
        let refuses = cancel_error.is_some();
        provider.script(|script| {
            script.statuses = vec![ImageJobStatus::Pending];
            script.cancel_error = cancel_error;
        });
        let generator = setup.generator();
        generate(&generator, &provider, soul().list_price).await;
        eventually(|| provider.polls() > 0).await;
        generator.cancel(generator.runs()[0].id);
        eventually(|| first_state(&generator) == Some(RunState::Canceled)).await;
        assert_eq!(provider.cancels.load(Ordering::SeqCst), 1);
        assert_eq!(setup.spend.charges().len(), usize::from(refuses));
    }
}

#[tokio::test]
async fn picks_follow_the_room_and_closing_drops_only_the_picks() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    let generator = setup.generator();
    generate(&generator, &provider, None).await;
    eventually(|| first_state(&generator) == Some(RunState::Finished)).await;
    generate(&generator, &provider, None).await;
    eventually(|| generator.images().len() == 2 && first_state(&generator) == Some(RunState::Finished)).await;
    // A new request replaces the rows of the ones that are over.
    assert_eq!(generator.runs().len(), 1);
    let first = generator.images()[1].id;
    let second = generator.images()[0].id;

    assert!(generator.toggle_selection(first, 1));
    assert!(!generator.toggle_selection(second, 1));
    assert!(generator.toggle_selection(first, 1));
    assert_eq!(generator.selection(), Vec::<Uuid>::new());
    generator.toggle_selection(second, 2);
    generator.toggle_selection(first, 2);
    assert_eq!(
        generator
            .selected_images()
            .iter()
            .map(|image| image.id)
            .collect::<Vec<_>>(),
        [second, first]
    );

    generator.mark_picked(second);
    assert_eq!(generator.selection(), [first]);
    assert!(!generator.toggle_selection(second, 2));

    // Closing the quick action drops the picks; the images stay for the Image side of Create.
    generator.clear_selection();
    assert_eq!(generator.selection(), Vec::<Uuid>::new());
    assert_eq!(
        generator
            .images()
            .iter()
            .map(|image| image.id)
            .collect::<Vec<_>>(),
        [second, first]
    );
    generator.toggle_selection(first, 2);

    assert_eq!(generator.remove(first).map(|image| image.id), Some(first));
    assert_eq!(generator.selection(), Vec::<Uuid>::new());
    assert!(!generator.contains(first));
    assert_eq!(generator.remove(first), None);

    generator.clear_results();
    assert_eq!(generator.images(), Vec::<GeneratedImage>::new());
    assert_eq!(generator.selection(), Vec::<Uuid>::new());
    assert_eq!(generator.runs(), Vec::<ImageRun>::new());
}

#[tokio::test]
async fn a_poll_turned_down_for_the_sign_in_keeps_the_job_and_its_price_and_follows_it_again_after_reconnecting()
 {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    provider.script(|script| script.poll_error = Some(ProviderError::Unauthorized));
    let generator = setup.generator();
    let sign_in_requests = Arc::new(AtomicUsize::new(0));
    let counter = sign_in_requests.clone();
    generator.set_on_sign_in_expired(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));

    generate(&generator, &provider, soul().list_price).await;
    eventually(|| matches!(first_state(&generator), Some(RunState::Interrupted(_)))).await;

    // Higgsfield accepted the request: it may still finish and be charged.
    assert_eq!(sign_in_requests.load(Ordering::SeqCst), 1);
    assert!(setup.spend.charges()[0].is_pending);
    assert_eq!(
        setup
            .jobs
            .pending(Some("key-1"))
            .iter()
            .map(|record| record.id().to_owned())
            .collect::<Vec<_>>(),
        vec!["req-1".to_owned()]
    );

    // Reconnected: the same job is followed again, not sent again.
    provider.script(|script| script.poll_error = None);
    generator.reset();
    generator.resume(&image_provider(&provider));
    eventually(|| first_state(&generator) == Some(RunState::Finished)).await;

    assert_eq!(provider.submitted().len(), 1);
    assert_eq!(
        generator
            .images()
            .iter()
            .map(|image| image.prompt.clone())
            .collect::<Vec<_>>(),
        vec!["a knight in silver armor".to_owned()]
    );
    assert!(!setup.spend.charges()[0].is_pending);
    assert_eq!(setup.jobs.records().len(), 0);
}

#[tokio::test]
async fn an_accepted_request_is_followed_again_after_a_relaunch() {
    let folder = tempfile::tempdir().unwrap();
    let provider = FakeImageProvider::default();
    provider.script(|script| script.statuses = vec![ImageJobStatus::Pending]);
    let open = || {
        (
            Arc::new(ImageSpendLog::open(
                Some(folder.path()),
                OffsetDateTime::now_utc(),
                time::UtcOffset::UTC,
            )),
            Arc::new(ImageJobLog::open(Some(folder.path()))),
        )
    };
    let (spend, jobs) = open();
    let first = ReferenceGenerator::new(spend, jobs, no_cap(), yielding());
    first.edit_form(|form| {
        form.use_models(vec![soul()], None);
        form.prompt = "a knight in silver armor".to_owned();
    });
    generate(&first, &provider, soul().list_price).await;
    eventually(|| provider.polls() > 0).await;
    // The app quits while the image is being made.
    first.reset();

    provider.script(|script| {
        script.statuses = vec![ImageJobStatus::Completed(vec![url(
            "https://cdn.example.com/1.png",
        )])];
    });
    let (spend, jobs) = open();
    let second = ReferenceGenerator::new(spend.clone(), jobs, no_cap(), yielding());
    let saved: Arc<Mutex<Vec<GeneratedImage>>> = Arc::default();
    let sink = saved.clone();
    second.set_on_images(Arc::new(move |images| sink.lock().unwrap().extend(images)));
    second.resume(&image_provider(&provider));
    eventually(|| first_state(&second) == Some(RunState::Finished)).await;

    let saved = saved.lock().unwrap().clone();
    assert_eq!(
        saved
            .iter()
            .map(|image| image.prompt.as_str())
            .collect::<Vec<_>>(),
        vec!["a knight in silver armor"]
    );
    assert_eq!(
        saved.iter().map(|image| image.model.as_str()).collect::<Vec<_>>(),
        vec!["Soul 2"]
    );
    assert!(spend.charges().iter().all(|charge| !charge.is_pending));
    // The log forgets the request once its images are saved, a moment after the run shows it finished.
    eventually(|| ImageJobLog::open(Some(folder.path())).records().is_empty()).await;
    assert_eq!(provider.submitted().len(), 1);
}

#[tokio::test]
async fn quitting_can_wait_for_a_submit_already_sent_and_its_job_is_written_down() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    let gate = Gate::new();
    provider.script(|script| {
        script.submit_gate = Some(gate.clone());
        script.statuses = vec![ImageJobStatus::Pending];
    });
    let generator = setup.generator();
    generate(&generator, &provider, soul().list_price).await;
    eventually(|| gate.is_waiting()).await;
    assert_eq!(generator.sending_count(), 1);
    assert_eq!(generator.active_count(), 1);

    let waiting = generator.clone();
    let finished = tokio::spawn(async move { waiting.finish_submits().await });
    gate.open(());
    finished.await.unwrap();

    assert_eq!(generator.sending_count(), 0);
    assert_eq!(setup.jobs.pending(Some("key-1")).len(), 1);
    assert_eq!(generator.active_count(), 1);
    generator.reset();
}

#[tokio::test]
async fn a_new_connection_uploads_the_input_again_and_a_late_upload_of_the_old_one_isnt_kept() {
    let setup = Setup::memory();
    let first = FakeImageProvider::new("a", "A-media");
    let second = FakeImageProvider::new("b", "B-media");
    let generator = setup.generator();
    generator.edit_form(|form| {
        form.select_model("qwen");
        form.input_image = Some(input());
    });

    generator
        .refresh_price(&image_provider(&first), None, Mode::ObjectSwap, &upload(&first))
        .await;
    // The connection switches.
    generator.reset();
    generator
        .refresh_price(&image_provider(&second), None, Mode::ObjectSwap, &upload(&second))
        .await;
    assert_eq!(second.uploads.load(Ordering::SeqCst), 1);

    // An upload of the first account still out when the second one connects.
    let gate = Gate::new();
    first.script(|script| script.upload_gate = Some(gate.clone()));
    generator.edit_form(|form| {
        form.input_image = Some(LocalImage::new(crate::testing::file_url("/tmp/cape.png")));
    });
    let late_generator = generator.clone();
    let late_provider = first.clone();
    let late = tokio::spawn(async move {
        late_generator
            .refresh_price(
                &image_provider(&late_provider),
                None,
                Mode::ObjectSwap,
                &upload(&late_provider),
            )
            .await;
    });
    eventually(|| gate.is_waiting()).await;
    generator.reset();
    gate.open(());
    late.await.unwrap();
    assert_eq!(generator.price(), None);

    generator
        .generate(
            image_provider(&second),
            None,
            Mode::ObjectSwap,
            None,
            false,
            upload(&second),
        )
        .await;
    eventually(|| first_state(&generator) == Some(RunState::Finished)).await;
    assert_eq!(second.uploads.load(Ordering::SeqCst), 2);
    assert_eq!(
        second
            .submitted()
            .iter()
            .map(|request| request
                .input_images
                .iter()
                .filter_map(|asset| asset.id.clone())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        vec![vec!["B-media".to_owned()]]
    );
}

#[tokio::test]
async fn a_request_whose_price_cant_be_written_is_not_sent() {
    let folder = tempfile::tempdir().unwrap();
    let blocker = folder.path().join("file");
    std::fs::write(&blocker, "").unwrap();
    let provider = FakeImageProvider::default();
    let generator = ReferenceGenerator::new(
        Arc::new(ImageSpendLog::open(
            Some(&blocker),
            OffsetDateTime::now_utc(),
            time::UtcOffset::UTC,
        )),
        Arc::new(ImageJobLog::open(None)),
        no_cap(),
        yielding(),
    );
    generator.edit_form(|form| {
        form.use_models(vec![soul()], None);
        form.prompt = "a knight in silver armor".to_owned();
    });
    generate(&generator, &provider, soul().list_price).await;
    eventually(|| first_state(&generator) != Some(RunState::Running)).await;
    let Some(RunState::Failed(message)) = first_state(&generator) else {
        panic!("expected a failed run, got {:?}", first_state(&generator));
    };
    assert!(message.starts_with("Nothing was sent"));
    assert_eq!(provider.submitted().len(), 0);
}

/// Generate reads today's spend itself right before sending, so a screen can't skip the cap.
#[tokio::test]
async fn generate_asks_before_going_past_the_cap_and_sends_once_agreed() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    let committed: Arc<Mutex<Decimal>> = Arc::new(Mutex::new(Decimal::ZERO));
    let read = committed.clone();
    let status: SpendStatusReader = Arc::new(move |kind| {
        let committed = *read.lock().unwrap();
        Box::pin(async move {
            Ok((kind == ProviderKind::ApiKey).then(|| SpendStatus {
                committed_today: committed,
                cap: Some(SpendCap::usd(Decimal::ONE)),
            }))
        })
    });
    let generator = ReferenceGenerator::new(setup.spend.clone(), setup.jobs.clone(), status, yielding());
    generator.edit_form(|form| {
        form.use_models(vec![soul()], None);
        form.prompt = "a knight in silver armor".to_owned();
    });
    let price = usd("0.08", false);
    let send = |price: Option<Cost>, approved: bool| {
        let generator = generator.clone();
        let provider = provider.clone();
        async move {
            generator
                .generate(
                    image_provider(&provider),
                    None,
                    Mode::MotionTransfer,
                    price,
                    approved,
                    upload(&provider),
                )
                .await
        }
    };

    // Under the cap: sent.
    assert_eq!(send(Some(price.clone()), false).await, PaidStart::Started);
    eventually(|| first_state(&generator) == Some(RunState::Finished)).await;

    // A price not known yet, under a cap: asks, sends nothing.
    let PaidStart::NeedsConfirmation(unknown) = send(None, false).await else {
        panic!("expected a question about the unknown price");
    };
    assert_eq!(unknown.title, "Price not known yet");

    // Over the cap: asks, sends nothing.
    *committed.lock().unwrap() = "0.95".parse().unwrap();
    let PaidStart::NeedsConfirmation(over) = send(Some(price.clone()), false).await else {
        panic!("expected a question about the cap");
    };
    assert_eq!(over.title, "Over today's spend cap");
    assert_eq!(over.confirm_title, "Generate anyway");
    assert_eq!(provider.submitted().len(), 1);
    assert_eq!(setup.spend.charges().len(), 1);

    // Agreed: sent without asking again.
    assert_eq!(send(Some(price.clone()), true).await, PaidStart::Started);
    eventually(|| provider.submitted().len() == 2 && first_state(&generator) == Some(RunState::Finished))
        .await;
    assert_eq!(
        setup
            .spend
            .charges()
            .iter()
            .map(|charge| charge.cost.clone())
            .collect::<Vec<_>>(),
        vec![price.clone(), price]
    );
    assert_eq!(
        setup
            .spend
            .committed(OffsetDateTime::now_utc(), CostUnit::Usd, time::UtcOffset::UTC),
        Ok("0.16".parse().unwrap())
    );
}

/// `ReferenceGeneratorTests.aRestoredFormQuotesTheSamePrice`: the panel restored from the sheet's form quotes the
/// same request, without the sheet's source.
#[tokio::test]
async fn a_restored_form_quotes_the_same_price() {
    let setup = Setup::memory();
    let provider = FakeImageProvider::default();
    let sheet = setup.generator();
    sheet.edit_form(|form| form.count = 4);
    sheet
        .refresh_price(
            &image_provider(&provider),
            Some(LANDSCAPE),
            Mode::MotionTransfer,
            &upload(&provider),
        )
        .await;

    let panel = setup.generator();
    let draft = sheet.edit_form(|form| form.snapshot());
    panel.edit_form(|form| form.restore(&draft));
    panel
        .refresh_price(
            &image_provider(&provider),
            None,
            Mode::MotionTransfer,
            &upload(&provider),
        )
        .await;

    assert_eq!(panel.price(), sheet.price());
    assert_eq!(panel.price(), Some(usd("0.0128", false)));
}
