//! `AgentImageQueueTests`, `AgentImageHoldTests`.

use jutsu_core::{
    AgentHold, AgentMoneyUnit, ImageJobStatus, ImageModelInput, LocalImage, Rejection, SpendCap, SpendStatus,
};
use rust_decimal::Decimal;
use url::Url;

use super::*;
use crate::agents::image_record::image_job;
use crate::agents::test_fakes::{ImageFake, MemoryLedger};
use crate::images::{ImageJobLog, ImageSpendLog};
use crate::testing::eventually;

fn usd(text: &str, is_estimate: bool) -> Cost {
    Cost::usd(text.parse().unwrap(), is_estimate)
}

fn soul() -> jutsu_core::ImageModel {
    let mut model = jutsu_core::ImageModel::new("soul", "Soul 2", vec!["1:1".to_owned(), "16:9".to_owned()])
        .with_variant_counts(vec![1, 4]);
    model.list_price = Some(usd("0.02", true));
    model
}

fn edit() -> jutsu_core::ImageModel {
    let mut model = jutsu_core::ImageModel::new("edit", "Edit", vec!["1:1".to_owned()]);
    model.input = ImageModelInput::Required;
    model.max_inputs = Some(3);
    model
}

fn picture(index: usize) -> LocalImage {
    LocalImage::new(Url::parse(&format!("file:///tmp/agent-input-{index}.png")).unwrap())
}

struct Setup {
    folder: tempfile::TempDir,
    ledger: Arc<MemoryLedger>,
    provider: Arc<ImageFake>,
}

impl Setup {
    fn new() -> Self {
        Self {
            folder: tempfile::tempdir().unwrap(),
            ledger: MemoryLedger::new(),
            provider: ImageFake::new(),
        }
    }

    fn generator(cap: Option<SpendStatus>) -> Arc<ReferenceGenerator> {
        let spend = Arc::new(ImageSpendLog::open(
            None,
            OffsetDateTime::now_utc(),
            time::UtcOffset::UTC,
        ));
        let generator = ReferenceGenerator::new(
            spend,
            Arc::new(ImageJobLog::open(None)),
            Arc::new(move |_| Box::pin(async move { Ok(cap) })),
            Arc::new(|_| Box::pin(tokio::task::yield_now())),
        );
        generator.edit_form(|form| form.use_models(vec![soul(), edit()], None));
        generator
    }

    fn queue(&self, generator: &Arc<ReferenceGenerator>, limit: i64) -> Arc<AgentImageQueue> {
        AgentImageQueue::new(
            Some(self.folder.path()),
            generator.clone(),
            self.ledger.clone(),
            Arc::new(move |_| limit),
            Arc::new(OffsetDateTime::now_utc),
        )
    }

    fn create(&self, queue: &AgentImageQueue, order: &AgentImageOrder, max_cents: i64) -> Uuid {
        let id = Uuid::new_v4();
        queue
            .create(order, id, origin(max_cents), self.provider.clone(), upload())
            .unwrap();
        id
    }
}

fn origin(max_cents: i64) -> AgentOrigin {
    AgentOrigin::new(
        Some("claude-code".to_owned()),
        "img-1",
        AgentMoney::new(max_cents, AgentMoneyUnit::UsdCents),
    )
}

fn upload() -> UploadFn {
    Arc::new(|_| {
        Box::pin(async {
            Ok(jutsu_core::RemoteAsset::new(
                Url::parse("https://cdn.example.com/in.png").unwrap(),
            ))
        })
    })
}

fn order(prompt: &str, count: Option<i64>, model: Option<&str>) -> AgentImageOrder {
    AgentImageOrder {
        model: model.map(str::to_owned),
        prompt: prompt.to_owned(),
        count,
        ..AgentImageOrder::default()
    }
}

fn fox() -> AgentImageOrder {
    order("a red fox", None, None)
}

