//! `enhance` on this computer on the wire: free runs without `max_cost`, the sheet `show` opens, and what `jobs` and
//! `capabilities` say of them.

use super::{FINGERPRINT_A, round_trip};
use crate::{
    AgentAmount, AgentBudgetUnit, AgentCapabilities, AgentCommand, AgentEnhancement, AgentJob, AgentJobs,
    AgentOnDeviceEngine, AgentRequest, AgentResponse, AgentSheet, Connection, ConnectionKind, EnhanceCommand,
    OnDevice, ShowCommand, Timestamp, Uuid, Window, wire,
};

fn enhance(
    operation: AgentEnhancement,
    engine: Option<&str>,
    mode: Option<&str>,
    target: Option<&str>,
    max_cost: Option<AgentAmount>,
) -> EnhanceCommand {
    EnhanceCommand {
        request_id: "r-1".to_owned(),
        fingerprint: FINGERPRINT_A.to_owned(),
        item: Uuid::new_v4(),
        operation,
        target: target.map(str::to_owned),
        frame_rate: None,
        preset: None,
        engine: engine.map(str::to_owned),
        mode: mode.map(str::to_owned),
        aspect_ratio: None,
        language: None,
        voice: None,
        max_cost,
    }
}

fn credits(amount: i64) -> AgentAmount {
    AgentAmount::new(amount, AgentBudgetUnit::Credits)
}

#[test]
fn on_device_runs_round_trip_without_max_cost() {
    let commands = [
        AgentCommand::Enhance(enhance(
            AgentEnhancement::SmoothMotion,
            None,
            Some("slow_2x"),
            None,
            None,
        )),
        AgentCommand::Enhance(enhance(
            AgentEnhancement::SmoothMotion,
            Some("mac"),
            Some("60fps"),
            None,
            None,
        )),
        AgentCommand::Enhance(enhance(AgentEnhancement::Upscale, Some("mac"), None, None, None)),
        AgentCommand::Enhance(enhance(
            AgentEnhancement::Upscale,
            None,
            None,
            Some("4k"),
            Some(credits(20)),
        )),
        AgentCommand::Show(ShowCommand::sheet(AgentSheet::UpscaleOnMac, Uuid::new_v4())),
    ];
    for command in &commands {
        assert_eq!(&round_trip(command), command);
    }
    let request = AgentRequest::new(AgentCommand::Enhance(enhance(
        AgentEnhancement::Upscale,
        Some("mac"),
        None,
        None,
        None,
    )));
    assert!(!String::from_utf8_lossy(&wire::line(&request).expect("line")).contains("maxCost"));
}

#[test]
fn only_runs_on_this_computer_go_without_max_cost() {
    let needs_max_cost = |command: EnhanceCommand| {
        command
            .problem()
            .is_some_and(|problem| problem.contains("needs max_cost"))
    };
    assert_eq!(
        enhance(AgentEnhancement::SmoothMotion, None, Some("60fps"), None, None).problem(),
        None
    );
    assert_eq!(
        enhance(AgentEnhancement::Upscale, Some("mac"), None, None, None).problem(),
        None
    );
    assert!(needs_max_cost(enhance(
        AgentEnhancement::Upscale,
        None,
        None,
        None,
        None
    )));
    assert!(needs_max_cost(enhance(
        AgentEnhancement::Upscale,
        Some("standard"),
        None,
        None,
        None
    )));
    assert_eq!(
        enhance(
            AgentEnhancement::Upscale,
            Some("standard"),
            None,
            None,
            Some(credits(1))
        )
        .problem(),
        None
    );
    assert!(!enhance(AgentEnhancement::Upscale, None, None, None, None).is_on_device());
    assert!(enhance(AgentEnhancement::SmoothMotion, None, Some("60fps"), None, None).is_on_device());
}

