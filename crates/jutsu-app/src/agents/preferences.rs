//! Settings › Agents as the person saved it (`AgentPreferences.swift`). The values live in `preferences.json` under the
//! macOS app's keys; only the app's own Settings writes that file, so nothing that launches the app can turn agents on
//! or give them a budget by itself.

use std::sync::Arc;

use jutsu_agent::{AgentAmount, AgentBudgetUnit};
use jutsu_core::ProviderKind;
use serde_json::Value;

use crate::preferences::{PreferencesError, PreferencesStore};

/// Settings › Agents, read from and written to the preferences file.
#[derive(Debug, Clone)]
pub struct AgentPreferences {
    store: Arc<PreferencesStore>,
}

impl AgentPreferences {
    pub const ALLOW_KEY: &'static str = "agents.allow";
    /// Integer cents, for an API key.
    pub const BUDGET_CENTS_KEY: &'static str = "agents.dailyBudgetCents";
    /// Whole credits, for an account.
    pub const BUDGET_CREDITS_KEY: &'static str = "agents.dailyBudgetCredits";
    /// Let agents use unlimited generations: whether an agent's job the plan's unlimited allowance would pay for goes
    /// out without the person.
    pub const UNLIMITED_KEY: &'static str = "agents.allowUnlimited";

    pub fn new(store: Arc<PreferencesStore>) -> Self {
        Self { store }
    }

    /// The most a daily budget may be: $10,000, or ten million credits, the most an agent's `max_cost` may be too
    /// (`AgentAmount::maximum`). Whatever is stored is read within these, so no sum of budgets and prices can overflow.
    pub fn max_budget_cents() -> i64 {
        AgentAmount::maximum(AgentBudgetUnit::UsdCents)
    }

    pub fn max_budget_credits() -> i64 {
        AgentAmount::maximum(AgentBudgetUnit::Credits)
    }

    /// Off until the person turns it on.
    pub fn is_allowed(&self) -> bool {
        self.store
            .value(Self::ALLOW_KEY)
            .and_then(|value| value.as_bool())
            == Some(true)
    }

    /// Off until the person turns it on: until then an agent's job the plan's unlimited allowance would pay for waits
    /// in the Queue for them, as one over the budget does.
    pub fn allows_unlimited(&self) -> bool {
        self.store
            .value(Self::UNLIMITED_KEY)
            .and_then(|value| value.as_bool())
            == Some(true)
    }

    /// What agents may spend a day on `connection`, in its whole units: cents with an API key, credits on an
    /// account. 0 until the person sets one, so agents can look but not spend.
    pub fn daily_budget(&self, connection: ProviderKind) -> i64 {
        let (key, limit) = Self::budget_key(connection);
        Self::read_budget(self.store.value(key).as_ref(), limit)
    }

    /// A stored number read within 0 and `limit`, rounded down; anything else is 0.
    fn read_budget(value: Option<&Value>, limit: i64) -> i64 {
        let Some(number) = value.and_then(Value::as_f64) else {
            return 0;
        };
        if !number.is_finite() {
            return 0;
        }
        #[allow(clippy::cast_precision_loss, reason = "the limits are far below 2^53")]
        let clamped = number.floor().clamp(0.0, limit as f64);
        #[allow(clippy::cast_possible_truncation, reason = "clamped to the limit first")]
        let whole = clamped as i64;
        whole
    }

    pub fn set_allowed(&self, is_allowed: bool) -> Result<(), PreferencesError> {
        self.store
            .set_value(Self::ALLOW_KEY, Some(Value::Bool(is_allowed)))
    }

    pub fn set_allows_unlimited(&self, allows: bool) -> Result<(), PreferencesError> {
        self.store
            .set_value(Self::UNLIMITED_KEY, Some(Value::Bool(allows)))
    }

    /// Stores `amount` within the most for `connection`.
    pub fn set_daily_budget(&self, amount: i64, connection: ProviderKind) -> Result<(), PreferencesError> {
        let (key, limit) = Self::budget_key(connection);
        self.store
            .set_value(key, Some(Value::from(amount.clamp(0, limit))))
    }

    pub fn budget_key(connection: ProviderKind) -> (&'static str, i64) {
        match connection {
            ProviderKind::ApiKey => (Self::BUDGET_CENTS_KEY, Self::max_budget_cents()),
            ProviderKind::Account => (Self::BUDGET_CREDITS_KEY, Self::max_budget_credits()),
        }
    }

    // MARK: Text