fn state(queue: &AgentImageQueue, id: Uuid) -> Option<AgentImageState> {
    queue.record(id).map(|record| record.state)
}

// MARK: The order

#[test]
fn the_request_is_built_as_the_image_side_builds_it() {
    let models = [soul(), edit()];
    let made = order("a red fox", Some(4), None).draft(&models, false).unwrap();
    assert_eq!(made.model.id, "soul");
    assert_eq!(made.aspect_ratio, "1:1");
    assert_eq!(made.count, 4);
    let refusal = |order: AgentImageOrder, saved: bool| {
        order.draft(&models, saved).err().map(|refusal| refusal.message)
    };
    assert!(
        refusal(order("x", None, Some("nope")), false)
            .unwrap()
            .contains("no image model nope")
    );
    assert!(refusal(order("  ", None, None), false).is_some());
    assert!(
        refusal(order("x", Some(3), None), false)
            .unwrap()
            .contains("1, 4")
    );
    assert!(
        refusal(order("x", None, Some("edit")), false)
            .unwrap()
            .contains("send inputs")
    );
    let mut wide = fox();
    wide.aspect_ratio = Some("21:9".to_owned());
    assert!(refusal(wide, false).is_some());
    assert!(refusal(fox(), true).is_some());
    // A model whose catalog lists no frame (the account's MiniMax H3 Image) offers only "auto", taken unasked.
    let any_frame =
        jutsu_core::ImageModel::new("minimax_h3_image", "MiniMax H3 Image", vec!["auto".to_owned()]);
    let auto = order("x", None, Some("minimax_h3_image"))
        .draft(&[soul(), edit(), any_frame], false)
        .unwrap();
    assert_eq!(auto.aspect_ratio, "auto");
}

/// Several images go in order, up to the model's own limit from its catalog; a text-only model takes none.
#[test]
fn inputs_keep_their_order_up_to_the_models_limit() {
    let models = [soul(), edit()];
    let pictures: Vec<LocalImage> = (1..=4).map(picture).collect();
    let draft = |model: &str, inputs: &[LocalImage]| {
        AgentImageOrder {
            model: Some(model.to_owned()),
            prompt: "x".to_owned(),
            inputs: inputs.to_vec(),
            ..AgentImageOrder::default()
        }
        .draft(&models, false)
    };
    assert_eq!(
        draft("edit", &pictures[..3]).unwrap().inputs,
        pictures[..3].to_vec()
    );
    assert!(
        draft("edit", &pictures)
            .unwrap_err()
            .message
            .contains("at most 3 input images")
    );
    assert!(
        draft("soul", &pictures[..1])
            .unwrap_err()
            .message
            .contains("leave out inputs")
    );
    assert_eq!(soul().input_limit(), 0);
    let mut optional = jutsu_core::ImageModel::new("g", "G", vec!["1:1".to_owned()]);
    optional.input = ImageModelInput::Optional;
    assert_eq!(optional.input_limit(), 1);
}

/// A record of one input holds only `input`, as a 0.2.0 record does; several are kept in order.
#[tokio::test]
async fn several_inputs_are_recorded_and_sent_in_order() {
    let setup = Setup::new();
    let generator = Setup::generator(None);
    let queue = setup.queue(&generator, 1_000);
    let pictures: Vec<LocalImage> = (1..=3).map(picture).collect();
    let several = AgentImageOrder {
        model: Some("edit".to_owned()),
        prompt: "swap".to_owned(),
        inputs: pictures.clone(),
        ..AgentImageOrder::default()
    };
    let id = setup.create(&queue, &several, 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Finished)).await;
    let record = queue.record(id).unwrap();
    assert_eq!(record.inputs(), pictures);
    assert_eq!(record.input, Some(pictures[0].clone()));
    assert_eq!(setup.provider.submitted()[0].input_images.len(), 3);
    let one = AgentImageOrder {
        inputs: vec![picture(1)],
        ..several
    };
    let single = setup.create(&queue, &one, 50);
    assert_eq!(queue.record(single).unwrap().several_inputs, None);
    assert_eq!(queue.record(single).unwrap().inputs(), vec![picture(1)]);
}

