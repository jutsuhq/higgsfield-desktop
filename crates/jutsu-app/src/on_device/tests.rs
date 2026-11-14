//! `OnDeviceRunsTests.swift`, with a fake engine and a fake catalog.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use jutsu_core::{
    MaterialReference, OnDeviceAction, OnDeviceEngine, OnDeviceEnhancing, OnDeviceOutput, OnDeviceProgress,
    OnDeviceUpscaleTarget, PixelSize, PortError, Progress, SmoothMotion,
};
use tokio::sync::Notify;
use url::Url;
use uuid::Uuid;

use super::runs::{
    OnDeviceModelConsent, OnDeviceOutcome, OnDeviceRequest, OnDeviceRunState, OnDeviceRuns, Register,
};
use super::text::{self, OnDeviceOffer};
use crate::preferences::PreferencesStore;

#[derive(Default)]
struct FakeEngine {
    running: AtomicUsize,
    most_at_once: AtomicUsize,
    started: Mutex<Vec<PathBuf>>,
    hold: AtomicBool,
    release: Notify,
    fails: AtomicBool,
    model_ready: AtomicBool,
}

impl FakeEngine {
    fn held() -> Arc<Self> {
        let engine = Self::default();
        engine.hold.store(true, Ordering::SeqCst);
        Arc::new(engine)
    }

    fn release(&self) {
        self.hold.store(false, Ordering::SeqCst);
        self.release.notify_waiters();
    }

