//! `CharacterBuilderTests`: Create Character against a scripted character sheet maker.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use jutsu_core::{
    CharacterPresetPage, CharacterPresetSettings, CharacterSheetPage, CharacterTrait, Cost, CostUnit,
    HistoryCursor, PreparedFile, Rejection, SpendCap, SpendStatus,
};
use rust_decimal::Decimal;

use super::*;
use crate::testing::{Gate, eventually};

/// A character sheet maker whose answers the test scripts: `statuses` are handed out in order, the last one repeated.
struct FakeMaker {
    credits: Decimal,
    script: Mutex<Script>,
    designs: Mutex<Vec<CharacterDesign>>,
    submitted: Mutex<Vec<CharacterQuote>>,
    uploads: AtomicUsize,
    polls: AtomicUsize,
}

struct Script {
    quote_error: Option<ProviderError>,
    submit_error: Option<ProviderError>,
    statuses: Vec<BTreeMap<String, ImageJobStatus>>,
    submit_gate: Option<Arc<Gate<()>>>,
    poll_error: Option<ProviderError>,
}

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

fn statuses(pairs: &[(&str, ImageJobStatus)]) -> BTreeMap<String, ImageJobStatus> {
    pairs
        .iter()
        .map(|(id, status)| ((*id).to_owned(), status.clone()))
        .collect()
}

impl FakeMaker {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            credits: "23.2".parse().unwrap(),
            script: Mutex::new(Script {
                quote_error: None,
                submit_error: None,
                statuses: vec![
                    statuses(&[
                        (
                            "job-1",
                            ImageJobStatus::Completed(vec![url("https://cdn.example.com/1.png")]),
                        ),
                        ("job-2", ImageJobStatus::Pending),
                    ]),
                    statuses(&[(
                        "job-2",
                        ImageJobStatus::Completed(vec![url("https://cdn.example.com/2.png")]),
                    )]),
                ],
                submit_gate: None,
                poll_error: None,
            }),
            designs: Mutex::new(Vec::new()),
            submitted: Mutex::new(Vec::new()),
            uploads: AtomicUsize::new(0),
            polls: AtomicUsize::new(0),
        })
    }

    fn script(&self, change: impl FnOnce(&mut Script)) {
        change(&mut self.script.lock().unwrap());
    }

    fn submitted(&self) -> Vec<CharacterQuote> {
        self.submitted.lock().unwrap().clone()
    }

    fn designs(&self) -> Vec<CharacterDesign> {
        self.designs.lock().unwrap().clone()
    }
}

#[async_trait]
impl CharacterSheetMaker for FakeMaker {
    fn owner(&self) -> Option<String> {
        Some("user_1".to_owned())
    }

    async fn character_traits(&self, _tier: CharacterTier) -> Result<Vec<CharacterTraitGroup>, PortError> {
        let traits = [("curly", "Curly"), ("bald", "Bald"), ("long", "Long")]
            .map(|(id, title)| CharacterTrait {
                id: id.to_owned(),
                title: title.to_owned(),
            })
            .to_vec();
        Ok(vec![CharacterTraitGroup::new("hair", "Hair", 2, traits)])
    }

    async fn character_presets(
        &self,
        _after: Option<HistoryCursor>,
    ) -> Result<CharacterPresetPage, PortError> {
        Ok(CharacterPresetPage {
            items: Vec::new(),
            next_cursor: None,
        })
    }

    async fn character_sheets(&self, _after: Option<HistoryCursor>) -> Result<CharacterSheetPage, PortError> {
        Err(ProviderError::Validation {
            field: None,
            message: "Not available for this account".to_owned(),
        }
        .into())
    }

    async fn upload(&self, _file: PreparedFile) -> Result<RemoteAsset, PortError> {
        let count = self.uploads.fetch_add(1, Ordering::SeqCst) + 1;
        let mut asset = RemoteAsset::new(url("https://cdn.example.com/in.png"));
        asset.id = Some(format!("media-{count}"));
        Ok(asset)
    }

