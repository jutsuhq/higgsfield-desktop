//! What the active connection can run.

use std::collections::{BTreeMap, BTreeSet};

use crate::models::{JobSpec, Mode, ModelParameter, Resolution, VideoModel};
use crate::support::string_enum;

string_enum! {
    /// What a connection pays in.
    pub enum CostUnit {
        /// Dollars.
        Usd = "usd",
        /// Credits.
        Credits = "credits",
    }
}

/// What the active connection can run; drives which modes and options the UI enables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// The modes.
    pub modes: BTreeSet<Mode>,
    /// Genjutsu always; the others where the connection offers them.
    pub models: BTreeSet<VideoModel>,
    /// The resolutions.
    pub resolutions: BTreeSet<Resolution>,
    /// What it pays in.
    pub cost_unit: CostUnit,
    /// `None` when unknown until the API reports it.
    pub max_concurrency: Option<i64>,
    /// The model parameters the connection can send; Create disables the control of any other one and says why.
    pub parameters: BTreeSet<ModelParameter>,
    /// The modes of each model the account can pay from its plan's unlimited allowance right now, instead of
    /// credits; empty when it has no allowance or the connection can't tell.
    pub unlimited: BTreeMap<VideoModel, BTreeSet<Mode>>,
    /// The most variants one submit of a generation makes (`JobSpec::variants`); 1 where the connection has none.
    /// A Genjutsu Style and an operation on a result always make one.
    pub max_variants: i64,
    /// Modes of an offered model that this connection can't run though the model serves them: Genjutsu Lite's
    /// Object Swap with an API key, which has no REST endpoint. Empty where every offered model runs all its modes.
    pub unserved_modes: BTreeMap<VideoModel, BTreeSet<Mode>>,
}

impl Capabilities {
    /// A connection of `modes` and `resolutions` that offers Genjutsu, sends every parameter, has no allowance and
    /// makes one variant per submit.
    pub fn new(
        modes: BTreeSet<Mode>,
        resolutions: BTreeSet<Resolution>,
        cost_unit: CostUnit,
        max_concurrency: Option<i64>,
    ) -> Self {
        Self {
            modes,
            models: BTreeSet::from([VideoModel::GENJUTSU]),
            resolutions,
            cost_unit,
            max_concurrency,
            parameters: ModelParameter::ALL.iter().copied().collect(),
            unlimited: BTreeMap::new(),
            max_variants: 1,
            unserved_modes: BTreeMap::new(),
        }
    }

    /// Whether a job on `model` in `mode` runs on this connection: the model is offered, serves the mode, and the
    /// connection runs it in that mode.
    pub fn runs(&self, model: &VideoModel, mode: Mode) -> bool {
        self.models.contains(model)
            && model.serves(mode)
            && !self
                .unserved_modes
                .get(model)
                .is_some_and(|modes| modes.contains(&mode))
    }

    /// Whether `spec` is paid from the unlimited allowance: only when the user opted in and the account has it for
    /// the spec's model and mode.
    pub fn pays_unlimited(&self, spec: &JobSpec, opted_in: bool) -> bool {
        opted_in
            && self
                .unlimited
                .get(&spec.model)
                .is_some_and(|modes| modes.contains(&spec.mode))
    }
}