    fn started(&self) -> Vec<PathBuf> {
        self.started
            .lock()
            .map(|started| started.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl OnDeviceEnhancing for FakeEngine {
    fn supports(&self, _engine: OnDeviceEngine) -> bool {
        true
    }

    fn model_is_ready(&self, _engine: OnDeviceEngine) -> bool {
        self.model_ready.load(Ordering::SeqCst)
    }

    async fn run(
        &self,
        _action: &OnDeviceAction,
        source: &Url,
        output: &Url,
        progress: Progress<OnDeviceProgress>,
    ) -> Result<OnDeviceOutput, PortError> {
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_at_once.fetch_max(now, Ordering::SeqCst);
        if let Ok(mut started) = self.started.lock() {
            started.push(source.to_file_path().unwrap_or_default());
        }
        let output = output.to_file_path().unwrap_or_default();
        std::fs::write(&output, b"result").map_err(|error| PortError::Other(error.to_string()))?;
        progress(OnDeviceProgress::Working(0.42));
        while self.hold.load(Ordering::SeqCst) {
            let released = self.release.notified();
            if !self.hold.load(Ordering::SeqCst) {
                break;
            }
            released.await;
        }
        self.running.fetch_sub(1, Ordering::SeqCst);
        if self.fails.load(Ordering::SeqCst) {
            return Err(PortError::Other("The encoder stopped.".to_owned()));
        }
        Ok(OnDeviceOutput {
            pixel_size: PixelSize::new(854, 480),
            duration: 5.0,
            file_extension: "mp4".to_owned(),
        })
    }
}

/// The catalog: the versions added, or a refusal.
#[derive(Default)]
struct Catalog {
    added: Mutex<Vec<(MaterialReference, PathBuf)>>,
    refuses: AtomicBool,
}

struct Fixture {
    folder: tempfile::TempDir,
    engine: Arc<FakeEngine>,
    catalog: Arc<Catalog>,
    runs: Arc<OnDeviceRuns>,
    preferences: Arc<PreferencesStore>,
}

impl Fixture {
    fn new(engine: Arc<FakeEngine>) -> Self {
        let folder = tempfile::tempdir().unwrap();
        let preferences = Arc::new(PreferencesStore::open(folder.path()));
        let catalog = Arc::new(Catalog::default());
        let added = catalog.clone();
        let register: Register = Arc::new(move |request: OnDeviceRequest, file: PathBuf, _output| {
            let added = added.clone();
            Box::pin(async move {
                if added.refuses.load(Ordering::SeqCst) {
                    return Err("This result is no longer in the library.".to_owned());
                }
                added.added.lock().unwrap().push((request.source, file));
                Ok(())
            })
        });
        let runs = OnDeviceRuns::new(
            engine.clone(),
            OnDeviceModelConsent::new(preferences.clone()),
            register,
        );
        Self {
            folder,
            engine,
            catalog,
            runs,
            preferences,
        }
    }

    fn videos(&self) -> PathBuf {
        self.folder.path().join("Videos")
    }

    fn request(&self, name: &str, action: OnDeviceAction) -> OnDeviceRequest {
        let file = self.folder.path().join(format!("{name}.mp4"));
        std::fs::write(&file, b"source").unwrap();
        OnDeviceRequest {
            source: MaterialReference::new(Uuid::new_v4(), Uuid::new_v4()),
            file,
            action,
            destination: self.videos(),
            scratch: self.videos(),
        }
    }

    fn added(&self) -> Vec<(MaterialReference, PathBuf)> {
        self.catalog.added.lock().unwrap().clone()
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.videos())
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

const SMOOTH: OnDeviceAction = OnDeviceAction::SmoothMotion(SmoothMotion::Fps60);

async fn until(condition: impl Fn() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!("the condition never held");
}

#[tokio::test]
async fn runs_one_at_a_time_and_adds_each_result_as_a_new_version() {
    let fixture = Fixture::new(FakeEngine::held());
    let first = fixture.request("rooftop", SMOOTH);
    let second = fixture.request("harbor", OnDeviceAction::SmoothMotion(SmoothMotion::SlowMotion2x));
    assert!(fixture.runs.start(first.clone(), Uuid::new_v4()));
    assert!(fixture.runs.start(second.clone(), Uuid::new_v4()));
    until(|| fixture.runs.runs().first().map(|run| run.state) == Some(OnDeviceRunState::Working(0.42))).await;
    assert_eq!(fixture.runs.runs()[1].state, OnDeviceRunState::Waiting);
    assert_eq!(
        text::progress(&fixture.runs.runs()[0]),
        "Smooth Motion to 60 fps · 42%"
    );
    fixture.engine.release();
    fixture.runs.finish().await;
    assert_eq!(fixture.engine.most_at_once.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.engine.started(),
        vec![first.file.clone(), second.file.clone()]
    );
    let added = fixture.added();
    assert_eq!(added.len(), 2);
    assert_eq!(added[0].0, first.source);
    assert_eq!(added[0].1, fixture.videos().join("rooftop 60fps.mp4"));
    assert_eq!(added[1].0, second.source);
    assert_eq!(fixture.files(), vec!["harbor slow 2x.mp4", "rooftop 60fps.mp4"]);
    assert!(fixture.runs.runs().is_empty(), "{:?}", fixture.runs.runs());
}

#[tokio::test]
async fn a_second_run_on_the_same_item_is_refused() {
    let fixture = Fixture::new(FakeEngine::held());
    let request = fixture.request("rooftop", SMOOTH);
    assert!(fixture.runs.start(request.clone(), Uuid::new_v4()));
    let mut again = request.clone();
    again.action = OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full);
    fixture.runs.agree(OnDeviceEngine::VideoSuperResolution);
    assert!(!fixture.runs.start(again, Uuid::new_v4()));
    let running = fixture.runs.run_for(request.source.material_id).unwrap();
    assert_eq!(
        text::busy(&running),
        "Wait until Smooth Motion to 60 fps finishes"
    );
    fixture.engine.release();
    fixture.runs.finish().await;
}

#[test]
fn leftovers_of_earlier_runs_go_at_launch_and_nothing_else() {
    let folder = tempfile::tempdir().unwrap();
    let leftover = folder.path().join(OnDeviceRuns::partial_name(Uuid::new_v4()));
    let job_scratch = folder.path().join(format!(".{}.partial", Uuid::new_v4()));
    let result = folder.path().join("rooftop 60fps.mp4");
    for file in [&leftover, &job_scratch, &result] {
        std::fs::write(file, b"x").unwrap();
    }
    OnDeviceRuns::remove_leftovers(folder.path());
    assert!(!leftover.exists());
    assert!(job_scratch.exists());
    assert!(result.exists());
}

#[tokio::test]
async fn canceling_the_running_run_leaves_no_file() {
    let fixture = Fixture::new(FakeEngine::held());
    let id = Uuid::new_v4();
    assert!(fixture.runs.start(fixture.request("rooftop", SMOOTH), id));
    until(|| fixture.engine.running.load(Ordering::SeqCst) == 1).await;
    fixture.runs.cancel(id);
    fixture.runs.finish().await;
    assert!(fixture.added().is_empty(), "{:?}", fixture.added());
    assert!(fixture.files().is_empty(), "{:?}", fixture.files());
    let (_, finished) = fixture.runs.run(id).unwrap();
    assert_eq!(
        finished.map(|finished| finished.outcome),
        Some(OnDeviceOutcome::Canceled)
    );
    assert!(fixture.runs.failure().is_none());
}

#[tokio::test]
async fn canceling_a_waiting_run_drops_it() {
    let fixture = Fixture::new(FakeEngine::held());
    let (first, second) = (Uuid::new_v4(), Uuid::new_v4());
    assert!(fixture.runs.start(fixture.request("rooftop", SMOOTH), first));
    assert!(fixture.runs.start(fixture.request("harbor", SMOOTH), second));
    fixture.runs.cancel(second);
    assert_eq!(
        fixture.runs.runs().iter().map(|run| run.id).collect::<Vec<_>>(),
        vec![first]
    );
    fixture.engine.release();
    fixture.runs.finish().await;
    assert_eq!(fixture.engine.started().len(), 1);
    assert_eq!(fixture.added().len(), 1);
}

#[tokio::test]
async fn a_failed_run_is_reported_and_leaves_no_file() {
    let engine = Arc::new(FakeEngine::default());
    engine.fails.store(true, Ordering::SeqCst);
    let fixture = Fixture::new(engine);
    let id = Uuid::new_v4();
    assert!(fixture.runs.start(fixture.request("rooftop", SMOOTH), id));
    fixture.runs.finish().await;
    let failure = fixture.runs.take_failure().unwrap();
    assert_eq!(failure.title, "Can't finish Smooth Motion to 60 fps");
    assert_eq!(failure.message, "The encoder stopped.");
    assert!(fixture.runs.take_failure().is_none());
    assert!(fixture.files().is_empty(), "{:?}", fixture.files());
    assert!(matches!(
        fixture
            .runs
            .run(id)
            .and_then(|(_, finished)| finished)
            .map(|finished| finished.outcome),
        Some(OnDeviceOutcome::Failed(_))
    ));
}

#[tokio::test]
async fn a_result_the_catalog_refuses_is_taken_back() {
    let fixture = Fixture::new(Arc::new(FakeEngine::default()));
    fixture.catalog.refuses.store(true, Ordering::SeqCst);
    assert!(
        fixture
            .runs
            .start(fixture.request("rooftop", SMOOTH), Uuid::new_v4())
    );
    fixture.runs.finish().await;
    assert!(fixture.files().is_empty(), "{:?}", fixture.files());
    assert_eq!(
        fixture.runs.failure().map(|failure| failure.message),
        Some("This result is no longer in the library.".to_owned())
    );
}

#[tokio::test]
async fn an_upscale_waits_for_consent_to_the_model_download() {
    let engine = Arc::new(FakeEngine::default());
    engine.model_ready.store(true, Ordering::SeqCst);
    let fixture = Fixture::new(engine);
    let upscale = OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full);
    let engine = upscale.engine();
    assert!(fixture.runs.needs_consent(engine));
    assert!(!fixture.runs.needs_consent(OnDeviceEngine::FrameRateConversion));
    assert!(
        !fixture
            .runs
            .start(fixture.request("rooftop", upscale), Uuid::new_v4())
    );
    assert!(
        fixture.engine.started().is_empty(),
        "{:?}",
        fixture.engine.started()
    );
    assert!(!fixture.runs.model_is_ready(engine));
    fixture.runs.agree(engine);
    assert_eq!(
        fixture.preferences.value(&OnDeviceModelConsent::key(engine)),
        Some(serde_json::Value::Bool(true))
    );
    assert!(!fixture.runs.needs_consent(engine));
    assert!(
        fixture
            .runs
            .start(fixture.request("rooftop", upscale), Uuid::new_v4())
    );
    fixture.runs.finish().await;
    assert_eq!(fixture.engine.started().len(), 1);
    assert!(fixture.runs.model_is_ready(engine));
}

#[test]
fn only_an_image_up_to_1080p_is_offered_an_upscale() {
    let all = |_: OnDeviceEngine| true;
    assert!(OnDeviceOffer::upscales_image(
        Some(PixelSize::new(1920, 1080)),
        all
    ));
    assert!(OnDeviceOffer::upscales_image(
        Some(PixelSize::new(1080, 1920)),
        all
    ));
    assert!(!OnDeviceOffer::upscales_image(
        Some(PixelSize::new(2048, 2048)),
        all
    ));
    assert!(!OnDeviceOffer::upscales_image(None, all));
    assert!(!OnDeviceOffer::upscales_image(
        Some(PixelSize::new(512, 512)),
        |_| false
    ));
}

#[test]
fn progress_reads_as_the_action_and_its_percent() {
    let run = |action, state| super::runs::OnDeviceRun {
        id: Uuid::new_v4(),
        request: OnDeviceRequest {
            source: MaterialReference::new(Uuid::new_v4(), Uuid::new_v4()),
            file: Path::new("/tmp/rooftop.mp4").to_path_buf(),
            action,
            destination: PathBuf::new(),
            scratch: PathBuf::new(),
        },
        state,
        created_at: time::OffsetDateTime::UNIX_EPOCH,
    };
    let upscale = OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full);
    assert_eq!(
        text::progress(&run(SMOOTH, OnDeviceRunState::Waiting)),
        "Smooth Motion to 60 fps · Waiting for the run before it"
    );
    assert_eq!(
        text::progress(&run(upscale, OnDeviceRunState::DownloadingModel(0.125))),
        "Downloading the model · 13%"
    );
    assert_eq!(
        text::progress(&run(upscale, OnDeviceRunState::Working(0.5))),
        "Upscale 4× · 50%"
    );
    assert_eq!(
        text::upscale_sizes(PixelSize::new(854, 480)),
        "4× · 854×480 → 3416×1920"
    );
    assert_eq!(text::expected_time(130.0), "Takes ~2 min on this computer");
}

mod agent {
    use jutsu_agent::{AgentBudgetUnit, AgentEnhancement, EnhanceCommand, ErrorCode};