    async fn quote_characters(&self, design: CharacterDesign) -> Result<CharacterQuote, PortError> {
        let count = design.count;
        let number = {
            let mut designs = self.designs.lock().unwrap();
            designs.push(design);
            designs.len()
        };
        if let Some(error) = self.script.lock().unwrap().quote_error.clone() {
            return Err(error.into());
        }
        Ok(CharacterQuote {
            credits: self.credits,
            count,
            request: format!("q{number}").into_bytes(),
        })
    }

    async fn generate_characters(&self, quote: CharacterQuote) -> Result<Vec<RemoteJob>, PortError> {
        self.submitted.lock().unwrap().push(quote);
        let gate = self.script.lock().unwrap().submit_gate.clone();
        if let Some(gate) = gate {
            gate.wait().await;
        }
        if let Some(error) = self.script.lock().unwrap().submit_error.clone() {
            return Err(error.into());
        }
        Ok(vec![RemoteJob::new("job-1"), RemoteJob::new("job-2")])
    }

    async fn character_sheet_statuses(
        &self,
        _jobs: Vec<RemoteJob>,
    ) -> Result<BTreeMap<String, ImageJobStatus>, PortError> {
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
}

fn face() -> LocalImage {
    LocalImage::new(url("file:///tmp/face.png"))
}

fn style() -> LocalImage {
    LocalImage::new(url("file:///tmp/style.png"))
}

fn yielding() -> SleepFn {
    Arc::new(|_| Box::pin(tokio::task::yield_now()))
}

fn no_cap() -> SpendStatusReader {
    Arc::new(|_| Box::pin(async { Ok(None) }))
}

fn memory_spend() -> Arc<ImageSpendLog> {
    Arc::new(ImageSpendLog::open(
        None,
        OffsetDateTime::now_utc(),
        time::UtcOffset::UTC,
    ))
}

struct Setup {
    spend: Arc<ImageSpendLog>,
    log: Arc<CharacterSheetLog>,
}

impl Setup {
    fn new() -> Self {
        Self {
            spend: memory_spend(),
            log: Arc::new(CharacterSheetLog::open(None)),
        }
    }

    fn builder(&self) -> Arc<CharacterBuilder> {
        CharacterBuilder::new(self.log.clone(), self.spend.clone(), no_cap(), yielding())
    }

