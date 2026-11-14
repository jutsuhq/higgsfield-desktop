//! A tool's arguments, read strictly: a key the tool doesn't take or a value of the wrong type is an error that names
//! it, never a crash or a silent default.

use jutsu_agent::{
    AgentAmount, AgentBudgetUnit, AgentError, AgentSource, AgentVideoRequest, Timestamp, Uuid, names, video,
};
use serde_json::{Map, Value};

pub(crate) struct Arguments<'a> {
    pub(crate) tool: String,
    pub(crate) values: &'a Map<String, Value>,
}

/// A tool's refusal: `Malformed request: {reason}.`, as the app words its own.
pub(crate) fn refuse(reason: impl std::fmt::Display) -> AgentError {
    AgentError::bad_request(reason)
}

impl<'a> Arguments<'a> {
    pub(crate) fn new(tool: &str, values: &'a Map<String, Value>) -> Self {
        Self {
            tool: tool.to_owned(),
            values,
        }
    }

    /// The value at `key`, `None` when left out or null.
    pub(crate) fn given(&self, key: &str) -> Option<&'a Value> {
        self.values.get(key).filter(|value| !value.is_null())
    }

    pub(crate) fn allow(&self, keys: &[&str]) -> Result<(), AgentError> {
        let mut given: Vec<&String> = self.values.keys().collect();
        given.sort();
        if let Some(unknown) = given.into_iter().find(|key| !keys.contains(&key.as_str())) {
            let mut known = keys.to_vec();
            known.sort_unstable();
            known.dedup();
            let known = if known.is_empty() {
                "no arguments".to_owned()
            } else {
                known.join(", ")
            };
            return Err(refuse(format!(
                "{} takes {known}, not '{}'",
                self.tool,
                names::prefix(unknown, 64)
            )));
        }
        Ok(())
    }

    /// `None` when left out or null.
    pub(crate) fn string(&self, key: &str) -> Result<Option<String>, AgentError> {
        match self.given(key) {
            None => Ok(None),
            Some(Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => Err(refuse(format!("{}: {key} is a string", self.tool))),
        }
    }

    /// `false` when left out or null.
    pub(crate) fn bool(&self, key: &str) -> Result<bool, AgentError> {
        Ok(self.optional_bool(key)?.unwrap_or(false))
    }

    /// `None` when left out or null, unlike `bool`.
    pub(crate) fn optional_bool(&self, key: &str) -> Result<Option<bool>, AgentError> {
        match self.given(key) {
            None => Ok(None),
            Some(Value::Bool(value)) => Ok(Some(*value)),
            Some(_) => Err(refuse(format!("{}: {key} is true or false", self.tool))),
        }
    }

    /// A finite number; `None` when left out or null.
    pub(crate) fn number(&self, key: &str) -> Result<Option<f64>, AgentError> {
        let Some(value) = self.given(key) else {
            return Ok(None);
        };
        match value.as_f64() {
            Some(number) if number.is_finite() => Ok(Some(number)),
            _ => Err(refuse(format!("{}: {key} is a number", self.tool))),
        }
    }

    /// A whole number that fits; `None` when left out or null.
    pub(crate) fn int(&self, key: &str) -> Result<Option<i64>, AgentError> {
        let Some(number) = self.number(key)? else {
            return Ok(None);
        };
        #[allow(clippy::float_cmp, reason = "a whole number rounds to itself exactly")]
        let whole = number.round() == number;
        if !whole || number.abs() >= 1e15 {
            return Err(refuse(format!("{}: {key} is a whole number", self.tool)));
        }
        #[allow(clippy::cast_possible_truncation, reason = "a whole number under 1e15 fits")]
        Ok(Some(number as i64))
    }

    /// An ISO 8601 date and time, such as 2026-10-21T09:00:00Z; `None` when left out or null.
    pub(crate) fn date(&self, key: &str) -> Result<Option<Timestamp>, AgentError> {
        let Some(text) = self.string(key)? else {
            return Ok(None);
        };
        Timestamp::parse_argument(&text).map(Some).ok_or_else(|| {
            refuse(format!(
                "{}: {key} is an ISO 8601 date and time, such as 2026-10-21T09:00:00Z",
                self.tool
            ))
        })
    }

    /// Library item IDs; empty when left out or null.
    pub(crate) fn ids(&self, key: &str) -> Result<Vec<Uuid>, AgentError> {
        self.strings(key)?
            .iter()
            .map(|text| {
                Uuid::parse(text).ok_or_else(|| refuse(format!("{}: {key} are Library item IDs", self.tool)))
            })
            .collect()
    }

    /// A UUID, refused with `message` when it isn't one; `None` when left out or null.
    pub(crate) fn uuid(&self, key: &str, message: &str) -> Result<Option<Uuid>, AgentError> {
        self.string(key)?
            .map(|text| Uuid::parse(&text).ok_or_else(|| refuse(message)))
            .transpose()
    }

    /// `request_id`, which every paid tool needs.
    pub(crate) fn request_id(&self) -> Result<String, AgentError> {
        let Some(request_id) = self.string("request_id")? else {
            return Err(refuse(format!(
                "{} needs request_id, a new ID of your own for each new request",
                self.tool
            )));
        };
        if !names::is_valid_request_id(&request_id) {
            return Err(refuse(names::REQUEST_ID_RULE));
        }
        Ok(request_id)
    }

    /// Empty when left out or null.
    pub(crate) fn strings(&self, key: &str) -> Result<Vec<String>, AgentError> {
        let list = || refuse(format!("{}: {key} is a list of strings", self.tool));
        match self.given(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_owned).ok_or_else(list))
                .collect(),
            Some(_) => Err(list()),
        }
    }

    /// `None` when left out or null.
    pub(crate) fn object(&self, key: &str) -> Result<Option<Arguments<'a>>, AgentError> {
        match self.given(key) {
            None => Ok(None),
            Some(Value::Object(fields)) => Ok(Some(Arguments {
                tool: format!("{}: {key}", self.tool),
                values: fields,
            })),
            Some(_) => Err(refuse(format!("{}: {key} is an object", self.tool))),
        }
    }

    /// The video fields `quote` and `create_video` share.
    pub(crate) fn video(&self) -> Result<AgentVideoRequest, AgentError> {
        let Some(mode) = self.string("mode")? else {
            return Err(refuse(format!(
                "{} needs mode, one of {}",
                self.tool,
                video::MODES.join(", ")
            )));
        };
        let mut source = None;
        if let Some(fields) = self.object("source")? {
            fields.allow(&["path", "start", "end"])?;
            let start = fields.number("start")?;
            let end = fields.number("end")?;
            if let Some(problem) = AgentSource::problem(start, end) {
                return Err(refuse(problem));
            }
            source = Some(AgentSource {
                path: fields.string("path")?,
                start,
                end,
            });
        }
        let style = self.uuid("style", &format!("{}: style is a style ID (a UUID)", self.tool))?;
        Ok(AgentVideoRequest {
            mode,
            model: self.string("model")?,
            prompt: self.string("prompt")?,
            source,
            images: self.strings("images")?,
            image_count: None,
            style,
            resolution: self.string("resolution")?,
            draft: self.bool("draft")?,
            length: self.number("length")?,
            aspect_ratio: self.string("aspect_ratio")?,
            audio: self.string("audio")?,
            variants: self.int("variants")?,
            elements: self.strings("elements")?,
            title: self.string("title")?,
            speech: self.string("speech")?,
            speech_length: None,
        })
    }

    /// `max_cost` as whole units: dollars become cents, rounded down so the job never costs more than was said.
    pub(crate) fn max_cost(&self) -> Result<AgentAmount, AgentError> {
        let Some(fields) = self.object("max_cost")? else {
            return Err(refuse(format!(
                "{} needs max_cost, {{amount, unit}}: the most you agree to pay, in usd or credits as capabilities says",
                self.tool
            )));
        };
        fields.allow(&["amount", "unit"])?;
        let amount = match fields.number("amount")? {
            Some(amount) if amount >= 0.0 => amount,
            _ => {
                return Err(refuse(format!(
                    "{}: max_cost.amount is a number, 0 or more",
                    self.tool
                )));
            }
        };
        // Both bounds are whole and far under 2^53, so the casts below are exact.
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            reason = "bounded above"
        )]
        match fields.string("unit")?.as_deref() {
            Some("usd") => {
                let most = AgentAmount::maximum(AgentBudgetUnit::UsdCents);
                let cents = (amount * 100.0).floor();
                if cents > most as f64 {
                    return Err(refuse(format!(
                        "{}: max_cost is at most ${}",
                        self.tool,
                        most / 100
                    )));
                }
                Ok(AgentAmount::new(cents as i64, AgentBudgetUnit::UsdCents))
            }
            Some("credits") => {
                let most = AgentAmount::maximum(AgentBudgetUnit::Credits);
                if amount > most as f64 {
                    return Err(refuse(format!(
                        "{}: max_cost is at most {most} credits",
                        self.tool
                    )));
                }
                Ok(AgentAmount::new(amount.floor() as i64, AgentBudgetUnit::Credits))
            }
            _ => Err(refuse(format!("{}: max_cost.unit is usd or credits", self.tool))),
        }
    }
}