    use super::*;
    use crate::on_device::agent::{
        AgentOnDeviceMedia, AgentOnDeviceRuns, AgentOnDeviceTarget, action, refusal,
    };

    fn command(operation: AgentEnhancement, mode: Option<&str>, engine: Option<&str>) -> EnhanceCommand {
        let mut fields: EnhanceCommand = serde_json::from_value(serde_json::json!({
            "requestId": "req-1",
            "fingerprint": "f",
            "item": Uuid::new_v4(),
            "operation": operation.as_str(),
        }))
        .unwrap();
        fields.mode = mode.map(str::to_owned);
        fields.engine = engine.map(str::to_owned);
        fields
    }

    fn video(fixture: &Fixture, width: i64, height: i64) -> AgentOnDeviceTarget {
        let request = fixture.request("rooftop", SMOOTH);
        AgentOnDeviceTarget {
            source: request.source,
            file: request.file,
            media: AgentOnDeviceMedia::Video {
                size: PixelSize::new(width, height),
                frame_rate: 24.0,
                duration: 5.0,
            },
            destination: request.destination,
            scratch: request.scratch,
        }
    }

    #[test]
    fn the_fields_name_the_run_and_its_mode() {
        let smooth = command(AgentEnhancement::SmoothMotion, Some("slow_2x"), None);
        let media = AgentOnDeviceMedia::Video {
            size: PixelSize::new(854, 480),
            frame_rate: 24.0,
            duration: 5.0,
        };
        assert_eq!(
            action(&smooth, media).unwrap(),
            OnDeviceAction::SmoothMotion(SmoothMotion::SlowMotion2x)
        );
        let missing = command(AgentEnhancement::SmoothMotion, None, None);
        assert_eq!(action(&missing, media).unwrap_err().code, ErrorCode::BAD_REQUEST);
        let on_image = action(&smooth, AgentOnDeviceMedia::Image(Some(PixelSize::new(512, 512))));
        assert_eq!(on_image.unwrap_err().code, ErrorCode::INVALID_REQUEST);
        let upscale = command(AgentEnhancement::Upscale, None, Some("mac"));
        assert_eq!(
            action(&upscale, AgentOnDeviceMedia::Image(None)).unwrap(),
            OnDeviceAction::UpscaleImage(OnDeviceUpscaleTarget::Full)
        );
    }