    /// A budget as typed: dollars with at most two decimals ("12", "12.5", "$1,000.00") for an API key, whole credits
    /// ("1000", "1,000 credits") for an account; 0 is a budget too. `None` for anything else or above the most, and
    /// the field keeps the last value. Read digit by digit, never through a floating-point number, so no input can
    /// overflow or round.
    pub fn parse_budget(text: &str, connection: ProviderKind) -> Option<i64> {
        let mut cleaned: String = text
            .to_lowercase()
            .chars()
            .filter(|character| !character.is_whitespace() && *character != ',')
            .collect();
        match connection {
            ProviderKind::ApiKey => {
                if let Some(rest) = cleaned.strip_prefix('$') {
                    cleaned = rest.to_owned();
                }
                let mut parts = cleaned.splitn(2, '.');
                let dollars = digits(parts.next()?, 7, false)?;
                let mut cents = 0;
                if let Some(fraction) = parts.next() {
                    let length = fraction.chars().count();
                    if length > 2 {
                        return None;
                    }
                    let value = digits(fraction, 2, true)?;
                    cents = if length == 1 { value * 10 } else { value };
                }
                let total = dollars * 100 + cents;
                (total <= Self::max_budget_cents()).then_some(total)
            }
            ProviderKind::Account => {
                for suffix in ["credits", "credit", "cr"] {
                    if let Some(rest) = cleaned.strip_suffix(suffix) {
                        cleaned = rest.to_owned();
                        break;
                    }
                }
                let credits = digits(&cleaned, 8, false)?;
                (credits <= Self::max_budget_credits()).then_some(credits)
            }
        }
    }

    /// The text `parse_budget` reads back: "12.50" or "1000".
    pub fn budget_text(amount: i64, connection: ProviderKind) -> String {
        match connection {
            ProviderKind::ApiKey => format!("{}.{:02}", amount / 100, amount % 100),
            ProviderKind::Account => amount.to_string(),
        }
    }
}

