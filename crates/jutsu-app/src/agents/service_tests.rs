//! The handler's pure rules (`AgentService+Video.swift`, `AgentService+Requests.swift`).

use jutsu_agent::{AgentAmount, AgentBudgetUnit, SubtitlesCommand};
use jutsu_core::{Cost, SpendCap};
use jutsu_engine::OverCap;

use super::*;

#[test]
fn max_cost_must_be_in_the_connections_unit() {
    let cents = AgentAmount::new(100, AgentBudgetUnit::UsdCents);
    let credits = AgentAmount::new(100, AgentBudgetUnit::Credits);
    assert!(check_unit(&cents, ProviderKind::ApiKey).is_ok());
    assert!(check_unit(&credits, ProviderKind::Account).is_ok());
    let wrong = check_unit(&credits, ProviderKind::ApiKey).unwrap_err();
    assert_eq!(wrong.code, ErrorCode::BAD_REQUEST);
    assert!(wrong.message.contains("max_cost is in usd on this connection"));
    assert!(
        check_unit(&cents, ProviderKind::Account)
            .unwrap_err()
            .message
            .contains("max_cost is in credits")
    );
}

/// `over_cap` tells the agent the cap, and a lost connection says how to fix it.
#[test]
fn enqueue_errors_become_the_agents_codes() {
    let over = enqueue_error(&EngineError::OverCap(OverCap {
        cap: SpendCap::credits(300),
        committed_today: rust_decimal::Decimal::from(290),
        price: Cost::credits(rust_decimal::Decimal::from(24), false),
    }));
    assert_eq!(over.code, ErrorCode::OVER_CAP);
    assert!(!over.retryable);
    assert!(over.message.contains("daily cap of 300 credits"));
    assert_eq!(
        enqueue_error(&EngineError::NotConnected).code,
        ErrorCode::NOT_CONNECTED
    );
    assert_eq!(
        enqueue_error(&EngineError::SubmitInProgress).code,
        ErrorCode::FAILED
    );
}

/// Genjutsu is offered until the connection's capabilities are read, as `lastOfferedModels` starts; nothing without a
/// connection.
#[test]
fn a_model_is_available_once_connected_and_offered() {
    let context = crate::create::context::CreateContext::default();
    let offered = context.offered_models();
    let other = VideoModel::all()
        .into_iter()
        .find(|model| *model != VideoModel::GENJUTSU)
        .unwrap();
    assert!(model_available(
        Some(ProviderKind::Account),
        &offered,
        &VideoModel::GENJUTSU
    ));
    assert!(!model_available(Some(ProviderKind::Account), &offered, &other));
    assert!(!model_available(None, &offered, &VideoModel::GENJUTSU));
}

/// The sweep keeps what the store's recent jobs and the image requests not over still read, not only the Queue's.
#[test]
fn the_inbox_keeps_the_inputs_of_recent_jobs_and_open_image_requests() {
    use super::super::image_record::{AgentImageRecord, AgentImageState};
    use crate::images::form::ImageRequestDraft;
    use jutsu_core::{AgentMoney, AgentMoneyUnit, ImageModel};
    use url::Url;

    let job = crate::library::fixture::item().job;
    let record = |path: &str, state: AgentImageState| {
        let mut record = AgentImageRecord::taken(
            Uuid::new_v4(),
            AgentOrigin::new(
                None,
                "r".to_owned(),
                AgentMoney::new(10, AgentMoneyUnit::UsdCents),
            ),
            ProviderKind::ApiKey,
            None,
            ImageRequestDraft {
                model: ImageModel::new("soul", "Soul", vec!["1:1".to_owned()]),
                prompt: "A knight".to_owned(),
                aspect_ratio: "1:1".to_owned(),
                count: 1,
                inputs: vec![LocalImage::new(Url::from_file_path(path).unwrap())],
                title: None,
            },
            OffsetDateTime::now_utc(),
        );
        record.state = state;
        record
    };
    let kept = kept_inputs(
        &[job],
        &[
            record("/tmp/inbox/a/open.png", AgentImageState::Held),
            record("/tmp/inbox/b/done.png", AgentImageState::Finished),
        ],
    );
    assert!(kept.contains("/home/me/Movies/rooftop.mov"));
    assert!(kept.contains("/tmp/inbox/a/open.png"));
    assert!(!kept.contains("/tmp/inbox/b/done.png"));
}