    #[tokio::test]
    async fn refusals_come_in_the_order_an_agent_can_act_on() {
        let fixture = Fixture::new(FakeEngine::held());
        let upscale = OnDeviceAction::UpscaleVideo(OnDeviceUpscaleTarget::Full);
        let large = video(&fixture, 1920, 1080);
        assert_eq!(
            refusal(upscale, &large, &fixture.runs).map(|error| error.code),
            Some(ErrorCode::INVALID_REQUEST)
        );
        let small = video(&fixture, 854, 480);
        let consent = refusal(upscale, &small, &fixture.runs).unwrap();
        assert_eq!(consent.code, ErrorCode::NEEDS_PERSON);
        assert!(consent.message.contains("Real-ESRGAN"), "{}", consent.message);
        assert!(consent.next.unwrap_or_default().contains("upscale_on_mac"));
        let request = OnDeviceRequest {
            source: small.source,
            file: small.file.clone(),
            action: SMOOTH,
            destination: small.destination.clone(),
            scratch: small.scratch.clone(),
        };
        assert!(fixture.runs.start(request, Uuid::new_v4()));
        // A run already going on the item comes before the person's agreement.
        assert_eq!(
            refusal(upscale, &small, &fixture.runs).map(|error| error.code),
            Some(ErrorCode::BUSY)
        );
        fixture.engine.release();
        fixture.runs.finish().await;
    }

