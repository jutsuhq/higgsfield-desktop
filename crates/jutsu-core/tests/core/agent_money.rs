//! `AgentMoneyTests.swift`.

use jutsu_core::*;

use crate::common::*;

fn origin(max: i64, approved: bool) -> AgentOrigin {
    let mut origin = AgentOrigin::new(None, "a", AgentMoney::new(max, AgentMoneyUnit::UsdCents));
    origin.approved_by_person = approved;
    origin
}

fn cents(amount: i64) -> AgentMoney {
    AgentMoney::new(amount, AgentMoneyUnit::UsdCents)
}

fn decide(price: &Cost, is_known: bool, origin: &AgentOrigin, daily_limit: i64) -> AgentSpendDecision {
    AgentSpendDecision::decide(
        price,
        is_known,
        origin,
        daily_limit,
        &AgentSpendContext::default(),
    )
}

#[test]
fn prices_round_up_to_whole_units() {
    assert_eq!(AgentMoney::from_cost(&usd("1.301", false)), Some(cents(131)));
    assert_eq!(AgentMoney::from_cost(&usd("2", false)), Some(cents(200)));
    assert_eq!(
        AgentMoney::from_cost(&credits("0.25", false)),
        Some(AgentMoney::new(1, AgentMoneyUnit::Credits))
    );
    assert_eq!(
        AgentMoney::from_cost(&credits("40", true)),
        Some(AgentMoney::new(40, AgentMoneyUnit::Credits))
    );
}

/// Never a trap: a huge, negative or unknown amount is clamped or refused. (Decimal has no NaN, so Swift's NaN case
/// has no counterpart.)
#[test]
fn huge_negative_and_unknown_amounts_dont_trap() {
    let huge = rust_decimal::Decimal::from_scientific("1e27").unwrap();
    assert_eq!(
        AgentMoney::from_cost(&Cost::usd(huge, false)).map(|money| money.amount),
        Some(AgentMoney::CEILING)
    );
    assert_eq!(AgentMoney::from_cost(&credits("-3", false)), None);
    assert_eq!(AgentMoney::from_cost(&Cost::UNKNOWN_USD), None);
    assert_eq!(AgentMoney::new(-5, AgentMoneyUnit::Credits).amount, 0);
    assert_eq!(AgentMoney::sum(i64::MAX, i64::MAX), AgentMoney::CEILING);
    assert_eq!(
        AgentSpendTotal {
            spent: i64::MAX,
            reserved: i64::MAX,
            unlimited_charged: false
        }
        .remaining(10),
        0
    );
}

#[test]
fn the_guard_holds_what_is_over_a_limit_or_unknown() {
    let price = usd("1.3", false);
    assert_eq!(
        decide(&price, true, &origin(130, false), 130),
        AgentSpendDecision::Send {
            money: cents(130),
            limit: Some(130)
        }
    );
    assert_eq!(
        decide(&price, true, &origin(129, false), 500),
        AgentSpendDecision::Hold(AgentHold::OverMaxCost)
    );
    assert_eq!(
        decide(&price, true, &origin(500, false), 129),
        AgentSpendDecision::Hold(AgentHold::OverDailyBudget)
    );
    assert_eq!(
        decide(&price, false, &origin(500, false), 500),
        AgentSpendDecision::Hold(AgentHold::UnknownPrice)
    );
    // Credits against a cents limit never pass.
    assert_eq!(
        decide(&credits("1", false), true, &origin(500, false), 500),
        AgentSpendDecision::Hold(AgentHold::OverMaxCost)
    );
    assert_eq!(
        decide(&price, false, &origin(1, true), 0),
        AgentSpendDecision::Send {
            money: cents(130),
            limit: None
        }
    );
}