/// `capabilities` with `image_models` lists each model as `create_image` takes it, its inputs within the protocol's
/// bound and saying so when the bound stands in for the model's own limit (`AgentCapabilities.ImageModel.init`).
#[test]
fn an_image_model_is_listed_as_create_image_takes_it() {
    let mut edits =
        jutsu_core::ImageModel::new("nano", "Nano Banana", vec!["1:1".to_owned(), "16:9".to_owned()])
            .with_variant_counts(vec![1, 4]);
    edits.input = jutsu_core::ImageModelInput::Optional;
    edits.unstated_input_limit = Some(jutsu_core::ImageModel::INPUT_BOUND);
    let listed = images::image_model(&edits);
    assert_eq!(listed.input, "optional");
    assert_eq!(listed.counts, [1, 4]);
    assert_eq!(listed.max_inputs, jutsu_agent::MAX_INPUTS);
    assert_eq!(
        listed.max_inputs_note,
        Some(jutsu_agent::ImageModel::bound_note())
    );

    let text = images::image_model(&jutsu_core::ImageModel::new(
        "soul",
        "Soul",
        vec!["1:1".to_owned()],
    ));
    assert_eq!(
        (text.input.as_str(), text.max_inputs, text.max_inputs_note),
        ("none", 0, None)
    );
}

/// A queued `create_video` answers each image it sends by its `@ImageN` tag: the elements' images first, in the order
/// named, then the inbox files, in order (`AgentService+Video.swift`).
#[test]
fn a_new_video_answers_its_references_in_order() {
    let image = |path: &str| LocalImage::new(url::Url::from_file_path(path).unwrap());
    let elements = [
        crate::agents::element_lookup::ElementImages {
            name: "Mara".to_owned(),
            images: vec![image("/tmp/elements/m1.png"), image("/tmp/elements/m2.png")],
        },
        crate::agents::element_lookup::ElementImages {
            name: "Roof".to_owned(),
            images: vec![image("/tmp/elements/r1.png")],
        },
    ];
    let references = images::references(&elements, &[image("/tmp/inbox/1/face.png")]).unwrap();
    assert_eq!(
        references
            .iter()
            .map(|reference| (
                reference.tag.as_str(),
                reference.element.as_deref(),
                reference.file.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            ("@Image1", Some("Mara"), "/tmp/elements/m1.png"),
            ("@Image2", Some("Mara"), "/tmp/elements/m2.png"),
            ("@Image3", Some("Roof"), "/tmp/elements/r1.png"),
            ("@Image4", None, "/tmp/inbox/1/face.png"),
        ]
    );
    assert!(images::references(&[], &[]).is_none());
}

/// Only an enhance and a subtitles `make` wait for the person in an item's sheet (`askedItem`).
#[test]
fn an_enhance_or_a_subtitles_make_asks_about_its_item() {
    let item = Uuid::new_v4();
    let subtitles = |action| {
        AgentCommand::Subtitles(SubtitlesCommand {
            action,
            item: item.into(),
            request_id: None,
            fingerprint: None,
            language: None,
            engine: None,
            format: None,
        })
    };
    assert_eq!(
        asked_item(&subtitles(AgentSubtitlesAction::Make)),
        Some((item, AgentSheet::Subtitles))
    );
    assert_eq!(asked_item(&subtitles(AgentSubtitlesAction::Get)), None);
    assert_eq!(
        asked_item(&AgentCommand::Capabilities(CapabilitiesCommand::default())),
        None
    );
}

/// `show` replaces the sheet it opens, never another (`showSheet`, `showSubtitles`).
#[test]
fn a_sheet_is_never_shown_over_the_persons() {
    use WindowSheet::{OnDevice, Other, Subtitles};
    let refused = |sheet, open| on_device::sheet_open(sheet, open).is_some();
    assert!(!refused(AgentSheet::UpscaleOnMac, None));
    assert!(!refused(AgentSheet::UpscaleOnMac, Some(OnDevice)));
    assert!(refused(AgentSheet::UpscaleOnMac, Some(Subtitles)));
    assert!(refused(AgentSheet::UpscaleOnMac, Some(Other)));
    assert!(!refused(AgentSheet::Subtitles, Some(OnDevice)));
    assert!(!refused(AgentSheet::Subtitles, Some(Subtitles)));
    assert!(refused(AgentSheet::Subtitles, Some(Other)));
}
