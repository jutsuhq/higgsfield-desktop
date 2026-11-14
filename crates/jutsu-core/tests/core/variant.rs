//! `VariantTests.swift`.

use jutsu_core::*;

use crate::common::*;

#[test]
fn credits_split_into_whole_shares_that_add_up() {
    for (total, parts, shares) in [
        (10, 3, vec![4, 3, 3]),
        (12, 4, vec![3, 3, 3, 3]),
        (5, 1, vec![5]),
        (1, 2, vec![1, 0]),
    ] {
        let split = Cost::credits(total.into(), false).split(parts);
        let expected: Vec<Cost> = shares
            .into_iter()
            .map(|share: i64| Cost::credits(share.into(), false))
            .collect();
        assert_eq!(split, expected, "{total} / {parts}");
    }
}

#[test]
fn fractional_credits_split_evenly() {
    assert_eq!(
        Cost::credits(dec("1.5"), false).split(3),
        vec![Cost::credits(dec("0.5"), false); 3]
    );
}

#[test]
fn dollars_split_evenly() {
    let split = usd("3.9", true).split(3);
    assert_eq!(split.len(), 3);
    assert!(split.iter().all(Cost::is_estimate));
    assert_eq!(
        split.iter().map(Cost::amount).sum::<rust_decimal::Decimal>(),
        dec("3.9")
    );
    assert!(usd("1", false).split(0).is_empty());
}

#[test]
fn a_price_times_the_variants() {
    assert_eq!(credits("14", true).times(3), credits("42", true));
    assert_eq!(usd("1.5", false).times(2), usd("3", false));
}

#[test]
fn a_spec_stored_before_variants_makes_one() {
    let mut spec = make_spec();
    spec.variants = 3;
    let mut object = serde_json::to_value(&spec).unwrap();
    assert_eq!(object.as_object_mut().unwrap().remove("variants"), Some(3.into()));

    let decoded: JobSpec = serde_json::from_value(object).unwrap();

    assert_eq!(decoded.variants, 1);
    let round_trip: JobSpec = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
    assert_eq!(round_trip.variants, 3);
}
