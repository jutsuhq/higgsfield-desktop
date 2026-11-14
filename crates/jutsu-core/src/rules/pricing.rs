//! List prices per second, from `pricing.json`.

use std::collections::{HashMap, HashSet};

use rust_decimal::Decimal;
use serde_json::Value;

use crate::models::{Cost, JobSpec, Mode, Resolution, VideoModel, decimal_shortest_of};
use crate::provider::ProviderKind;
use crate::resources::PRICING_JSON;
use crate::support::Seconds;

/// Why prices couldn't be loaded or a rate found. The texts are the case names, as Swift has no wording for them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum PricingError {
    /// `pricing.json` is missing.
    #[error("missingFile")]
    MissingFile,
    /// The tariff ID (`tariff_id`) and the resolution without a rate.
    #[error("missingRate")]
    MissingRate(String, Resolution),
    /// The tariff ID and the resolution `pricing.json` lists as unpriced (`null`): no public page states it.
    #[error("unpricedRate")]
    UnpricedRate(String, Resolution),
    /// `pricing.json` isn't in its format (Swift's decoding error).
    #[error("{0}")]
    Malformed(String),
}

/// One tariff at one resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TariffKey {
    /// The tariff ID.
    pub tariff: String,
    /// The resolution.
    pub resolution: Resolution,
}

impl TariffKey {
    /// `tariff` at `resolution`.
    pub fn new(tariff: impl Into<String>, resolution: Resolution) -> Self {
        Self {
            tariff: tariff.into(),
            resolution,
        }
    }
}

/// List prices per second, from `pricing.json`, by tariff: the REST endpoint a job runs on with an API key
/// (`VideoModel::endpoint_id`), and for a model that bills it (Seedance) whether the source video goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pricing {
    /// USD per billed second, by tariff ID.
    pub rates: HashMap<String, HashMap<Resolution, Decimal>>,
    /// The tariffs `pricing.json` lists without a rate (`null`), because no public page states it: a job on one
    /// costs `Cost::UNKNOWN_USD` until the server quotes it.
    pub unpriced: HashSet<TariffKey>,
    /// The tariffs whose rate is the top of a published range rather than their own price (`{"upTo": rate}` in
    /// `pricing.json`): a job on one is shown as costing "up to" its price, never less than it can cost, until the
    /// server quotes it.
    pub upper_bounds: HashSet<TariffKey>,
}

impl Pricing {
    const WITH_VIDEO_REFERENCE: &'static str = " with video reference";
    /// Far below one frame, far above the error of converting media timestamps to seconds.
    pub const DURATION_TOLERANCE: Seconds = 1e-6;

    /// Fails unless every tariff a job can have is priced, or listed as unpriced, at every resolution it can run
    /// at, so a gap in `pricing.json` stops the app at load.
    pub fn new(
        rates: HashMap<String, HashMap<Resolution, Decimal>>,
        unpriced: HashSet<TariffKey>,
        upper_bounds: HashSet<TariffKey>,
    ) -> Result<Self, PricingError> {
        for (tariff, resolution) in Self::tariffs() {
            let priced = rates
                .get(&tariff)
                .is_some_and(|rates| rates.contains_key(&resolution));
            if !priced && !unpriced.contains(&TariffKey::new(tariff.clone(), resolution)) {
                return Err(PricingError::MissingRate(tariff, resolution));
            }
        }
        Ok(Self {
            rates,
            unpriced,
            upper_bounds,
        })
    }

