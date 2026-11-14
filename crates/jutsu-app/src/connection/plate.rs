//! The plate above Settings in the sidebar (`AccountPlate.swift`, `LowCreditWarning.swift`).

use serde::Serialize;

use super::state::ConnectionState;

/// The rank of a Higgsfield plan, which decides how its badge looks. Read from the plan's name, since plans get
/// renamed and added more often than they change rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PlanTier {
    Free,
    Starter,
    Pro,
    Premium,
    Elite,
}

impl PlanTier {
    /// "Creator Plus" → `Elite`, "Pro" → `Pro`. An unknown paid plan counts as `Pro`.
    pub fn from_plan_title(plan_title: &str) -> Self {
        let lowered = plan_title.to_lowercase();
        let words: Vec<&str> = lowered
            .split(|character: char| !character.is_alphabetic())
            .filter(|word| !word.is_empty())
            .collect();
        let has = |any: &[&str]| words.iter().any(|word| any.contains(word));
        if has(&["ultra", "business", "team", "enterprise", "studio", "unlimited"])
            || (words.contains(&"creator") && words.contains(&"plus"))
        {
            Self::Elite
        } else if has(&["ultimate", "creator", "premium", "max"]) {
            Self::Premium
        } else if has(&["pro", "plus", "standard"]) {
            Self::Pro
        } else if has(&["basic", "starter", "start", "lite"]) {
            Self::Starter
        } else if has(&["free", "trial"]) {
            Self::Free
        } else {
            Self::Pro
        }
    }
}

/// The plate above Settings in the sidebar: the plan and what is left on it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AccountPlate {
    /// "Creator Plus", "Account" while the plan is unknown, "API key".
    pub badge: String,
    /// `None` for an API key and for an account whose plan isn't known yet: a neutral badge.
    pub tier: Option<PlanTier>,
    /// "2,596"; "—" while the credits are unknown.
    pub amount: String,
    /// "credits", or what an API key is billed in.
    pub unit: String,
    /// "Low credits: 42 left" below the threshold.
    pub warning: Option<String>,
    #[serde(rename = "isAPIKey")]
    pub is_api_key: bool,
    /// "Creator Plus plan, 2,596 credits left".
    pub accessibility_label: String,
}

impl AccountPlate {
    /// `None` without a connection.
    pub fn new(connection: &ConnectionState, low_credit_threshold: i64) -> Option<Self> {
        match connection {
            ConnectionState::None => None,
            ConnectionState::ApiKey { .. } => Some(Self {
                badge: "API key".to_owned(),
                tier: None,
                amount: "Per job".to_owned(),
                unit: "billed in USD".to_owned(),
                warning: None,
                is_api_key: true,
                accessibility_label: "API key, billed in USD per job".to_owned(),
            }),
            ConnectionState::Account(info) => {
                let tier = info.plan.as_deref().map(PlanTier::from_plan_title);
                let badge = info.plan.clone().unwrap_or_else(|| "Account".to_owned());
                let accessibility_label = match (tier, info.credits) {
                    (None, None) => "Higgsfield account".to_owned(),
                    (None, Some(credits)) => format!("Higgsfield account, {} left", credits_text(credits)),
                    (Some(_), None) => format!("{badge} plan"),
                    (Some(_), Some(credits)) => format!("{badge} plan, {} left", credits_text(credits)),
                };
                Some(Self {
                    badge,
                    tier,
                    amount: info.credits.map_or_else(|| "—".to_owned(), grouped),
                    unit: "credits".to_owned(),
                    warning: low_credit_warning(connection, low_credit_threshold),
                    is_api_key: false,
                    accessibility_label,
                })
            }
        }
    }
}

/// "Low credits: 42 left" when the account's credits are known and below `threshold`. `None` for an API key,
/// whose balance the app can't read, and while the credits are unknown.
pub fn low_credit_warning(connection: &ConnectionState, threshold: i64) -> Option<String> {
    let credits = connection.account_info()?.credits?;
    (credits < threshold).then(|| format!("Low credits: {} left", grouped(credits)))
}

/// "1,000 credits", "1 credit".
pub fn credits_text(credits: i64) -> String {
    if credits == 1 {
        "1 credit".to_owned()
    } else {
        format!("{} credits", grouped(credits))
    }
}

/// Thousands grouped with a comma, as `en_US` writes them. Only English is ported.
pub fn grouped(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if value < 0 {
        text.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::AccountInfo;

    fn account(plan: Option<&str>, credits: Option<i64>) -> ConnectionState {
        ConnectionState::Account(AccountInfo {
            plan: plan.map(str::to_owned),
            credits,
            ..AccountInfo::default()
        })
    }

    #[test]
    fn tier_follows_the_plan_name() {
        for (plan, tier) in [
            ("Free", PlanTier::Free),
            ("Basic", PlanTier::Starter),
            ("Starter", PlanTier::Starter),
            ("Pro", PlanTier::Pro),
            ("Plus", PlanTier::Pro),
            ("Ultimate", PlanTier::Premium),
            ("Creator", PlanTier::Premium),
            ("Creator Plus", PlanTier::Elite),
            ("Ultra", PlanTier::Elite),
            ("Business", PlanTier::Elite),
            ("Team Pro", PlanTier::Elite),
            ("Something New", PlanTier::Pro),
        ] {
            assert_eq!(PlanTier::from_plan_title(plan), tier, "{plan}");
        }
    }

    #[test]
    fn account_shows_plan_and_grouped_credits() {
        let plate = AccountPlate::new(&account(Some("Creator Plus"), Some(2596)), 50).unwrap();
        assert_eq!(plate.badge, "Creator Plus");
        assert_eq!(plate.tier, Some(PlanTier::Elite));
        assert_eq!(plate.amount, "2,596");
        assert_eq!(plate.unit, "credits");
        assert_eq!(plate.warning, None);
        assert_eq!(plate.accessibility_label, "Creator Plus plan, 2,596 credits left");
    }

    #[test]
    fn low_credits_warn() {
        let plate = AccountPlate::new(&account(Some("Pro"), Some(12)), 50).unwrap();
        assert_eq!(plate.warning.as_deref(), Some("Low credits: 12 left"));
    }

    #[test]
    fn unknown_plan_and_credits_are_neutral() {
        let plate = AccountPlate::new(&account(None, None), 50).unwrap();
        assert_eq!(plate.badge, "Account");
        assert_eq!(plate.tier, None);
        assert_eq!(plate.amount, "—");
        assert_eq!(plate.accessibility_label, "Higgsfield account");
        let one = AccountPlate::new(&account(None, Some(1)), 0).unwrap();
        assert_eq!(one.accessibility_label, "Higgsfield account, 1 credit left");
    }

    #[test]
    fn api_key_has_no_credits_and_no_connection_has_no_plate() {
        let plate = AccountPlate::new(&ConnectionState::ApiKey { key_id: "kid".into() }, 50).unwrap();
        assert_eq!(plate.badge, "API key");
        assert!(plate.is_api_key);
        assert_eq!(plate.tier, None);
        assert_eq!(plate.warning, None);
        assert_eq!(AccountPlate::new(&ConnectionState::None, 50), None);
    }

    #[test]
    fn grouping_and_plurals() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(-1_500), "-1,500");
        assert_eq!(credits_text(1), "1 credit");
        assert_eq!(credits_text(1_000), "1,000 credits");
    }
}
