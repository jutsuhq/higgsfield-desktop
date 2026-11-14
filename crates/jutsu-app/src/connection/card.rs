//! The rows of the Settings connection card (`SettingsContent.swift`).

use jutsu_core::{Capabilities, Mode, RequestShape, VideoModel};
use serde::Serialize;

use super::plate::grouped;
use super::state::ConnectionState;
use super::vault::credential_store_title;

/// One label/value line of the Settings connection card.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionRow {
    pub label: String,
    pub value: String,
    /// Key IDs read better in a fixed-width font.
    pub is_monospaced: bool,
}

impl ConnectionRow {
    fn new(label: &str, value: impl Into<String>) -> Self {
        Self {
            label: label.to_owned(),
            value: value.into(),
            is_monospaced: false,
        }
    }
}

/// The mode's title (`Mode.title`).
pub fn mode_title(mode: Mode) -> &'static str {
    match mode {
        Mode::MotionTransfer => "Motion transfer",
        Mode::Restyle => "Restyle",
        Mode::ObjectSwap => "Object swap",
        Mode::Edit => "Edit",
        Mode::Extend => "Extend",
    }
}

impl ConnectionState {
    /// The rows of the Settings connection card; empty when not connected. `capabilities` is `None` while they
    /// are still loading.
    pub fn card_rows(&self, capabilities: Option<&Capabilities>) -> Vec<ConnectionRow> {
        match self {
            Self::None => Vec::new(),
            Self::ApiKey { key_id } => vec![
                ConnectionRow {
                    is_monospaced: true,
                    ..ConnectionRow::new("Key ID", mask(key_id))
                },
                ConnectionRow::new("Secret", "••••••••••••"),
                ConnectionRow::new("Stored in", credential_store_title()),
            ],
            Self::Account(info) => {
                let mut rows = vec![ConnectionRow::new(
                    "Account",
                    info.email.clone().unwrap_or_else(|| "Signed in".to_owned()),
                )];
                if let Some(plan) = &info.plan {
                    rows.push(ConnectionRow::new("Plan", plan.clone()));
                }
                if let Some(credits) = info.credits {
                    rows.push(ConnectionRow::new("Credits", grouped(credits)));
                }
                rows.push(ConnectionRow::new("Genjutsu", modes_text(capabilities)));
                rows
            }
        }
    }
}