/// The agent's title names its row in the Queue and the images it makes; a request without one is named as before.
#[tokio::test]
async fn a_title_names_the_request_and_its_images() {
    let setup = Setup::new();
    let generator = Setup::generator(None);
    let queue = setup.queue(&generator, 1_000);
    let mut titled = fox();
    titled.title = Some("  Fox portrait\n".to_owned());
    let id = setup.create(&queue, &titled, 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Finished)).await;
    assert_eq!(queue.record(id).unwrap().title.as_deref(), Some("Fox portrait"));
    let run = generator.runs().into_iter().find(|run| run.id == id).unwrap();
    assert!(run.title().starts_with("Fox portrait · Soul 2 · "));
    let images = generator.images();
    assert!(
        !images.is_empty()
            && images
                .iter()
                .all(|image| image.title.as_deref() == Some("Fox portrait"))
    );

    let plain = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, plain) == Some(AgentImageState::Finished)).await;
    assert_eq!(queue.record(plain).unwrap().title, None);
    let run = generator.runs().into_iter().find(|run| run.id == plain).unwrap();
    assert!(run.title().starts_with("Soul 2 · "));
}

#[tokio::test]
async fn the_visible_form_is_never_touched() {
    let setup = Setup::new();
    let generator = Setup::generator(None);
    generator.edit_form(|form| form.prompt = "the person's own prompt".to_owned());
    let queue = setup.queue(&generator, 1_000);
    setup.create(&queue, &order("a red fox", Some(4), None), 50);
    assert_eq!(generator.form().prompt, "the person's own prompt");
    assert_eq!(generator.form().count, 1);
}

// MARK: The guard

#[tokio::test]
async fn a_request_under_every_limit_is_reserved_sent_and_settled() {
    let setup = Setup::new();
    let generator = Setup::generator(None);
    let queue = setup.queue(&generator, 1_000);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Finished)).await;
    assert_eq!(setup.provider.submitted().len(), 1);
    let record = queue.record(id).unwrap();
    assert!(record.may_have_been_sent());
    assert!(record.origin.reservation.is_some());
    let spends = setup.ledger.spends();
    assert_eq!(
        spends.iter().map(|spend| spend.is_settled).collect::<Vec<_>>(),
        [true]
    );
    assert_eq!(spends[0].money, AgentMoney::new(8, AgentMoneyUnit::UsdCents));
    assert_eq!(
        generator.runs()[0]
            .agent
            .as_ref()
            .and_then(|agent| agent.client.as_deref()),
        Some("claude-code")
    );
}

#[tokio::test]
async fn a_price_over_max_cost_is_held_and_nothing_is_sent() {
    let setup = Setup::new();
    let generator = Setup::generator(None);
    let queue = setup.queue(&generator, 1_000);
    let id = setup.create(&queue, &fox(), 7);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;
    let record = queue.record(id).unwrap();
    assert_eq!(record.hold, Some(AgentImageHold::Agent(AgentHold::OverMaxCost)));
    assert_eq!(setup.provider.submitted().len(), 0);
    assert_eq!(setup.ledger.spends().len(), 0);
    assert_eq!(
        generator.runs()[0].state,
        RunState::Held(hold_text(AgentImageHold::Agent(AgentHold::OverMaxCost)).to_owned())
    );
    let status = image_job(&record, &[]);
    assert_eq!(status.state, "paused");
    assert!(status.needs_person);
    assert_eq!(status.hold.as_deref(), Some("over_max_cost"));
    assert!(status.kind == "image" && status.mode.as_deref() == Some("image") && status.results.is_none());
}

