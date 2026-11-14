//! Swift's `String`-backed enums: each case has a stored name, and the cases are listed in order.

/// Declares an enum whose cases are coded by their raw values, with `ALL`, `raw_value`, `from_raw` and `Display`.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident = $raw:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
        #[cfg_attr(feature = "specta", derive(specta::Type))]
        $vis enum $name {
            $( $(#[$vmeta])* #[serde(rename = $raw)] $variant ),+
        }

        impl $name {
            /// Every case, in order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The stored name.
            pub const fn raw_value(self) -> &'static str {
                match self { $(Self::$variant => $raw),+ }
            }

            /// The case stored as `raw`.
            pub fn from_raw(raw: &str) -> Option<Self> {
                match raw { $($raw => Some(Self::$variant),)+ _ => None }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.raw_value())
            }
        }
    };
}

pub(crate) use string_enum;