    fn ids(&self, owner: &str) -> Vec<String> {
        self.log
            .pending(Some(owner))
            .into_iter()
            .map(|record| record.id)
            .collect()
    }
}

/// Uploads through the fake maker, as the app uploads through the provider.
fn upload(maker: &Arc<FakeMaker>) -> UploadFn {
    let maker = maker.clone();
    Arc::new(move |_| {
        let maker = maker.clone();
        Box::pin(async move {
            maker
                .upload(PreparedFile {
                    file_url: url("file:///tmp/x.png"),
                    content_type: "image/png".to_owned(),
                    byte_count: 1,
                    content_hash: "x".to_owned(),
                })
                .await
        })
    })
}

fn as_maker(maker: &Arc<FakeMaker>) -> Arc<dyn CharacterSheetMaker> {
    maker.clone()
}

fn first_state(builder: &CharacterBuilder) -> Option<CharacterRunState> {
    builder.runs().first().map(|run| run.state.clone())
}

fn credits(text: &str) -> Cost {
    Cost::credits(text.parse().unwrap(), false)
}

#[tokio::test]
async fn traits_keep_to_the_groups_limit_and_a_tier_change_drops_them() {
    let setup = Setup::new();
    let builder = setup.builder();
    builder.load_traits(FakeMaker::new().as_ref()).await;
    let group = builder.trait_groups()[0].clone();

    for item in &group.traits {
        builder.toggle(&group.id, &item.id);
    }
    assert_eq!(
        builder.form().traits,
        BTreeMap::from([("hair".to_owned(), vec!["bald".to_owned(), "long".to_owned()])])
    );
    builder.toggle(&group.id, &group.traits[1].id);
    assert_eq!(
        builder.form().traits,
        BTreeMap::from([("hair".to_owned(), vec!["long".to_owned()])])
    );

    builder.select_kind(CharacterTierKind::Animal);
    assert_eq!(builder.form().tier, CharacterTier::Insects);
    assert!(builder.form().traits.is_empty());
}

#[test]
fn a_preset_fills_the_form_and_keeps_its_design() {
    let setup = Setup::new();
    let builder = setup.builder();
    let preset = CharacterPreset {
        id: "p".to_owned(),
        name: "Chef".to_owned(),
        preview_url: None,
        settings: Some(CharacterPresetSettings {
            tier: CharacterTier::Cats,
            traits: BTreeMap::from([("fur".to_owned(), vec!["ginger".to_owned()])]),
            brief: "a chef".to_owned(),
            seed: Some(12),
        }),
        unavailable_reason: None,
    };

    builder.apply_preset(&preset);

    let form = builder.form();
    assert_eq!(form.tier, CharacterTier::Cats);
    assert_eq!(form.tier.kind(), CharacterTierKind::Animal);
    assert_eq!(
        form.traits,
        BTreeMap::from([("fur".to_owned(), vec!["ginger".to_owned()])])
    );
    assert_eq!(form.brief, "a chef");
    assert_eq!(form.seed, Some(12));
    assert!(!form.randomize);
    builder.clear_preset();
    assert_eq!(builder.form().seed, None);
    assert_eq!(builder.form().preset, None);
}

#[tokio::test]
async fn the_quote_uploads_the_images_once_and_belongs_to_the_form_it_priced() {
    let maker = FakeMaker::new();
    let setup = Setup::new();
    let builder = setup.builder();
    builder.edit_form(|form| {
        form.face = Some(face());
        form.count = 2;
    });
    builder.add_styles(vec![style(), style(), style(), style()]);
    assert_eq!(builder.form().styles.len(), 3);

    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;
    assert_eq!(
        builder.current_quote().map(|quote| quote.credits),
        Some("23.2".parse().unwrap())
    );
    assert_eq!(
        builder.current_quote().map(|quote| quote.cost()),
        Some(credits("23.2"))
    );
    builder.edit_form(|form| form.brief = "a magician".to_owned());
    assert_eq!(builder.current_quote(), None);
    assert!(builder.is_quoting());
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;

    let designs = maker.designs();
    assert_eq!(designs.len(), 2);
    assert_eq!(designs[1].brief, "a magician");
    assert_eq!(designs[1].count, 2);
    assert!(
        designs[1]
            .face
            .as_ref()
            .and_then(|face| face.id.clone())
            .is_some()
    );
    assert_eq!(designs[1].styles.len(), 3);
    // Two distinct files: the face and the style, each uploaded once.
    assert_eq!(maker.uploads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_failed_quote_says_why_and_generate_stays_off() {
    let maker = FakeMaker::new();
    maker.script(|script| {
        script.quote_error = Some(ProviderError::Validation {
            field: None,
            message: "Unknown trait".to_owned(),
        });
    });
    let setup = Setup::new();
    let builder = setup.builder();

    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;

    assert_eq!(builder.quote_failure().as_deref(), Some("Unknown trait"));
    assert!(!builder.is_quoting());
    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::NotStarted
    );
    assert_eq!(maker.submitted(), []);
}

#[tokio::test]
async fn generate_sends_the_quote_once_tracks_every_job_and_saves_the_sheets() {
    let maker = FakeMaker::new();
    let gate = Gate::new();
    maker.script(|script| script.submit_gate = Some(gate.clone()));
    let setup = Setup::new();
    let builder = setup.builder();
    let saved: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = saved.clone();
    builder.set_on_sheets(Arc::new(move |records| {
        sink.lock()
            .unwrap()
            .extend(records.into_iter().map(|record| record.id));
    }));
    builder.edit_form(|form| form.brief = "a magician".to_owned());
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;
    let quote = builder.current_quote().unwrap();

    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::Started
    );
    // A second click while the submit is out sends nothing.
    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::NotStarted
    );
    gate.open(());
    let charges = setup.spend.charges();
    assert_eq!(
        charges
            .iter()
            .map(|charge| charge.cost.clone())
            .collect::<Vec<_>>(),
        vec![credits("23.2")]
    );
    assert_eq!(charges[0].purpose, Some(ImageChargePurpose::CharacterSheets));
    eventually(|| first_state(&builder) == Some(CharacterRunState::Finished)).await;

