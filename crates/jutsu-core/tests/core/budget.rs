//! `BudgetTests.swift`.

use std::collections::{BTreeMap, BTreeSet};

use jutsu_core::*;
use time::{Duration, OffsetDateTime, UtcOffset};

use crate::common::*;

#[allow(clippy::unnecessary_wraps, reason = "the cap is optional where it is passed")]
fn usd_cap(amount: &str) -> Option<SpendCap> {
    Some(SpendCap::usd(dec(amount)))
}

/// Under its cap, in its unit, a price goes; over it, the person confirms the new total. A cap never limits the other
/// unit, and an unknown price under a credit cap needs the person.
#[test]
fn a_price_over_the_cap_in_its_unit_needs_confirmation() {
    let allowed = BudgetDecision::Allowed;
    let confirm = |total: &str| BudgetDecision::NeedsConfirmation { total: dec(total) };
    let credit_cap = SpendCap::credits(500);
    for (price, committed, cap, expected) in [
        (usd("5", false), "70", usd_cap("75"), allowed),
        (usd("5.01", false), "70", usd_cap("75"), confirm("75.01")),
        (usd("1000", false), "1000", None, allowed),
        (credits("100", true), "400", Some(credit_cap), allowed),
        (credits("101", true), "400", Some(credit_cap), confirm("501")),
        (credits("1000", true), "1000", usd_cap("1"), allowed),
        (usd("1000", false), "1000", Some(SpendCap::credits(1)), allowed),
        (
            Cost::UNKNOWN_CREDITS,
            "0",
            Some(SpendCap::credits(100)),
            confirm("0"),
        ),
        (Cost::UNKNOWN_CREDITS, "0", None, allowed),
        (Cost::UNKNOWN_USD, "0", Some(SpendCap::credits(100)), allowed),
    ] {
        assert_eq!(
            Budget::allows(&price, dec(committed), cap.as_ref()),
            expected,
            "{price:?} on {committed} under {cap:?}"
        );
    }
}

#[test]
fn refined_price_over_the_cap_needs_the_user_only_when_the_preliminary_fit() {
    let check = |committed: &str, approved: Option<Cost>, cap: Option<SpendCap>| {
        Budget::refined_price_needs_confirmation(
            &usd("4", false),
            &usd("6", false),
            approved.as_ref(),
            dec(committed),
            cap.as_ref(),
        )
    };
    assert!(check("70", None, usd_cap("75")));
    assert!(!check("60", None, usd_cap("75")));
    assert!(
        !check("72", None, usd_cap("75")),
        "the user already confirmed the preliminary price"
    );
    assert!(!check("70", None, None));
    assert!(!check("70", Some(usd("6", false)), usd_cap("75")));
    assert!(check("70", Some(usd("5", false)), usd_cap("75")));
    assert!(check("70", Some(credits("100", false)), usd_cap("75")));
    assert!(
        check("72", Some(usd("4", false)), usd_cap("75")),
        "agreed to 4 over the cap, not 6"
    );
}

#[test]
fn jobs_paid_from_the_unlimited_allowance_dont_count() {
    let mut credit_job = new_job(make_spec(), ProviderKind::Account, credits("40", true));
    credit_job.state = JobState::Processing;
    let mut unlimited = credit_job.clone();
    unlimited.spec.uses_unlimited = true;
    assert_eq!(
        Budget::committed(
            &[credit_job, unlimited],
            OffsetDateTime::now_utc(),
            CostUnit::Credits,
            UtcOffset::UTC
        ),
        dec("40")
    );
}

#[test]
fn only_an_opted_in_spec_the_allowance_covers_pays_unlimited() {
    let mut capabilities = Capabilities::new(
        BTreeSet::from([Mode::MotionTransfer, Mode::ObjectSwap]),
        BTreeSet::from([Resolution::P480]),
        CostUnit::Credits,
        None,
    );
    capabilities.unlimited =
        BTreeMap::from([(VideoModel::genjutsu(), BTreeSet::from([Mode::MotionTransfer]))]);
    assert!(capabilities.pays_unlimited(&make_spec(), true));
    assert!(!capabilities.pays_unlimited(&make_spec(), false));
    assert!(!capabilities.pays_unlimited(
        &spec_with(SpecArgs {
            mode: Mode::ObjectSwap,
            ..SpecArgs::default()
        }),
        true
    ));
}

#[test]
fn failed_jobs_that_may_have_been_charged_count_on_the_day_they_ended() {
    let day = OffsetDateTime::from_unix_timestamp(1_791_892_800).unwrap();
    let failed = |change: &dyn Fn(&mut Job)| {
        let mut job = new_job(make_spec(), ProviderKind::Account, credits("10", true));
        job.state = JobState::Failed {
            reason: FailureReason::Message("x".into()),
        };
        job.finished_at = Some(day);
        change(&mut job);
        job
    };
    let unknown = failed(&|job| job.submit_attempt = Some(SubmitAttempt { is_idempotent: false }));
    let past_deadline = failed(&|job| {
        job.state = JobState::Failed {
            reason: FailureReason::Deadline,
        };
        job.timed_out_at = Some(day);
        job.remote = Some(RemoteJob::new("r1"));
    });
    let refused = failed(&|_| {});
    let charged_the_day_before = failed(&|job| {
        job.charged_at = Some(day - Duration::hours(24));
        job.remote = Some(RemoteJob::new("r2"));
    });
    assert_eq!(
        Budget::committed(
            &[unknown, past_deadline, refused, charged_the_day_before],
            day,
            CostUnit::Credits,
            UtcOffset::UTC
        ),
        dec("20")
    );
}

/// A price the person agreed to covers the same price or a lower one in its unit, and nothing in another unit.
#[test]
fn an_approved_price_covers_no_more_than_itself() {
    let approved = credits("20", false);
    assert!(Budget::covers(Some(&approved), &credits("20", true)));
    assert!(Budget::covers(Some(&approved), &credits("12", false)));
    assert!(!Budget::covers(Some(&approved), &credits("21", false)));
    assert!(!Budget::covers(Some(&approved), &usd("1", false)));
    assert!(!Budget::covers(None, &credits("0", false)));
}