#[tokio::test]
async fn an_unknown_price_is_held() {
    let setup = Setup::new();
    setup
        .provider
        .script(|script| script.estimate = Err(ProviderError::Server { status: 500 }));
    let queue = setup.queue(&Setup::generator(None), 1_000);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;
    // The list price is only an estimate: never trusted for an agent.
    assert_eq!(
        queue.record(id).unwrap().hold,
        Some(AgentImageHold::Agent(AgentHold::UnknownPrice))
    );
    assert_eq!(setup.provider.submitted().len(), 0);
}

/// A model whose catalog states no limit takes up to the bound; Higgsfield's quote refuses more than it takes. The
/// quote comes before anything is reserved or sent, so the refusal fails the request and costs nothing.
#[tokio::test]
async fn a_quote_that_refuses_the_inputs_fails_the_request_before_anything_is_sent() {
    let setup = Setup::new();
    setup.provider.script(|script| {
        script.estimate = Err(ProviderError::Validation {
            field: None,
            message: "This model takes at most 4 images.".to_owned(),
        });
    });
    let mut open = jutsu_core::ImageModel::new("open", "Open", vec!["1:1".to_owned()]);
    open.input = ImageModelInput::Optional;
    open.unstated_input_limit = Some(jutsu_core::ImageModel::INPUT_BOUND);
    let generator = Setup::generator(None);
    generator.edit_form(|form| form.use_models(vec![soul(), edit(), open.clone()], None));
    let queue = setup.queue(&generator, 1_000);
    let six = AgentImageOrder {
        model: Some("open".to_owned()),
        prompt: "x".to_owned(),
        inputs: (1..=6).map(picture).collect(),
        ..AgentImageOrder::default()
    };
    let id = setup.create(&queue, &six, 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Failed)).await;
    let record = queue.record(id).unwrap();
    assert!(record.error.unwrap().contains("at most 4 images"));
    assert!(record.sent_at.is_none());
    assert_eq!(setup.provider.submitted().len(), 0);
    assert_eq!(setup.ledger.spends().len(), 0);
    assert!(open.input_limit() == 14 && open.input_limit_is_bound());
}

#[tokio::test]
async fn over_the_daily_agent_budget_is_held() {
    let setup = Setup::new();
    let queue = setup.queue(&Setup::generator(None), 5);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;
    assert_eq!(
        queue.record(id).unwrap().hold,
        Some(AgentImageHold::Agent(AgentHold::OverDailyBudget))
    );
    assert_eq!(setup.provider.submitted().len(), 0);
}

#[tokio::test]
async fn reservations_in_flight_count_against_the_budget() {
    let setup = Setup::new();
    let other = AgentSpend::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        AgentMoney::new(95, AgentMoneyUnit::UsdCents),
        OffsetDateTime::now_utc(),
        false,
    );
    assert!(setup.ledger.reserve(other, None).await.unwrap());
    let queue = setup.queue(&Setup::generator(None), 100);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;
    assert_eq!(
        queue.record(id).unwrap().hold,
        Some(AgentImageHold::Agent(AgentHold::OverDailyBudget))
    );
}

#[tokio::test]
async fn a_ledger_that_cant_be_written_sends_nothing() {
    let setup = Setup::new();
    setup.ledger.break_it();
    let queue = setup.queue(&Setup::generator(None), 1_000);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;
    assert_eq!(
        queue.record(id).unwrap().hold,
        Some(AgentImageHold::Agent(AgentHold::LedgerUnavailable))
    );
    assert_eq!(setup.provider.submitted().len(), 0);
}

#[tokio::test]
async fn the_persons_own_cap_holds_it_too() {
    let setup = Setup::new();
    let cap = SpendStatus {
        committed_today: "0.95".parse::<Decimal>().unwrap(),
        cap: Some(SpendCap::usd(Decimal::ONE)),
    };
    let queue = setup.queue(&Setup::generator(Some(cap)), 1_000);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;
    let record = queue.record(id).unwrap();
    assert_eq!(record.hold, Some(AgentImageHold::PriceAboveCap));
    assert_eq!(
        image_job(&record, &[]).pause_reason.as_deref(),
        Some("price_above_cap")
    );
    assert_eq!(setup.provider.submitted().len(), 0);
}