    assert_eq!(maker.submitted(), vec![quote]);
    let records = setup.log.records();
    assert_eq!(
        records
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        vec!["job-1", "job-2"]
    );
    assert!(
        records
            .iter()
            .all(|record| record.state == CharacterSheetState::Completed {}
                && record.owner.as_deref() == Some("user_1")
                && record.brief == "a magician")
    );
    assert_eq!(*saved.lock().unwrap(), vec!["job-1", "job-2"]);
    assert_eq!(
        setup
            .log
            .sheets(Some("user_1"))
            .into_iter()
            .map(|sheet| sheet.id)
            .collect::<Vec<_>>(),
        vec!["job-2", "job-1"]
    );
    assert_eq!(setup.log.sheets(Some("user_2")), []);
    assert!(!setup.spend.charges()[0].is_pending);
    // The quote was used: the next Generate is quoted again.
    assert_eq!(builder.current_quote(), None);
}

#[tokio::test]
async fn a_submit_with_an_unknown_outcome_is_never_resent_and_keeps_its_price() {
    let maker = FakeMaker::new();
    maker.script(|script| script.submit_error = Some(ProviderError::OutcomeUnknown));
    let setup = Setup::new();
    let builder = setup.builder();
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;

    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::Started
    );
    eventually(|| first_state(&builder) != Some(CharacterRunState::Sending)).await;

    assert_eq!(
        first_state(&builder),
        Some(CharacterRunState::Failed(MAY_HAVE_STARTED.to_owned()))
    );
    assert_eq!(maker.submitted().len(), 1);
    assert_eq!(setup.spend.charges().len(), 1);
    assert!(!setup.spend.charges()[0].is_pending);
    assert_eq!(setup.log.records(), []);
    assert!(!builder.is_submitting());
}

#[tokio::test]
async fn a_refused_submit_is_not_charged() {
    let maker = FakeMaker::new();
    maker.script(|script| {
        script.submit_error = Some(ProviderError::Validation {
            field: None,
            message: "Price is above max_credits".to_owned(),
        });
    });
    let setup = Setup::new();
    let builder = setup.builder();
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;

    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::Started
    );
    eventually(|| first_state(&builder) != Some(CharacterRunState::Sending)).await;

    assert_eq!(
        first_state(&builder),
        Some(CharacterRunState::Failed("Price is above max_credits".to_owned()))
    );
    assert_eq!(setup.spend.charges(), []);
}

#[tokio::test]
async fn sheets_that_all_fail_are_taken_off_the_spend() {
    let maker = FakeMaker::new();
    maker.script(|script| {
        script.statuses = vec![statuses(&[
            (
                "job-1",
                ImageJobStatus::Failed {
                    message: Some("Server error".to_owned()),
                },
            ),
            ("job-2", ImageJobStatus::Rejected(Rejection::ExplicitContent)),
        ])];
    });
    let setup = Setup::new();
    let builder = setup.builder();
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;

    builder.generate(as_maker(&maker), false).await;
    eventually(|| matches!(first_state(&builder), Some(CharacterRunState::Failed(_)))).await;

    assert_eq!(
        first_state(&builder),
        Some(CharacterRunState::Failed("Server error".to_owned()))
    );
    assert_eq!(setup.spend.charges(), []);
    assert_eq!(setup.log.sheets(Some("user_1")), []);
}

#[tokio::test]
async fn sheets_still_running_are_followed_again_after_a_relaunch() {
    let maker = FakeMaker::new();
    let setup = Setup::new();
    let request = Uuid::new_v4();
    let now = OffsetDateTime::now_utc();
    setup.log.add(vec![
        CharacterSheetRecord::new("job-1", Some("user_1"), request, now, ""),
        CharacterSheetRecord::new("job-2", Some("user_1"), request, now, ""),
        CharacterSheetRecord::new("job-9", Some("user_2"), Uuid::new_v4(), now, ""),
    ]);
    let builder = setup.builder();

    builder.resume(&as_maker(&maker));
    eventually(|| setup.log.pending(Some("user_1")).is_empty()).await;

    assert_eq!(setup.log.sheets(Some("user_1")).len(), 2);
    // Another account's sheet waits for that account.
    assert_eq!(setup.ids("user_2"), vec!["job-9"]);
}

