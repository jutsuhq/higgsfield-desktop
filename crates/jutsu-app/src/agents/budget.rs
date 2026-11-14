//! The agents' wire amounts from the app's own values and back (`AgentWire+Domain.swift`), today's agent budget as
//! `capabilities` reports it, and the person's own daily cap as agents read it (`AgentCap.swift`).

use jutsu_agent::{AgentAmount, AgentBudgetUnit, Budget};
use jutsu_core::{AgentMoney, AgentMoneyUnit, AgentSpendTotal, Cost, CostUnit, ProviderKind, SpendCap};
use rust_decimal::Decimal;

/// What `kind`'s agent budget counts in: cents with an API key, credits on an account.
pub fn unit_for(kind: ProviderKind) -> AgentBudgetUnit {
    wire_unit(AgentMoney::unit_for(kind))
}

pub fn wire_unit(unit: AgentMoneyUnit) -> AgentBudgetUnit {
    match unit {
        AgentMoneyUnit::UsdCents => AgentBudgetUnit::UsdCents,
        AgentMoneyUnit::Credits => AgentBudgetUnit::Credits,
    }
}

pub fn money_unit(unit: AgentBudgetUnit) -> AgentMoneyUnit {
    match unit {
        AgentBudgetUnit::UsdCents => AgentMoneyUnit::UsdCents,
        AgentBudgetUnit::Credits => AgentMoneyUnit::Credits,
    }
}

pub fn amount(money: AgentMoney) -> AgentAmount {
    AgentAmount::new(money.amount, wire_unit(money.unit))
}

pub fn money(amount: AgentAmount) -> AgentMoney {
    AgentMoney::new(amount.amount, money_unit(amount.unit))
}

/// Today's agent budget on `kind`'s connection as `capabilities` reports it: what is left is counted as the spending
/// guard and `quote` count it (`AgentSpendTotal::remaining`), and the plan's unlimited generations are offered only
/// while none of them was charged anyway today.
pub fn budget(
    kind: ProviderKind,
    daily_limit: i64,
    taken: AgentSpendTotal,
    allows_unlimited: bool,
    daily_cap: Option<i64>,
    cap_taken_today: Option<i64>,
) -> Budget {
    Budget {
        unit: unit_for(kind),
        daily_limit: daily_limit.max(0),
        spent_today: taken.spent.max(0),
        reserved_today: taken.reserved.max(0),
        remaining: taken.remaining(daily_limit),
        unlimited_allowed: allows_unlimited && !taken.unlimited_charged,
        daily_cap,
        cap_taken_today,
    }
}

/// `amount` of `unit` in whole units, rounded up as every agent amount is; `None` for one that isn't a number.
pub fn cap_money(amount: Decimal, unit: CostUnit) -> Option<AgentMoney> {
    let cost = match unit {
        CostUnit::Usd => Cost::usd(amount, false),
        CostUnit::Credits => Cost::credits(amount, false),
    };
    AgentMoney::from_cost(&cost)
}

/// `120 usd_cents`, `24 credits`.
pub fn money_text(money: AgentMoney) -> String {
    format!("{} {}", money.amount, wire_unit(money.unit).as_str())
}

/// What `over_cap` says: the cap, what it counts today besides the job, and what the job was counted at.
pub fn over_cap_message(cap: &SpendCap, committed_today: Decimal, price: &Cost) -> String {
    let text = |amount: Decimal| {
        cap_money(amount, cap.unit).map_or_else(|| "an unknown amount".to_owned(), money_text)
    };
    let counted = if price.is_unknown_price() {
        "an unknown price, which the cap never lets pass".to_owned()
    } else {
        text(price.amount())
    };
    format!(
        "The job would take today's spending over the person's daily cap of {}: {} are spent or queued today, the \
         person's jobs and agents' together, and the job counts at {counted}. The cap is apart from the agent budget.",
        text(cap.amount),
        text(committed_today)
    )
}