#[test]
fn each_on_device_run_takes_only_its_own_options() {
    let problem = |command: EnhanceCommand| command.problem().unwrap_or_default();
    assert_eq!(
        problem(enhance(AgentEnhancement::SmoothMotion, None, None, None, None)),
        "smooth_motion needs mode: 60fps, slow_2x, slow_4x"
    );
    assert_eq!(
        problem(enhance(
            AgentEnhancement::SmoothMotion,
            None,
            Some("120fps"),
            None,
            None
        )),
        "mode is one of 60fps, slow_2x, slow_4x"
    );
    assert_eq!(
        problem(enhance(
            AgentEnhancement::SmoothMotion,
            Some("standard"),
            Some("60fps"),
            None,
            None
        )),
        "smooth_motion runs on this computer only: engine is mac or left out"
    );
    // A target is checked against the item later, by the app.
    assert_eq!(
        enhance(AgentEnhancement::Upscale, Some("mac"), None, Some("1080p"), None).problem(),
        None
    );
    assert_eq!(
        enhance(AgentEnhancement::Upscale, Some("mac"), None, Some("2.5x"), None).problem(),
        None
    );
    assert_eq!(
        problem(enhance(
            AgentEnhancement::Upscale,
            Some("mac"),
            Some("60fps"),
            None,
            None
        )),
        "upscale with engine mac takes no mode"
    );
    assert_eq!(
        problem(enhance(
            AgentEnhancement::Reframe,
            None,
            Some("60fps"),
            None,
            Some(credits(1))
        )),
        "reframe takes no mode"
    );
}

#[test]
fn a_decoded_on_device_run_is_checked_like_the_helpers_one() {
    let line = format!(
        r#"{{"version":1,"command":"enhance","requestId":"r-1","fingerprint":"{FINGERPRINT_A}","item":"{}","operation":"smooth_motion","mode":"90fps"}}"#,
        Uuid::new_v4()
    );
    let refused = wire::request(line.as_bytes()).1.err().map(|error| error.message);
    assert_eq!(
        refused.as_deref(),
        Some("Malformed request: mode is one of 60fps, slow_2x, slow_4x.")
    );
}

#[test]
fn show_takes_an_item_only_with_a_sheet() {
    assert_eq!(ShowCommand::problem(false, false, true, true), None);
    assert!(ShowCommand::problem(false, false, true, false).is_some());
    assert!(ShowCommand::problem(true, false, true, true).is_some());
    assert!(ShowCommand::problem(true, false, false, true).is_some());
    let item = Uuid::new_v4();
    let line = format!(r#"{{"version":1,"command":"show","item":"{item}","sheet":"upscale_on_mac"}}"#);
    assert_eq!(
        wire::request(line.as_bytes()).1.expect("request").command,
        AgentCommand::Show(ShowCommand::sheet(AgentSheet::UpscaleOnMac, item))
    );
}

#[test]
fn an_on_device_run_and_the_engines_read_back() {
    let mut job = AgentJob::new(Uuid::new_v4(), "processing", Some(Timestamp::from_unix(1)));
    job.progress = Some(0.4);
    job.price = Some(credits(0));
    job.is_price_exact = true;
    "on_device".clone_into(&mut job.kind);
    job.request_id = Some("r-1".to_owned());
    job.mode = Some("slow_2x".to_owned());
    job.operation = Some("smooth_motion".to_owned());
    job.item = Some(Uuid::new_v4());
    job.estimated_seconds = Some(12.5);
    let engine = |available, long: Option<i64>, short: Option<i64>, model: &str| AgentOnDeviceEngine {
        available,
        max_long_side: long,
        max_short_side: short,
        model: model.to_owned(),
    };
    let engines = OnDevice::new(
        engine(true, None, None, "none"),
        engine(true, Some(1280), Some(720), "needs_consent"),
        engine(false, Some(1920), Some(1080), "agreed"),
    );
    let mut capabilities = AgentCapabilities::new(
        "1",
        Connection {
            kind: ConnectionKind::None,
            plan: None,
            credits: None,
        },
        None,
    );
    capabilities.on_device = Some(engines.clone());
    capabilities.window = Some(Window {
        open: false,
        menu_bar_mode: true,
    });
    let response = AgentResponse {
        capabilities: Some(capabilities),
        jobs: Some(AgentJobs {
            jobs: vec![job],
            timed_out: false,
            next_cursor: None,
        }),
        ..AgentResponse::default()
    };
    let line = wire::line(&response).expect("line");
    assert_eq!(wire::decode::<AgentResponse>(&line).expect("response"), response);
    assert_eq!(engines.modes, ["60fps", "slow_2x", "slow_4x"]);
}