#[tokio::test]
async fn quitting_can_wait_for_the_submit_already_sent_and_its_jobs_are_written_down() {
    let maker = FakeMaker::new();
    let gate = Gate::new();
    maker.script(|script| {
        script.statuses = vec![BTreeMap::new()];
        script.submit_gate = Some(gate.clone());
    });
    let setup = Setup::new();
    let builder = setup.builder();
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;

    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::Started
    );
    eventually(|| gate.is_waiting()).await;
    assert_eq!(builder.active_count(), 1);
    let waiting = builder.clone();
    let finished = tokio::spawn(async move { waiting.finish_submit().await });
    gate.open(());
    finished.await.unwrap();

    assert_eq!(setup.ids("user_1"), vec!["job-1", "job-2"]);
    assert!(!builder.is_submitting());
    builder.stop_following();
}

#[tokio::test]
async fn a_submit_whose_price_cant_be_written_is_not_sent() {
    // A regular file where the data folder should be: nothing can be written under it.
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("file");
    std::fs::write(&file, "").unwrap();
    let maker = FakeMaker::new();
    let log = Arc::new(CharacterSheetLog::open(None));
    let spend = Arc::new(ImageSpendLog::open(
        Some(&file),
        OffsetDateTime::now_utc(),
        time::UtcOffset::UTC,
    ));
    let builder = CharacterBuilder::new(log, spend, no_cap(), yielding());
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;

    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::NotStarted
    );

    let Some(CharacterRunState::Failed(message)) = first_state(&builder) else {
        panic!("expected a failed run, got {:?}", first_state(&builder));
    };
    assert!(message.starts_with("Nothing was sent"));
    assert_eq!(maker.submitted(), []);
    assert!(!builder.is_submitting());
}

#[test]
fn jobs_that_cant_be_written_are_still_followed_and_listed_to_copy() {
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("file");
    std::fs::write(&file, "").unwrap();
    let log = CharacterSheetLog::open(Some(&file));

    log.add(vec![CharacterSheetRecord::new(
        "job-1",
        Some("user_1"),
        Uuid::new_v4(),
        OffsetDateTime::now_utc(),
        "",
    )]);

    assert_eq!(
        log.pending(Some("user_1"))
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        vec!["job-1"]
    );
    assert_eq!(log.unsaved_ids(), vec!["job-1".to_owned()]);
    assert_eq!(
        log.save_error().map(|error| error.file),
        Some(CharacterSheetLog::FILE_NAME.to_owned())
    );
}

/// Generate reads today's spend itself right before sending, so a screen can't skip the cap.
#[tokio::test]
async fn generate_asks_before_going_past_the_cap_and_sends_once_agreed() {
    let maker = FakeMaker::new();
    let setup = Setup::new();
    let committed: Arc<Mutex<Decimal>> = Arc::new(Mutex::new(Decimal::ZERO));
    let read = committed.clone();
    let reader: SpendStatusReader = Arc::new(move |kind| {
        let committed = *read.lock().unwrap();
        Box::pin(async move {
            Ok((kind == ProviderKind::Account).then(|| SpendStatus {
                committed_today: committed,
                cap: Some(SpendCap::credits(30)),
            }))
        })
    });
    let builder = CharacterBuilder::new(setup.log.clone(), setup.spend.clone(), reader, yielding());

    // 10 + 23.2 credits goes past 30: asks, sends nothing.
    *committed.lock().unwrap() = Decimal::TEN;
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;
    let PaidStart::NeedsConfirmation(question) = builder.generate(as_maker(&maker), false).await else {
        panic!("expected a question about the cap");
    };
    assert_eq!(question.title, "Over today's spend cap");
    assert_eq!(maker.submitted(), []);
    assert_eq!(setup.spend.charges(), []);
    assert_eq!(builder.runs(), []);

    // Agreed: sent without asking again.
    assert_eq!(builder.generate(as_maker(&maker), true).await, PaidStart::Started);
    eventually(|| first_state(&builder) == Some(CharacterRunState::Finished)).await;
    assert_eq!(maker.submitted().len(), 1);

    // Under the cap: sent.
    *committed.lock().unwrap() = Decimal::ZERO;
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;
    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::Started
    );
    assert_eq!(setup.spend.charges().len(), 2);
    eventually(|| !builder.is_submitting()).await;
    assert_eq!(maker.submitted().len(), 2);
    builder.stop_following();
}