/// `capTakenToday`: today's spend toward the person's cap in its unit; `None` without a cap, or when the spend can't
/// be read (`try? await committedToday`), never 0 for a spend that isn't known.
pub fn cap_taken<E>(cap: Option<&SpendCap>, committed_today: Result<Decimal, E>) -> Option<i64> {
    let cap = cap?;
    let committed = committed_today.ok()?;
    cap_money(committed, cap.unit).map(|money| money.amount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros_free::dec;

    #[test]
    fn a_spend_that_cant_be_read_leaves_cap_taken_out() {
        let cap = SpendCap::credits(300);
        assert_eq!(cap_taken(Some(&cap), Ok::<_, String>(dec("12"))), Some(12));
        assert_eq!(cap_taken(Some(&cap), Err("unreadable")), None);
        assert_eq!(cap_taken(None, Ok::<_, String>(dec("12"))), None);
    }

    /// `Decimal` from text, for the cases below.
    mod rust_decimal_macros_free {
        pub fn dec(text: &str) -> rust_decimal::Decimal {
            text.parse().unwrap()
        }
    }

    fn total(spent: i64, reserved: i64, unlimited_charged: bool) -> AgentSpendTotal {
        AgentSpendTotal {
            spent,
            reserved,
            unlimited_charged,
        }
    }

    #[test]
    fn the_budget_counts_reservations_and_never_overflows() {
        let budget = budget(
            ProviderKind::Account,
            100,
            total(30, 50, false),
            false,
            None,
            None,
        );
        assert_eq!(budget.unit, AgentBudgetUnit::Credits);
        assert_eq!((budget.spent_today, budget.reserved_today), (30, 50));
        assert_eq!(budget.remaining, 20);
        let huge = super::budget(
            ProviderKind::Account,
            100,
            total(i64::MAX, i64::MAX, false),
            false,
            None,
            None,
        );
        assert_eq!(huge.remaining, 0);
    }

    #[test]
    fn the_budget_never_goes_below_zero() {
        let over = budget(
            ProviderKind::Account,
            100,
            total(140, 0, false),
            false,
            None,
            None,
        );
        assert_eq!(over.remaining, 0);
        let negative = budget(ProviderKind::ApiKey, -5, total(-1, 0, false), false, None, None);
        assert_eq!(negative.unit, AgentBudgetUnit::UsdCents);
        assert_eq!((negative.daily_limit, negative.spent_today), (0, 0));
        assert_eq!(negative.remaining, 0);
    }

    /// Once the allowance didn't pay for an agent's job today, agents aren't told they may use it.
    #[test]
    fn unlimited_is_offered_only_while_none_was_charged_today() {
        assert!(budget(ProviderKind::Account, 0, total(0, 0, false), true, None, None).unlimited_allowed);
        assert!(!budget(ProviderKind::Account, 0, total(0, 0, true), true, None, None).unlimited_allowed);
    }

    #[test]
    fn amounts_round_trip_and_units_follow_the_connection() {
        let credits = AgentMoney::new(250, AgentMoneyUnit::Credits);
        assert_eq!(money(amount(credits)), credits);
        assert_eq!(unit_for(ProviderKind::ApiKey), AgentBudgetUnit::UsdCents);
        assert_eq!(unit_for(ProviderKind::Account), AgentBudgetUnit::Credits);
    }

    /// `over_cap` says what the cap counted, in the whole units agents read, and that it isn't the agent budget.
    #[test]
    fn over_cap_gives_the_cap_today_and_the_jobs_price() {
        let credits = over_cap_message(
            &SpendCap::credits(300),
            dec("290"),
            &Cost::credits(dec("24"), false),
        );
        assert!(credits.contains("daily cap of 300 credits"));
        assert!(credits.contains("290 credits are spent or queued"));
        assert!(credits.contains("counts at 24 credits"));
        assert!(credits.contains("apart from the agent budget"));
        let dollars = over_cap_message(
            &SpendCap::usd(dec("5")),
            dec("4.5"),
            &Cost::usd(dec("1.301"), true),
        );
        assert!(dollars.contains("500 usd_cents") && dollars.contains("450 usd_cents"));
        assert!(dollars.contains("counts at 131 usd_cents"));
        assert!(
            over_cap_message(&SpendCap::usd(dec("5")), Decimal::ZERO, &Cost::UNKNOWN_USD)
                .contains("unknown price")
        );
    }
}