/// The value of up to `count` ASCII digits; `None` for none (unless `allows_empty`), more, or anything else.
fn digits(text: &str, count: usize, allows_empty: bool) -> Option<i64> {
    let length = text.chars().count();
    if length > count || (!allows_empty && length == 0) {
        return None;
    }
    let mut value = 0_i64;
    for character in text.chars() {
        let digit = character.to_digit(10).filter(|_| character.is_ascii_digit())?;
        value = value * 10 + i64::from(digit);
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preferences() -> (tempfile::TempDir, Arc<PreferencesStore>, AgentPreferences) {
        let folder = tempfile::tempdir().unwrap();
        let store = Arc::new(PreferencesStore::open(folder.path()));
        let preferences = AgentPreferences::new(store.clone());
        (folder, store, preferences)
    }

    #[test]
    fn agents_are_off_with_no_budget_until_the_person_says_otherwise() {
        let (_folder, _store, preferences) = preferences();
        assert!(!preferences.is_allowed());
        assert_eq!(preferences.daily_budget(ProviderKind::ApiKey), 0);
        assert_eq!(preferences.daily_budget(ProviderKind::Account), 0);
    }

    #[test]
    fn what_settings_saved_is_read() {
        let (_folder, store, preferences) = preferences();
        store
            .set_value(AgentPreferences::ALLOW_KEY, Some(Value::Bool(true)))
            .unwrap();
        store
            .set_value(AgentPreferences::BUDGET_CENTS_KEY, Some(Value::from(1250)))
            .unwrap();
        store
            .set_value(AgentPreferences::BUDGET_CREDITS_KEY, Some(Value::from(300)))
            .unwrap();
        assert!(preferences.is_allowed());
        assert_eq!(preferences.daily_budget(ProviderKind::ApiKey), 1250);
        assert_eq!(preferences.daily_budget(ProviderKind::Account), 300);
    }

    #[test]
    fn settings_writes_what_it_reads_and_survives_a_relaunch() {
        let (folder, _store, preferences) = preferences();
        preferences.set_allowed(true).unwrap();
        preferences
            .set_daily_budget(i64::MAX, ProviderKind::ApiKey)
            .unwrap();
        preferences.set_daily_budget(-5, ProviderKind::Account).unwrap();
        assert!(preferences.is_allowed());
        assert_eq!(
            preferences.daily_budget(ProviderKind::ApiKey),
            AgentPreferences::max_budget_cents()
        );
        assert_eq!(preferences.daily_budget(ProviderKind::Account), 0);
        let reopened = AgentPreferences::new(Arc::new(PreferencesStore::open(folder.path())));
        assert!(reopened.is_allowed());
        assert_eq!(
            reopened.daily_budget(ProviderKind::ApiKey),
            AgentPreferences::max_budget_cents()
        );
    }

    #[test]
    fn unlimited_generations_are_off_for_agents_until_the_person_turns_them_on() {
        let (_folder, _store, preferences) = preferences();
        assert!(!preferences.allows_unlimited());
        preferences.set_allows_unlimited(true).unwrap();
        assert!(preferences.allows_unlimited());
    }

    /// Anything but `true` is off: a value some other writer put there doesn't turn agents on.
    #[test]
    fn only_a_saved_true_turns_agents_on() {
        let (_folder, store, preferences) = preferences();
        for value in [Value::from("YES"), Value::from(1), Value::Null] {
            store.set_value(AgentPreferences::ALLOW_KEY, Some(value)).unwrap();
            assert!(!preferences.is_allowed());
        }
    }

    #[test]
    fn a_stored_budget_out_of_range_is_read_within_it() {
        let (_folder, store, preferences) = preferences();
        store
            .set_value(AgentPreferences::BUDGET_CENTS_KEY, Some(Value::from(1e300)))
            .unwrap();
        store
            .set_value(AgentPreferences::BUDGET_CREDITS_KEY, Some(Value::from(-40)))
            .unwrap();
        assert_eq!(
            preferences.daily_budget(ProviderKind::ApiKey),
            AgentPreferences::max_budget_cents()
        );
        assert_eq!(preferences.daily_budget(ProviderKind::Account), 0);
        store
            .set_value(AgentPreferences::BUDGET_CREDITS_KEY, Some(Value::from("lots")))
            .unwrap();
        assert_eq!(preferences.daily_budget(ProviderKind::Account), 0);
        store
            .set_value(AgentPreferences::BUDGET_CREDITS_KEY, Some(Value::from(12.9)))
            .unwrap();
        assert_eq!(preferences.daily_budget(ProviderKind::Account), 12);
    }

    #[test]
    fn dollars_are_read_to_the_cent() {
        let cases: [(&str, Option<i64>); 17] = [
            ("0", Some(0)),
            ("12", Some(1200)),
            ("12.5", Some(1250)),
            ("12.05", Some(1205)),
            ("$1,000.00", Some(100_000)),
            (" 7. ", Some(700)),
            ("10000", Some(1_000_000)),
            ("10000.01", None),
            ("12.345", None),
            ("-1", None),
            ("1e3", None),
            ("", None),
            ("$", None),
            (".5", None),
            ("1.2.3", None),
            ("99999999999999999999999", None),
            ("١٢", None),
        ];
        for (text, cents) in cases {
            assert_eq!(
                AgentPreferences::parse_budget(text, ProviderKind::ApiKey),
                cents,
                "{text}"
            );
        }
    }

    #[test]
    fn credits_are_whole_numbers() {
        let cases: [(&str, Option<i64>); 11] = [
            ("0", Some(0)),
            ("1000", Some(1000)),
            ("1,000 credits", Some(1000)),
            ("1 credit", Some(1)),
            ("250cr", Some(250)),
            ("10000000", Some(10_000_000)),
            ("10000001", None),
            ("12.5", None),
            ("-3", None),
            ("", None),
            ("999999999999999999999", None),
        ];
        for (text, credits) in cases {
            assert_eq!(
                AgentPreferences::parse_budget(text, ProviderKind::Account),
                credits,
                "{text}"
            );
        }
    }

    #[test]
    fn the_fields_text_reads_back() {
        for cents in [0, 5, 1250, 1_000_000] {
            let text = AgentPreferences::budget_text(cents, ProviderKind::ApiKey);
            assert_eq!(
                AgentPreferences::parse_budget(&text, ProviderKind::ApiKey),
                Some(cents)
            );
        }
        assert_eq!(AgentPreferences::budget_text(1205, ProviderKind::ApiKey), "12.05");
        assert_eq!(
            AgentPreferences::parse_budget(
                &AgentPreferences::budget_text(4200, ProviderKind::Account),
                ProviderKind::Account
            ),
            Some(4200)
        );
    }

    /// The numbers agents and the person see: changing them is a decision, not a refactor.
    #[test]
    fn the_most_a_budget_or_a_max_cost_can_be_is_pinned() {
        assert_eq!(AgentAmount::maximum(AgentBudgetUnit::UsdCents), 1_000_000);
        assert_eq!(AgentAmount::maximum(AgentBudgetUnit::Credits), 10_000_000);
        assert_eq!(
            AgentPreferences::budget_key(ProviderKind::ApiKey).1,
            AgentAmount::maximum(crate::agents::budget::unit_for(ProviderKind::ApiKey))
        );
        assert_eq!(
            AgentPreferences::budget_key(ProviderKind::Account).1,
            AgentAmount::maximum(crate::agents::budget::unit_for(ProviderKind::Account))
        );
    }
}