    #[tokio::test]
    async fn an_agent_s_run_is_followed_and_cancelled_but_not_the_person_s() {
        let fixture = Fixture::new(FakeEngine::held());
        let agents = AgentOnDeviceRuns::new(fixture.runs.clone());
        let target = video(&fixture, 854, 480);
        let fields = command(AgentEnhancement::SmoothMotion, Some("60fps"), None);
        let mut recorded = Vec::new();
        let job = agents
            .start(
                &fields,
                Some("claude".to_owned()),
                &target,
                AgentBudgetUnit::Credits,
                |ids| {
                    recorded.extend_from_slice(ids);
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(job.kind, "on_device");
        assert_eq!(job.mode.as_deref(), Some("60fps"));
        assert_eq!(job.operation.as_deref(), Some("smooth_motion"));
        assert!(job.estimated_seconds.is_some());
        assert_eq!(agents.started().len(), 1);
        assert!(!agents.is_settled(recorded[0]));

        let person = fixture.request("harbor", SMOOTH);
        let theirs = Uuid::new_v4();
        assert!(fixture.runs.start(person, theirs));
        let refused = agents.cancel(theirs, AgentBudgetUnit::Credits).await.unwrap_err();
        assert_eq!(refused.code, ErrorCode::NEEDS_PERSON);

        let canceled = agents
            .cancel(recorded[0], AgentBudgetUnit::Credits)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(canceled.state, "canceled");
        assert!(agents.is_settled(recorded[0]));
        fixture.engine.release();
        fixture.runs.finish().await;
    }

    #[test]
    fn capabilities_say_the_upscale_needs_consent_until_agreed() {
        let fixture = Fixture::new(Arc::new(FakeEngine::default()));
        let agents = AgentOnDeviceRuns::new(fixture.runs.clone());
        let before = agents.capabilities();
        assert_eq!(before.smooth_motion.model, "none");
        assert_eq!(before.upscale_video.model, "needs_consent");
        assert_eq!(before.upscale_video.max_long_side, Some(1280));
        assert_eq!(before.upscale_image.max_long_side, Some(1920));
        fixture.runs.agree(OnDeviceEngine::VideoSuperResolution);
        assert_eq!(agents.capabilities().upscale_video.model, "agreed");
    }
}