    /// Decodes the `pricing.json` format: a rate, `null` for an unpriced one, or `{"upTo": rate}` for the top of a
    /// published range.
    pub fn from_json(text: &str) -> Result<Self, PricingError> {
        let malformed = |message: &str| PricingError::Malformed(message.to_owned());
        let file: Value =
            serde_json::from_str(text).map_err(|error| PricingError::Malformed(error.to_string()))?;
        let models = file
            .get("models")
            .and_then(Value::as_object)
            .ok_or_else(|| malformed("models"))?;
        let mut rates: HashMap<String, HashMap<Resolution, Decimal>> = HashMap::new();
        let mut unpriced = HashSet::new();
        let mut upper_bounds = HashSet::new();
        for (tariff, prices) in models {
            let prices = prices.as_object().ok_or_else(|| malformed(tariff))?;
            for (key, value) in prices {
                let rate = match value {
                    Value::Null => None,
                    Value::Number(number) => Some((number.as_f64().ok_or_else(|| malformed(key))?, false)),
                    other => {
                        let top = other
                            .get("upTo")
                            .and_then(Value::as_f64)
                            .ok_or_else(|| malformed(key))?;
                        Some((top, true))
                    }
                };
                let Some(resolution) = Resolution::from_raw(key) else {
                    continue;
                };
                match rate {
                    None => {
                        unpriced.insert(TariffKey::new(tariff.clone(), resolution));
                    }
                    Some((rate, is_upper_bound)) => {
                        let rate = decimal_shortest_of(rate).ok_or_else(|| malformed(key))?;
                        rates.entry(tariff.clone()).or_default().insert(resolution, rate);
                        if is_upper_bound {
                            upper_bounds.insert(TariffKey::new(tariff.clone(), resolution));
                        }
                    }
                }
            }
        }
        Self::new(rates, unpriced, upper_bounds)
    }

    /// The prices bundled with the crate.
    pub fn bundled() -> Result<Self, PricingError> {
        Self::from_json(PRICING_JSON)
    }

    /// The key of a job's rates in `pricing.json`: its endpoint, with " with video reference" appended for a source
    /// on a model that bills the source video (`VideoModelDescriptor::bills_source_video`, Seedance). Seedance bills
    /// the input video's seconds too, at 0.6 of the rate for input and output alike, so a source clip costs 1.2 times
    /// the rate per second of it (open.higgsfield.ai lists this "with video reference" price separately). Edit and
    /// Extend have their own endpoints and rates (`billed_seconds_for`).
    pub fn tariff_id(
        model: &VideoModel,
        mode: Mode,
        resolution: Resolution,
        has_source: bool,
        has_references: bool,
        uses_frames: bool,
    ) -> String {
        // Frames only go without a source (`JobSpec::uses_frames`).
        let endpoint = model.endpoint_id(
            mode,
            resolution,
            has_source || has_references,
            uses_frames && !has_source,
        );
        let adds_video_reference =
            model.descriptor().bills_source_video && has_source && !mode.is_video_operation();
        if adds_video_reference {
            endpoint + Self::WITH_VIDEO_REFERENCE
        } else {
            endpoint
        }
    }

    /// Every tariff a job can have with an API key, with each resolution its model renders at in that mode
    /// (`VideoModel::resolutions_for`). A model or mode the account alone runs (`VideoModel::runs_with_api_key_in`)
    /// has none.
    pub fn tariffs() -> Vec<(String, Resolution)> {
        let mut tariffs = Vec::new();
        for model in VideoModel::all().iter().filter(|model| model.runs_with_api_key()) {
            for mode in Mode::ALL
                .iter()
                .copied()
                .filter(|mode| model.runs_with_api_key_in(*mode))
            {
                for resolution in model.resolutions_for(mode) {
                    for has_source in [true, false] {
                        for has_references in [true, false] {
                            for uses_frames in [true, false] {
                                let tariff = Self::tariff_id(
                                    model,
                                    mode,
                                    resolution,
                                    has_source,
                                    has_references,
                                    uses_frames,
                                );
                                tariffs.push((tariff, resolution));
                            }
                        }
                    }
                }
            }
        }
        tariffs
    }

    /// USD per billed second. Fails with `MissingRate`, or `UnpricedRate` for one no page states, rather than
    /// pricing a tariff at $0.
    pub fn rate(
        &self,
        model: &VideoModel,
        mode: Mode,
        resolution: Resolution,
        has_source: bool,
        has_references: bool,
        uses_frames: bool,
    ) -> Result<Decimal, PricingError> {
        let tariff = Self::tariff_id(model, mode, resolution, has_source, has_references, uses_frames);
        if let Some(rate) = self.rates.get(&tariff).and_then(|rates| rates.get(&resolution)) {
            return Ok(*rate);
        }
        if self
            .unpriced
            .contains(&TariffKey::new(tariff.clone(), resolution))
        {
            return Err(PricingError::UnpricedRate(tariff, resolution));
        }
        Err(PricingError::MissingRate(tariff, resolution))
    }

