//! What the connection has left.

use rust_decimal::Decimal;

/// What the connection has left.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Balance {
    /// Dollars.
    Usd(Decimal),
    /// Credits; `plan` is the subscription the credits come with, as the provider names it.
    Credits {
        /// The credits.
        credits: i64,
        /// The plan.
        plan: Option<String>,
    },
}