#[tokio::test]
async fn a_poll_turned_down_for_the_sign_in_stops_asks_for_it_and_follows_again_after_reconnecting() {
    let maker = FakeMaker::new();
    maker.script(|script| script.poll_error = Some(ProviderError::Unauthorized));
    let setup = Setup::new();
    let builder = setup.builder();
    let sign_ins = Arc::new(AtomicUsize::new(0));
    let counter = sign_ins.clone();
    builder.set_on_sign_in_expired(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    builder.refresh_quote(maker.as_ref(), &upload(&maker)).await;

    assert_eq!(
        builder.generate(as_maker(&maker), false).await,
        PaidStart::Started
    );
    eventually(|| matches!(first_state(&builder), Some(CharacterRunState::Interrupted(_)))).await;

    // Stopped at once instead of polling on, and the connection was flagged.
    assert_eq!(sign_ins.load(Ordering::SeqCst), 1);
    assert_eq!(maker.polls.load(Ordering::SeqCst), 1);
    assert_eq!(builder.active_count(), 0);
    // Higgsfield accepted the request: it may still finish and be charged.
    assert_eq!(
        setup
            .spend
            .charges()
            .iter()
            .map(|charge| charge.is_pending)
            .collect::<Vec<_>>(),
        vec![true]
    );
    assert_eq!(setup.ids("user_1"), vec!["job-1", "job-2"]);

    // Reconnected: the same sheets are followed again, not sent again.
    maker.script(|script| script.poll_error = None);
    builder.reset();
    builder.resume(&as_maker(&maker));
    eventually(|| first_state(&builder) == Some(CharacterRunState::Finished)).await;

    assert_eq!(maker.submitted().len(), 1);
    assert_eq!(
        setup
            .spend
            .charges()
            .iter()
            .map(|charge| charge.is_pending)
            .collect::<Vec<_>>(),
        vec![false]
    );
    assert_eq!(setup.log.pending(Some("user_1")), []);
}

#[test]
fn the_price_line_shows_the_quote_as_higgsfield_gives_it() {
    let quote = |credits: &str, count| CharacterQuote {
        credits: credits.parse().unwrap(),
        count,
        request: Vec::new(),
    };
    // Live: one sheet is 1.125 credits, four are 4.5, and today's spend counts them so.
    let one = quote("1.125", 1);
    let four = quote("4.5", 4);
    assert_eq!(quote_price(&one), "1.125 cr");
    assert_eq!(quote_price(&four), "4.5 cr");
    assert_eq!(quote_price(&quote("10", 4)), "10 cr");
    // Never shown lower than the quote.
    assert_eq!(quote_price(&quote("1.1251", 1)), "1.126 cr");
    assert_eq!(quote_price_text(&four), "4.5 cr · Price from Higgsfield");
    assert_eq!(one.cost(), credits("1.125"));
    assert_eq!(four.cost(), credits("4.5"));
    assert_eq!(one.cost().unit(), CostUnit::Credits);
}

#[test]
fn the_log_survives_a_relaunch() {
    let folder = tempfile::tempdir().unwrap();
    let first = CharacterSheetLog::open(Some(folder.path()));
    // Whole seconds: the file keeps ISO 8601 dates.
    let date = OffsetDateTime::from_unix_timestamp(1_760_000_000).unwrap();
    first.add(vec![CharacterSheetRecord::new(
        "job-1",
        Some("user_1"),
        Uuid::new_v4(),
        date,
        "x",
    )]);
    first.update(
        "job-1",
        CharacterSheetState::Completed {},
        Some(url("https://cdn.example.com/1.png")),
    );
    let saved = url("file:///tmp/1.png");
    first.set_file(saved.clone(), "job-1");

    let second = CharacterSheetLog::open(Some(folder.path()));

    assert_eq!(second.records(), first.records());
    assert_eq!(
        second
            .sheets(Some("user_1"))
            .first()
            .and_then(|sheet| sheet.file_url.clone()),
        Some(saved)
    );
}
