//! The price of a job, in dollars or credits.

use std::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::provider::CostUnit;

/// Price of a job. A job is paid in one unit, never mixed. Credits are exact too: an image can cost 0.25 credits.
///
/// Coded as `{"usd":{"_0":"3.816","isEstimate":false}}`: the amount is a string, so it round-trips exactly. Whole
/// credits stay a number, as they were when credits were whole only. Libraries saved before the string format hold
/// a number, which reads through its shortest decimal text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "specta", specta(type = CostRepr))]
#[serde(into = "CostRepr", try_from = "CostRepr")]
pub enum Cost {
    /// Dollars.
    Usd {
        /// The amount.
        amount: Decimal,
        /// A list price or an estimate rather than the provider's exact price.
        is_estimate: bool,
    },
    /// Credits of the account.
    Credits {
        /// The amount.
        amount: Decimal,
        /// A list price or an estimate rather than the provider's exact price.
        is_estimate: bool,
    },
}

impl Cost {
    /// `amount` dollars.
    pub const fn usd(amount: Decimal, is_estimate: bool) -> Self {
        Self::Usd { amount, is_estimate }
    }

    /// `amount` credits.
    pub const fn credits(amount: Decimal, is_estimate: bool) -> Self {
        Self::Credits { amount, is_estimate }
    }

    /// The unit it is paid in.
    pub fn unit(&self) -> CostUnit {
        match self {
            Self::Usd { .. } => CostUnit::Usd,
            Self::Credits { .. } => CostUnit::Credits,
        }
    }

    /// Whether it is a list price or an estimate.
    pub fn is_estimate(&self) -> bool {
        match self {
            Self::Usd { is_estimate, .. } | Self::Credits { is_estimate, .. } => *is_estimate,
        }
    }

    /// Dollars or credits, as `unit` says.
    pub fn amount(&self) -> Decimal {
        match self {
            Self::Usd { amount, .. } | Self::Credits { amount, .. } => *amount,
        }
    }

    /// The same unit and estimate flag with another amount.
    #[must_use]
    pub fn with_amount(&self, amount: Decimal) -> Self {
        match self {
            Self::Usd { is_estimate, .. } => Self::usd(amount, *is_estimate),
            Self::Credits { is_estimate, .. } => Self::credits(amount, *is_estimate),
        }
    }

    /// The price of each of `parts` jobs that share this one price, in order. Whole credits split as evenly as whole
    /// credits allow, the first parts taking the remainder, so the parts always add up to the whole; a fractional
    /// amount splits evenly. Empty for fewer than 1 part.
    pub fn split(&self, parts: i64) -> Vec<Self> {
        let Ok(count) = usize::try_from(parts) else {
            return Vec::new();
        };
        if count == 0 {
            return Vec::new();
        }
        if let Self::Credits { amount, is_estimate } = self
            && let Some(whole) = whole(*amount)
        {
            let (share, remainder) = (whole.div_euclid(parts), whole.rem_euclid(parts));
            return (0..parts)
                .map(|index| Self::credits(Decimal::from(share + i64::from(index < remainder)), *is_estimate))
                .collect();
        }
        let share = self
            .amount()
            .checked_div(Decimal::from(parts))
            .unwrap_or_default();
        vec![self.with_amount(share); count]
    }

    /// This price for `count` jobs of the same request.
    #[must_use]
    pub fn times(&self, count: i64) -> Self {
        self.with_amount(self.amount().saturating_mul(Decimal::from(count)))
    }
}

/// The amount as a whole number, when it is one.
fn whole(amount: Decimal) -> Option<i64> {
    if amount.fract().is_zero() {
        amount.to_i64()
    } else {
        None
    }
}

/// `Decimal(shortestOf:)`: through the shortest decimal text of `value`, so 0.318 stays exact; `None` for a value
/// that isn't finite or doesn't fit.
pub fn decimal_shortest_of(value: f64) -> Option<Decimal> {
    if !value.is_finite() {
        return None;
    }
    let text = value.to_string();
    Decimal::from_str(&text)
        .or_else(|_| Decimal::from_scientific(&text))
        .ok()
}

/// Swift's `Decimal.description`: the shortest text, without trailing zeros.
pub fn decimal_text(amount: Decimal) -> String {
    amount.normalize().to_string()
}

/// The stored shape of a `Cost`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase", rename = "Cost")]
pub enum CostRepr {
    /// Dollars.
    Usd(CostPayload),
    /// Credits.
    Credits(CostPayload),
}

/// The payload of a stored `Cost`.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct CostPayload {
    /// The amount.
    #[serde(rename = "_0")]
    pub amount: StoredAmount,
    /// Whether it is an estimate.
    pub is_estimate: bool,
}

/// A stored amount: decimal text, or a number in libraries saved before the text form, and for whole credits.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(untagged)]
pub enum StoredAmount {
    /// Decimal text.
    Text(String),
    /// A whole number.
    Whole(i64),
    /// Any other number.
    Number(f64),
}

impl StoredAmount {
    fn decimal(self) -> Result<Decimal, String> {
        match self {
            Self::Text(text) => Decimal::from_str(&text)
                .or_else(|_| Decimal::from_scientific(&text))
                .map_err(|_| format!("Not a decimal: {text}")),
            Self::Whole(whole) => Ok(Decimal::from(whole)),
            Self::Number(number) => {
                decimal_shortest_of(number).ok_or_else(|| format!("Not a finite amount: {number}"))
            }
        }
    }
}

impl From<Cost> for CostRepr {
    fn from(cost: Cost) -> Self {
        match cost {
            Cost::Usd { amount, is_estimate } => Self::Usd(CostPayload {
                amount: StoredAmount::Text(decimal_text(amount)),
                is_estimate,
            }),
            Cost::Credits { amount, is_estimate } => {
                let amount = whole(amount)
                    .map_or_else(|| StoredAmount::Text(decimal_text(amount)), StoredAmount::Whole);
                Self::Credits(CostPayload { amount, is_estimate })
            }
        }
    }
}

impl TryFrom<CostRepr> for Cost {
    type Error = String;

    fn try_from(repr: CostRepr) -> Result<Self, Self::Error> {
        Ok(match repr {
            CostRepr::Usd(payload) => Self::usd(payload.amount.decimal()?, payload.is_estimate),
            CostRepr::Credits(payload) => Self::credits(payload.amount.decimal()?, payload.is_estimate),
        })
    }
}