/// The first and last four characters of a long key ID.
pub fn mask(key_id: &str) -> String {
    let characters: Vec<char> = key_id.chars().collect();
    if characters.len() <= 10 {
        return key_id.to_owned();
    }
    let head: String = characters[..4].iter().collect();
    let tail: String = characters[characters.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

fn modes_text(capabilities: Option<&Capabilities>) -> String {
    let Some(capabilities) = capabilities else {
        return "Checking…".to_owned();
    };
    let titles: Vec<&str> = Mode::ALL
        .iter()
        .filter(|mode| capabilities.modes.contains(mode))
        .map(|mode| mode_title(*mode))
        .collect();
    if titles.is_empty() {
        "None".to_owned()
    } else {
        titles.join(" · ")
    }
}

/// What the unlimited allowance covers, for the Settings switch: "Genjutsu Motion transfer · Seedance 2.5"; `None`
/// when it covers nothing, and the switch isn't offered.
pub fn unlimited_text(capabilities: &Capabilities) -> Option<String> {
    let names: Vec<String> = VideoModel::all()
        .into_iter()
        .flat_map(|model| {
            let modes: Vec<Mode> = Mode::ALL
                .iter()
                .copied()
                .filter(|mode| {
                    capabilities
                        .unlimited
                        .get(&model)
                        .is_some_and(|covered| covered.contains(mode))
                })
                .collect();
            if modes.is_empty() {
                return Vec::new();
            }
            // Genjutsu and Genjutsu Lite name the modes: each has more than one, and the allowance may cover one.
            if model.descriptor().shape == RequestShape::Genjutsu {
                modes
                    .into_iter()
                    .map(|mode| format!("{} {}", model.title(), mode_title(mode)))
                    .collect()
            } else {
                vec![model.title()]
            }
        })
        .collect();
    (!names.is_empty()).then(|| names.join(" · "))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use jutsu_core::{CostUnit, Resolution};

    use super::*;
    use crate::connection::AccountInfo;

    fn some_modes() -> Capabilities {
        Capabilities::new(
            BTreeSet::from([Mode::ObjectSwap, Mode::MotionTransfer]),
            BTreeSet::from([Resolution::P480]),
            CostUnit::Credits,
            None,
        )
    }

    fn labels(rows: &[ConnectionRow]) -> Vec<&str> {
        rows.iter().map(|row| row.label.as_str()).collect()
    }

    #[test]
    fn api_key_is_masked_and_kept_in_the_credential_store() {
        let rows = ConnectionState::ApiKey {
            key_id: "hf_live_1234567890abcd".into(),
        }
        .card_rows(Some(&some_modes()));
        assert_eq!(labels(&rows), ["Key ID", "Secret", "Stored in"]);
        assert_eq!(rows[0].value, "hf_l…abcd");
        assert!(rows[0].is_monospaced);
        assert!(!rows[1].value.contains("1234"));
        assert_eq!(rows[2].value, credential_store_title());
        // A short key ID is shown as is.
        assert_eq!(mask("kid-1"), "kid-1");
    }

    #[test]
    fn account_shows_what_is_known() {
        let info = AccountInfo {
            email: Some("me@example.com".into()),
            plan: Some("Creator".into()),
            credits: Some(1_200),
            ..AccountInfo::default()
        };
        let rows = ConnectionState::Account(info).card_rows(Some(&some_modes()));
        assert_eq!(labels(&rows), ["Account", "Plan", "Credits", "Genjutsu"]);
        let values: Vec<&str> = rows.iter().map(|row| row.value.as_str()).collect();
        assert_eq!(
            values,
            [
                "me@example.com",
                "Creator",
                "1,200",
                "Motion transfer · Object swap"
            ]
        );
    }

    #[test]
    fn account_before_the_first_provider_call() {
        let state = ConnectionState::Account(AccountInfo::default());
        let rows = state.card_rows(None);
        assert_eq!(labels(&rows), ["Account", "Genjutsu"]);
        assert_eq!(rows[0].value, "Signed in");
        assert_eq!(rows[1].value, "Checking…");
        let none = Capabilities::new(BTreeSet::new(), BTreeSet::new(), CostUnit::Credits, None);
        assert_eq!(state.card_rows(Some(&none)).last().unwrap().value, "None");
    }

    #[test]
    fn the_setting_is_offered_only_for_what_the_allowance_covers() {
        let mut capabilities = some_modes();
        capabilities.unlimited =
            BTreeMap::from([(VideoModel::GENJUTSU, BTreeSet::from([Mode::MotionTransfer]))]);
        assert_eq!(
            unlimited_text(&capabilities).as_deref(),
            Some("Genjutsu Motion transfer")
        );
        capabilities.unlimited = BTreeMap::from([
            (
                VideoModel::unchecked("seedance25"),
                BTreeSet::from([Mode::MotionTransfer]),
            ),
            (
                VideoModel::GENJUTSU,
                BTreeSet::from([Mode::ObjectSwap, Mode::MotionTransfer]),
            ),
        ]);
        assert_eq!(
            unlimited_text(&capabilities).as_deref(),
            Some("Genjutsu Motion transfer · Genjutsu Object swap · Seedance 2.5")
        );
        capabilities.unlimited = BTreeMap::new();
        assert_eq!(unlimited_text(&capabilities), None);
        capabilities.unlimited = BTreeMap::from([(
            VideoModel::unchecked("genjutsuLite"),
            BTreeSet::from([Mode::MotionTransfer]),
        )]);
        assert_eq!(
            unlimited_text(&capabilities).as_deref(),
            Some("Genjutsu Lite Motion transfer")
        );
    }
}