#[tokio::test]
async fn only_the_person_sends_a_held_request() {
    let setup = Setup::new();
    let generator = Setup::generator(None);
    let queue = setup.queue(&generator, 1_000);
    let id = setup.create(&queue, &fox(), 7);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;

    queue.approve(id, setup.provider.clone(), upload());
    eventually(|| state(&queue, id) == Some(AgentImageState::Finished)).await;

    assert_eq!(setup.provider.submitted().len(), 1);
    assert!(queue.record(id).unwrap().origin.approved_by_person);
    assert_eq!(setup.ledger.spends().len(), 1);
}

/// The person approved the price they saw: a higher quote right before the submit waits for them again.
#[tokio::test]
async fn a_price_above_the_approved_one_is_held_again() {
    let setup = Setup::new();
    let generator = Setup::generator(None);
    let queue = setup.queue(&generator, 1_000);
    let id = setup.create(&queue, &fox(), 7);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;

    setup
        .provider
        .script(|script| script.estimate = Ok(usd("0.2", false)));
    queue.approve(id, setup.provider.clone(), upload());
    eventually(|| {
        queue.record(id).and_then(|record| record.hold)
            == Some(AgentImageHold::Agent(AgentHold::AboveApprovedPrice))
    })
    .await;
    let record = queue.record(id).unwrap();
    assert_eq!(record.state, AgentImageState::Held);
    assert_eq!(setup.provider.submitted().len(), 0);
    assert_eq!(
        image_job(&record, &[]).hold.as_deref(),
        Some("above_approved_price")
    );

    queue.approve(id, setup.provider.clone(), upload());
    eventually(|| state(&queue, id) == Some(AgentImageState::Finished)).await;
    assert_eq!(setup.provider.submitted().len(), 1);
    let money: Vec<AgentMoney> = setup.ledger.spends().iter().map(|spend| spend.money).collect();
    assert_eq!(money, [AgentMoney::new(20, AgentMoneyUnit::UsdCents)]);
}

#[tokio::test]
async fn a_refused_submit_gives_the_reservation_back() {
    let setup = Setup::new();
    setup.provider.script(|script| {
        script.submit_error = Some(ProviderError::Validation {
            field: None,
            message: "No".to_owned(),
        });
    });
    let queue = setup.queue(&Setup::generator(None), 1_000);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Failed)).await;
    eventually(|| setup.ledger.spends().is_empty()).await;
    eventually(|| {
        queue
            .record(id)
            .is_some_and(|record| !record.may_have_been_sent())
    })
    .await;
}

#[tokio::test]
async fn an_unanswered_submit_keeps_the_reservation() {
    let setup = Setup::new();
    setup
        .provider
        .script(|script| script.submit_error = Some(ProviderError::Network));
    let queue = setup.queue(&Setup::generator(None), 1_000);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Failed)).await;
    let settled: Vec<bool> = setup
        .ledger
        .spends()
        .iter()
        .map(|spend| spend.is_settled)
        .collect();
    assert_eq!(settled, [false]);
    let record = queue.record(id).unwrap();
    assert!(record.may_have_been_sent());
    assert!(image_job(&record, &[]).charged);
}

// MARK: What it was charged

/// Higgsfield bills only the jobs that made images: the others' share goes back once all of them ended.
#[tokio::test]
async fn a_request_counts_only_the_jobs_that_made_images() {
    let setup = Setup::new();
    let image = ImageJobStatus::Completed(vec![Url::parse("https://cdn.example.com/1.png").unwrap()]);
    setup.provider.script(|script| {
        script.outcomes = vec![
            image.clone(),
            ImageJobStatus::Failed { message: None },
            ImageJobStatus::Rejected(Rejection::ExplicitContent),
            image,
        ];
    });
    let queue = setup.queue(&Setup::generator(None), 1_000);
    let id = setup.create(&queue, &order("a red fox", Some(4), None), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Finished)).await;
    eventually(|| setup.ledger.spent() == 4).await;
    let money: Vec<AgentMoney> = setup.ledger.spends().iter().map(|spend| spend.money).collect();
    assert_eq!(money, [AgentMoney::new(8, AgentMoneyUnit::UsdCents)]);
}

