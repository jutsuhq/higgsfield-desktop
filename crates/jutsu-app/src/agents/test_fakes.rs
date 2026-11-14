//! Fakes for agents' tests: an image provider with a scripted quote and submit, and the agent spend ledger in
//! memory with the budget check `AgentSpendFile` makes.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use jutsu_core::{
    AgentMoneyUnit, AgentSpend, AgentSpendLedger, AgentSpendTotal, Cost, ImageJobStatus, ImageModel,
    ImageProvider, ImageRequest, PortError, PreparedFile, ProviderError, ProviderKind, RemoteAsset,
    RemoteJob,
};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

pub struct ImageScript {
    pub estimate: Result<Cost, ProviderError>,
    pub submit_error: Option<ProviderError>,
    /// How each job of a request ends, one job per entry; empty makes one job that completes.
    pub outcomes: Vec<ImageJobStatus>,
    /// Refuses to cancel, as for a job that already started.
    pub refuses_cancel: bool,
}

/// An image provider for agents' requests: a scripted quote and submit, every job done at the first poll.
pub struct ImageFake {
    pub kind: ProviderKind,
    pub owner: Option<String>,
    pub script: Mutex<ImageScript>,
    pub submitted: Mutex<Vec<ImageRequest>>,
}

pub fn image_url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

impl ImageFake {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            kind: ProviderKind::ApiKey,
            owner: Some("key-1".to_owned()),
            script: Mutex::new(ImageScript {
                estimate: Ok(Cost::usd("0.08".parse().unwrap(), false)),
                submit_error: None,
                outcomes: Vec::new(),
                refuses_cancel: false,
            }),
            submitted: Mutex::new(Vec::new()),
        })
    }

    pub fn script(&self, change: impl FnOnce(&mut ImageScript)) {
        change(&mut self.script.lock().unwrap());
    }

    pub fn submitted(&self) -> Vec<ImageRequest> {
        self.submitted.lock().unwrap().clone()
    }
}

#[async_trait]
impl ImageProvider for ImageFake {
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    fn owner(&self) -> Option<String> {
        self.owner.clone()
    }
    async fn image_models(&self) -> Result<Vec<ImageModel>, PortError> {
        Ok(Vec::new())
    }
    async fn image_estimate(&self, _request: &ImageRequest) -> Result<Cost, PortError> {
        self.script
            .lock()
            .unwrap()
            .estimate
            .clone()
            .map_err(PortError::from)
    }
    async fn upload(&self, _file: PreparedFile) -> Result<RemoteAsset, PortError> {
        let mut asset = RemoteAsset::new(image_url("https://cdn.example.com/in.png"));
        asset.id = Some("media-1".to_owned());
        Ok(asset)
    }
    async fn submit_images(&self, request: &ImageRequest, _key: Uuid) -> Result<Vec<RemoteJob>, PortError> {
        self.submitted.lock().unwrap().push(request.clone());
        let script = self.script.lock().unwrap();
        if let Some(error) = script.submit_error.clone() {
            return Err(error.into());
        }
        if script.outcomes.is_empty() {
            return Ok(vec![RemoteJob::new("req-1")]);
        }
        Ok((0..script.outcomes.len())
            .map(|index| RemoteJob::new(format!("req-{index}")))
            .collect())
    }
    async fn image_status(&self, job: &RemoteJob) -> Result<ImageJobStatus, PortError> {
        let script = self.script.lock().unwrap();
        let index = job
            .request_id
            .strip_prefix("req-")
            .and_then(|index| index.parse::<usize>().ok());
        Ok(match index.and_then(|index| script.outcomes.get(index)) {
            Some(status) => status.clone(),
            None => ImageJobStatus::Completed(vec![image_url("https://cdn.example.com/1.png")]),
        })
    }
    async fn cancel_images(&self, _job: &RemoteJob) -> Result<(), PortError> {
        if self.script.lock().unwrap().refuses_cancel {
            return Err(ProviderError::Validation {
                field: None,
                message: "already running".to_owned(),
            }
            .into());
        }
        Ok(())
    }
}

/// The agent spend ledger in memory, with the budget check `AgentSpendFile` makes.
#[derive(Default)]
pub struct MemoryLedger {
    pub spends: Mutex<Vec<AgentSpend>>,
    pub is_broken: Mutex<bool>,
}

impl MemoryLedger {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn break_it(&self) {
        *self.is_broken.lock().unwrap() = true;
    }

    pub fn spends(&self) -> Vec<AgentSpend> {
        self.spends.lock().unwrap().clone()
    }

    /// What settled spends count.
    pub fn spent(&self) -> i64 {
        self.spends()
            .iter()
            .filter(|spend| spend.is_settled)
            .map(AgentSpend::counted)
            .sum()
    }

    fn check(&self) -> Result<(), PortError> {
        if *self.is_broken.lock().unwrap() {
            return Err(PortError::Other("broken".to_owned()));
        }
        Ok(())
    }
}

#[async_trait]
impl AgentSpendLedger for MemoryLedger {
    async fn reserve(&self, spend: AgentSpend, daily_limit: Option<i64>) -> Result<bool, PortError> {
        self.check()?;
        let mut spends = self.spends.lock().unwrap();
        let taken: i64 = spends.iter().map(|spend| spend.money.amount).sum();
        if let Some(limit) = daily_limit
            && taken + spend.money.amount > limit
        {
            return Ok(false);
        }
        spends.push(spend);
        Ok(true)
    }
    async fn release(&self, id: Uuid) -> Result<(), PortError> {
        self.check()?;
        self.spends.lock().unwrap().retain(|spend| spend.id != id);
        Ok(())
    }
    async fn settle(&self, id: Uuid, amount: Option<i64>) -> Result<(), PortError> {
        if let Some(spend) = self
            .spends
            .lock()
            .unwrap()
            .iter_mut()
            .find(|spend| spend.id == id)
        {
            spend.is_settled = true;
            if let Some(amount) = amount {
                spend.money.amount = amount;
            }
        }
        Ok(())
    }
    async fn unbill(&self, id: Uuid, part: String, parts: i64) -> Result<(), PortError> {
        let mut spends = self.spends.lock().unwrap();
        let Some(spend) = spends.iter_mut().find(|spend| spend.id == id) else {
            return Ok(());
        };
        if spend
            .unbilled
            .as_ref()
            .is_some_and(|unbilled| unbilled.contains(&part))
        {
            return Ok(());
        }
        spend.parts = Some(parts);
        spend.unbilled.get_or_insert_with(Vec::new).push(part);
        Ok(())
    }
    async fn total(&self, _day: OffsetDateTime, _unit: AgentMoneyUnit) -> Result<AgentSpendTotal, PortError> {
        let spends = self.spends();
        Ok(AgentSpendTotal {
            spent: spends
                .iter()
                .filter(|spend| spend.is_settled)
                .map(AgentSpend::counted)
                .sum(),
            reserved: spends
                .iter()
                .filter(|spend| !spend.is_settled)
                .map(AgentSpend::counted)
                .sum(),
            unlimited_charged: false,
        })
    }
    async fn unsettled(&self) -> Result<Vec<AgentSpend>, PortError> {
        Ok(self
            .spends()
            .into_iter()
            .filter(|spend| !spend.is_settled)
            .collect())
    }
}