    /// Whether the tariff's rate is only the top of a published range (`upper_bounds`).
    pub fn is_upper_bound(
        &self,
        model: &VideoModel,
        mode: Mode,
        resolution: Resolution,
        has_source: bool,
        has_references: bool,
        uses_frames: bool,
    ) -> bool {
        let tariff = Self::tariff_id(model, mode, resolution, has_source, has_references, uses_frames);
        self.upper_bounds.contains(&TariffKey::new(tariff, resolution))
    }

    /// `is_upper_bound` for `spec`.
    pub fn is_upper_bound_for(&self, spec: &JobSpec) -> bool {
        self.is_upper_bound(
            &spec.model,
            spec.mode,
            spec.resolution,
            spec.source.is_some(),
            !spec.references.is_empty(),
            spec.uses_frames(),
        )
    }

    /// `rate` for `spec`.
    pub fn rate_for(&self, spec: &JobSpec) -> Result<Decimal, PricingError> {
        self.rate(
            &spec.model,
            spec.mode,
            spec.resolution,
            spec.source.is_some(),
            !spec.references.is_empty(),
            spec.uses_frames(),
        )
    }

    /// Exact where the catalog says so (`list_price_is_exact`): Genjutsu bills whole seconds of source. Seedance bills
    /// tokens of the output frame, which the rate fits for 16:9, and Kling's rounding isn't published, so theirs is
    /// an estimate, as is the top of a range (`is_upper_bound`). `Cost::UNKNOWN_USD` for an unpriced tariff, which the
    /// server's quote replaces after upload.
    pub fn cost(&self, spec: &JobSpec) -> Result<Cost, PricingError> {
        let seconds = Decimal::from(Self::billed_seconds_for(spec));
        match self.rate_for(spec) {
            Ok(rate) => {
                let is_estimate =
                    !spec.model.descriptor().list_price_is_exact || self.is_upper_bound_for(spec);
                Ok(Cost::usd(seconds.saturating_mul(rate), is_estimate))
            }
            Err(PricingError::UnpricedRate(..)) => Ok(Cost::UNKNOWN_USD),
            Err(error) => Err(error),
        }
    }

    /// Whole seconds a job's rate applies to. Seedance bills the source video's seconds as well as the output's,
    /// both at 0.6 of its rate: Edit's output is as long as its source, so its rate (1.2 times) is per second of
    /// source, while Extend's (0.6 times) applies to the source and the added part together.
    pub fn billed_seconds_for(spec: &JobSpec) -> i64 {
        if spec.mode != Mode::Extend {
            return Self::billed_seconds(spec.duration());
        }
        Self::billed_seconds(
            spec.source
                .as_ref()
                .map_or(0.0, crate::models::SourceClip::selected_duration)
                + spec.duration(),
        )
    }

    /// Whole seconds a job is billed for on `connection`. The account bills Extend's added part alone: a 4 s
    /// extension of a 4 s clip was billed 12 credits, 4 s at 3 a second, in a live run on 2026-10-15. REST bills the
    /// source selection too (`billed_seconds_for`), as does any connection not known yet.
    pub fn billed_seconds_on(spec: &JobSpec, connection: Option<ProviderKind>) -> i64 {
        if connection == Some(ProviderKind::Account) {
            Self::billed_seconds(spec.duration())
        } else {
            Self::billed_seconds_for(spec)
        }
    }

    /// Whole seconds the API bills: the duration rounded up, ignoring float noise (12.0000001 s → 12 s).
    /// NaN and out-of-range input, which `Validation::check` rejects, clamp instead of trapping: display paths
    /// call this before validation.
    pub fn billed_seconds(duration: Seconds) -> i64 {
        let seconds = (duration - Self::DURATION_TOLERANCE).ceil();
        if seconds.is_nan() || seconds <= 0.0 {
            return 0;
        }
        if seconds < i64::MAX as f64 {
            seconds as i64
        } else {
            i64::MAX
        }
    }
}
