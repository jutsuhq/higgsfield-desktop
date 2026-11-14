//! Swift's `ClosedRange`, which the stored values and the rules use for lengths, selections and counts.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A range that includes both bounds, coded as Swift codes it: `[lower, upper]`. Decoding refuses a lower bound
/// above the upper one, as Swift's decoder does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    try_from = "(T, T)",
    bound(deserialize = "T: Deserialize<'de> + Copy + PartialOrd")
)]
pub struct ClosedRange<T>(T, T);

impl<T: Copy + PartialOrd> ClosedRange<T> {
    /// `lower...upper`. Callers pass `lower <= upper`; nothing checks it here, as with Swift's
    /// `ClosedRange(uncheckedBounds:)`.
    pub const fn new(lower: T, upper: T) -> Self {
        Self(lower, upper)
    }

    /// The smallest value in the range.
    pub const fn lower(&self) -> T {
        self.0
    }

    /// The largest value in the range.
    pub const fn upper(&self) -> T {
        self.1
    }

    /// Whether `value` lies within both bounds.
    pub fn contains(&self, value: T) -> bool {
        self.0 <= value && value <= self.1
    }

    /// This range inside `limits`, as Swift's `clamped(to:)`: each bound moved into `limits`.
    #[must_use]
    pub fn clamped(&self, limits: Self) -> Self {
        let lower = if limits.0 > self.0 {
            limits.0
        } else if self.0 > limits.1 {
            limits.1
        } else {
            self.0
        };
        let upper = if limits.1 < self.1 {
            limits.1
        } else if self.1 < limits.0 {
            limits.0
        } else {
            self.1
        };
        Self(lower, upper)
    }
}

impl<T: Copy + PartialOrd> TryFrom<(T, T)> for ClosedRange<T> {
    type Error = &'static str;

    fn try_from((lower, upper): (T, T)) -> Result<Self, Self::Error> {
        if lower > upper {
            return Err("Cannot initialize ClosedRange with a lowerBound greater than its upperBound");
        }
        Ok(Self(lower, upper))
    }
}

impl fmt::Display for ClosedRange<f64> {
    /// "4.0...30.0", as Swift describes a range of seconds.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}...{}",
            crate::support::text::swift_double(self.0),
            crate::support::text::swift_double(self.1)
        )
    }
}

impl fmt::Display for ClosedRange<usize> {
    /// "1...8".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}...{}", self.0, self.1)
    }
}

#[cfg(test)]
mod tests {
    use super::ClosedRange;

    #[test]
    fn codes_as_a_pair_and_refuses_a_reversed_one() {
        let range = ClosedRange::new(0.0, 8.0);
        assert_eq!(serde_json::to_string(&range).unwrap(), "[0.0,8.0]");
        assert_eq!(serde_json::from_str::<ClosedRange<f64>>("[0, 8]").unwrap(), range);
        assert!(serde_json::from_str::<ClosedRange<i64>>("[3, 2]").is_err());
    }

    #[test]
    fn clamping_moves_each_bound_inside() {
        let window = ClosedRange::new(3.0, 10.0);
        assert_eq!(
            ClosedRange::new(3.0, 15.0).clamped(window),
            ClosedRange::new(3.0, 10.0)
        );
        assert_eq!(
            ClosedRange::new(12.0, 15.0).clamped(window),
            ClosedRange::new(10.0, 10.0)
        );
        assert_eq!(
            ClosedRange::new(1.0, 2.0).clamped(window),
            ClosedRange::new(3.0, 3.0)
        );
    }
}