#[tokio::test]
async fn a_request_whose_jobs_all_failed_counts_nothing() {
    let setup = Setup::new();
    setup.provider.script(|script| {
        script.outcomes = vec![ImageJobStatus::Failed {
            message: Some("Boom".to_owned()),
        }];
    });
    let queue = setup.queue(&Setup::generator(None), 1_000);
    let id = setup.create(&queue, &fox(), 50);
    eventually(|| state(&queue, id) == Some(AgentImageState::Failed)).await;
    eventually(|| setup.ledger.spent() == 0 && setup.ledger.spends().iter().all(|spend| spend.is_settled))
        .await;
    assert_eq!(setup.ledger.spends().len(), 1);
}

/// Cancelled while its jobs waited: none ran, so none is billed. One the provider wouldn't cancel may have run, and
/// keeps its whole price.
#[tokio::test]
async fn a_canceled_request_counts_nothing_unless_the_cancel_was_refused() {
    for refused in [false, true] {
        let setup = Setup::new();
        setup.provider.script(|script| {
            script.outcomes = vec![ImageJobStatus::Pending, ImageJobStatus::Pending];
            script.refuses_cancel = refused;
        });
        let generator = Setup::generator(None);
        let queue = setup.queue(&generator, 1_000);
        let id = setup.create(&queue, &order("a red fox", Some(4), None), 50);
        eventually(|| state(&queue, id) == Some(AgentImageState::Following)).await;
        eventually(|| generator.job_log().records().iter().any(|job| job.run_id == id)).await;

        assert!(queue.cancel(id));
        eventually(|| queue.record(id).is_some_and(|record| record.is_over())).await;
        let expected = if refused { 8 } else { 0 };
        eventually(|| setup.ledger.spent() == expected).await;
        assert_eq!(state(&queue, id), Some(AgentImageState::Canceled));
    }
}

#[tokio::test]
async fn a_held_request_canceled_was_never_sent() {
    let setup = Setup::new();
    let queue = setup.queue(&Setup::generator(None), 1_000);
    let id = setup.create(&queue, &fox(), 1);
    eventually(|| state(&queue, id) == Some(AgentImageState::Held)).await;

    assert!(queue.cancel(id));
    assert_eq!(state(&queue, id), Some(AgentImageState::Canceled));
    assert!(!queue.cancel(id));
    assert_eq!(setup.provider.submitted().len(), 0);
}

// MARK: Relaunch