/// The person approved the price they saw: a higher one, or one in another unit, goes back to them.
#[test]
fn a_price_above_the_approved_one_is_held_again() {
    let price = usd("2", false);
    let approved = origin(1, true);
    let with = |agreed: Cost| AgentSpendContext {
        approved: Some(agreed),
        ..AgentSpendContext::default()
    };
    assert_eq!(
        AgentSpendDecision::decide(&price, true, &approved, 0, &with(usd("2", true))),
        AgentSpendDecision::Send {
            money: cents(200),
            limit: None
        }
    );
    for agreed in [usd("1.5", true), credits("500", false)] {
        assert_eq!(
            AgentSpendDecision::decide(&price, true, &approved, 0, &with(agreed)),
            AgentSpendDecision::Hold(AgentHold::AboveApprovedPrice)
        );
    }
    assert_eq!(
        AgentSpendDecision::decide(&price, true, &approved, 0, &with(Cost::UNKNOWN_USD)),
        AgentSpendDecision::Send {
            money: cents(200),
            limit: None
        }
    );
}

/// The plan's unlimited allowance pays it: nothing counts against the budget, but it goes out without the person
/// only when they let agents use it.
#[test]
fn an_unlimited_request_is_held_unless_agents_may_use_unlimited() {
    let price = usd("1.3", false);
    let unlimited = AgentSpendContext {
        pays_unlimited: true,
        ..AgentSpendContext::default()
    };
    assert_eq!(
        AgentSpendDecision::decide(&price, true, &origin(500, false), 500, &unlimited),
        AgentSpendDecision::Hold(AgentHold::UnlimitedNotAllowed)
    );
    let allowed = AgentSpendContext {
        pays_unlimited: true,
        allows_unlimited: true,
        approved: None,
    };
    assert_eq!(
        AgentSpendDecision::decide(&price, false, &origin(0, false), 0, &allowed),
        AgentSpendDecision::Send {
            money: cents(0),
            limit: None
        }
    );
    assert_eq!(
        AgentSpendDecision::decide(&price, true, &origin(0, true), 0, &unlimited),
        AgentSpendDecision::Send {
            money: cents(0),
            limit: None
        }
    );
}

/// Once a request for the allowance was charged anyway today, agents' requests for it stop trusting it, but only
/// while the person lets agents use it: otherwise they are held for the person anyway.
#[test]
fn the_allowance_is_trusted_until_it_charged_an_agents_request_today() {
    assert!(AgentSpendDecision::pays_unlimited(true, true, false));
    assert!(!AgentSpendDecision::pays_unlimited(true, true, true));
    assert!(AgentSpendDecision::pays_unlimited(true, false, true));
    assert!(!AgentSpendDecision::pays_unlimited(false, true, false));
}

#[test]
fn only_a_quote_or_an_exact_price_is_known() {
    let spec = bare_spec(Mode::MotionTransfer, VideoModel::genjutsu(), Resolution::P720);
    let mut job = new_job(spec.clone(), ProviderKind::ApiKey, usd("1", true));
    assert!(!job.agent_price_is_known());
    job.server_price = Some(usd("1.2", false));
    assert!(job.agent_price_is_known());
    job = new_job(spec.clone(), ProviderKind::ApiKey, usd("1", false));
    assert!(job.agent_price_is_known());
    job = new_job(spec, ProviderKind::ApiKey, Cost::UNKNOWN_USD);
    assert!(!job.agent_price_is_known());
}

/// Jobs stored before agents existed decode without one.
#[test]
fn jobs_without_an_agent_still_decode() {
    let job = new_job(
        bare_spec(Mode::MotionTransfer, VideoModel::genjutsu(), Resolution::P720),
        ProviderKind::ApiKey,
        usd("1", false),
    );
    let mut object = serde_json::to_value(&job).unwrap();
    object.as_object_mut().unwrap().remove("agent");
    let decoded: Job = serde_json::from_value(object).unwrap();
    assert_eq!(decoded.agent, None);
    let mut agent_job = job;
    agent_job.agent = Some(origin(5, false));
    let back: Job = serde_json::from_str(&serde_json::to_string(&agent_job).unwrap()).unwrap();
    assert_eq!(back.agent, Some(origin(5, false)));
}