#[tokio::test]
async fn a_relaunch_releases_orphans_fails_lost_answers_and_shows_held_requests() {
    let setup = Setup::new();
    let first = setup.queue(&Setup::generator(None), 1_000);
    let held = setup.create(&first, &fox(), 1);
    eventually(|| state(&first, held) == Some(AgentImageState::Held)).await;

    // Another launch's request: reserved and saved as being sent, then the app quit before an answer.
    let now = OffsetDateTime::now_utc();
    let money = AgentMoney::new(8, AgentMoneyUnit::UsdCents);
    let sent = AgentSpend::new(Uuid::new_v4(), Uuid::new_v4(), money, now, false);
    let orphan = AgentSpend::new(Uuid::new_v4(), held, money, now, false);
    assert!(setup.ledger.reserve(orphan, None).await.unwrap());
    assert!(setup.ledger.reserve(sent.clone(), None).await.unwrap());
    let file = setup.folder.path().join("agent-images.json");
    let mut records: Vec<AgentImageRecord> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let draft = order("x", None, None).draft(&[soul()], false).unwrap();
    let mut sending = AgentImageRecord::taken(
        sent.job_id,
        records[0].origin.clone(),
        ProviderKind::ApiKey,
        Some("key-1".to_owned()),
        draft,
        now,
    );
    sending.state = AgentImageState::Sending;
    sending.price_is_known = true;
    sending.sent_at = Some(now);
    sending.origin.reservation = Some(sent.id);
    records.push(sending);
    std::fs::write(&file, serde_json::to_vec(&records).unwrap()).unwrap();

    let generator = Setup::generator(None);
    let relaunched = setup.queue(&generator, 1_000);
    relaunched.restore().await;

    let ids: Vec<Uuid> = setup.ledger.spends().iter().map(|spend| spend.id).collect();
    assert_eq!(ids, [sent.id]);
    assert_eq!(state(&relaunched, sent.job_id), Some(AgentImageState::Failed));
    assert!(relaunched.record(sent.job_id).unwrap().may_have_been_sent());
    let runs = generator.runs();
    assert_eq!(runs.iter().map(|run| run.id).collect::<Vec<_>>(), [held]);
    assert_eq!(
        runs[0].agent.as_ref().map(|agent| agent.request_id.as_str()),
        Some("img-1")
    );
}

#[tokio::test]
async fn a_record_that_cant_be_read_takes_nothing() {
    let setup = Setup::new();
    std::fs::write(setup.folder.path().join("agent-images.json"), "not json").unwrap();
    let queue = setup.queue(&Setup::generator(None), 1_000);
    let refused = queue.create(
        &fox(),
        Uuid::new_v4(),
        origin(50),
        setup.provider.clone(),
        upload(),
    );
    assert!(refused.unwrap_err().is_not_saved);
}

// MARK: AgentImageHoldTests

/// `agent-images.json` keeps the holds as 0.2.2 wrote them: read back as the same holds, and written the same.
#[test]
fn stored_holds_read_and_write_as_before() {
    let stored = r#"["unknownPrice","overMaxCost","overDailyBudget","ledgerUnavailable","unlimitedNotAllowed","aboveApprovedPrice","priceAboveCap"]"#;
    let holds: Vec<AgentImageHold> = serde_json::from_str(stored).unwrap();
    assert_eq!(
        holds,
        [
            AgentImageHold::Agent(AgentHold::UnknownPrice),
            AgentImageHold::Agent(AgentHold::OverMaxCost),
            AgentImageHold::Agent(AgentHold::OverDailyBudget),
            AgentImageHold::Agent(AgentHold::LedgerUnavailable),
            AgentImageHold::Agent(AgentHold::UnlimitedNotAllowed),
            AgentImageHold::Agent(AgentHold::AboveApprovedPrice),
            AgentImageHold::PriceAboveCap,
        ]
    );
    assert_eq!(serde_json::to_string(&holds).unwrap(), stored);
    assert!(serde_json::from_str::<Vec<AgentImageHold>>(r#"["overBudget"]"#).is_err());
}

/// A held image request says why in the words a held video job's row uses; only the action differs, as the image
/// row has Review… for Confirm.
#[test]
fn held_images_and_videos_share_their_words() {
    let holds = [
        AgentHold::UnknownPrice,
        AgentHold::OverMaxCost,
        AgentHold::OverDailyBudget,
        AgentHold::LedgerUnavailable,
        AgentHold::UnlimitedNotAllowed,
        AgentHold::AboveApprovedPrice,
    ];
    for hold in holds {
        let video = crate::queue::row::agent_note_text(Some(hold));
        assert!(video.ends_with("confirm to send it"), "{hold:?}");
        assert_eq!(
            hold_text(AgentImageHold::Agent(hold)),
            video.replace("confirm to send it", "review to send it"),
            "{hold:?}"
        );
    }
    assert_eq!(
        hold_text(AgentImageHold::PriceAboveCap),
        "Price from Higgsfield goes over today's spending cap · review to send it"
    );
}
