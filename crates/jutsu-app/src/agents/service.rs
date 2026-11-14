//! The app's side of agents (`AgentService.swift` and its extensions): the commands the `jutsu` helper runs, each on
//! the app's own services. Settings › Agents › Allow agents is read for every command, so turning it off refuses the
//! next one at once and stops nothing an agent already started. Nothing here decides what may be spent: every paid
//! submit passes the engine's spending guard (`AgentSpendPolicy`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use jutsu_agent::{
    AgentActed, AgentCapabilities, AgentCommand, AgentCreated, AgentEndpoint, AgentError, AgentHandler,
    AgentInbox, AgentJob, AgentJobAction, AgentJobs, AgentPage, AgentQuote, AgentRequest, AgentResponse,
    AgentSheet, AgentShown, AgentSubtitlesAction, AgentVideoRequest, CapabilitiesCommand, Connection,
    ConnectionKind, CreateVideoCommand, ErrorCode, JobActionCommand, JobsCommand, ModeDetails, Model,
    ModelDetails, QuoteCommand, ShowCommand, UploadCommand, Window,
};
use jutsu_core::{
    AgentMoney, AgentOrigin, AgentSpendDecision, AgentSpendLedger, AgentSpendTotal, Cost, Job, JobSpec,
    JobState, LocalImage, ProviderKind, VideoControl, VideoModel,
};
use jutsu_engine::{EngineError, EnqueueOptions, FreeSpace, VolumeFreeSpace};
use time::OffsetDateTime;
use url::Url;
use uuid::Uuid;

use super::activity::{AgentActivity, Ask};
use super::budget::{self, amount, cap_money, money, over_cap_message, unit_for};
use super::job::{AgentJobRules, agent_job, agent_wait, is_priced, is_settled, mode_name, page};
use super::preferences::AgentPreferences;
use super::raising::{self, AgentRaisedJob};
use super::request_gate::{AgentRaiseOutcome, AgentRequestGate, AgentRequestState, Refusals};
use super::video_order::{AgentVideoOrder, OrderSource, variants};
use super::{app_id, wire_id};
use crate::AppEnvironment;

/// What the window does for agents; the shell implements it.
pub trait AgentWindow: Send + Sync {
    /// Whether the main window is open.
    fn is_open(&self) -> bool;
    /// Whether the app keeps running in the system tray with no window.
    fn runs_in_tray(&self) -> bool;
    /// Opens the main window on `target`; `false` when it couldn't.
    fn show(&self, target: &AgentShowTarget) -> bool;
}

/// What `show` opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentShowTarget {
    Page(AgentPage),
    /// A job's row in the Queue.
    QueueJob(Uuid),
    /// A finished job's result in Library.
    LibraryJob(Uuid),
    /// An item's sheet in Library.
    Sheet(Uuid, AgentSheet),
}

/// The sheet or dialog over the main window, as the React app reports it (`mainWindowHasSheet`). `show` replaces
/// Upscale on This Computer's sheet and the Subtitles sheet (`NavigationModel.onDeviceSheet`, `.subtitlesSheet`), never
/// another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum WindowSheet {
    /// An Enhance run on this computer's sheet (`OnDeviceSheet`).
    OnDevice,
    /// The Subtitles sheet.
    Subtitles,
    /// Any other sheet, alert or confirmation.
    Other,
}

/// Where the window goes for `show`, as the React app routes it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentShowRoute {
    /// `create`, `queue`, `library` or `settings`.
    pub page: String,
    /// A job to select: its Queue row, or its result in Library.
    pub job: Option<String>,
    /// A Library item whose `sheet` opens.
    pub item: Option<String>,
    /// `upscale`, `reframe` and the other sheets `show` takes.
    pub sheet: Option<String>,
}

impl AgentShowTarget {
    pub fn route(&self) -> AgentShowRoute {
        let route = |page: AgentPage| AgentShowRoute {
            page: page.as_str().to_owned(),
            job: None,
            item: None,
            sheet: None,
        };
        match self {
            Self::Page(page) => route(*page),
            Self::QueueJob(id) => AgentShowRoute {
                job: Some(id.to_string()),
                ..route(AgentPage::Queue)
            },
            Self::LibraryJob(id) => AgentShowRoute {
                job: Some(id.to_string()),
                ..route(AgentPage::Library)
            },
            Self::Sheet(item, sheet) => AgentShowRoute {
                item: Some(item.to_string()),
                sheet: Some(sheet.as_str().to_owned()),
                ..route(AgentPage::Library)
            },
        }
    }
}

type Gate = AgentRequestGate<AgentCreated, AgentError>;

/// The handler the agent server runs commands on.
pub struct AgentService {
    me: Weak<AgentService>,
    environment: Arc<AppEnvironment>,
    preferences: AgentPreferences,
    window: Arc<dyn AgentWindow>,
    /// The paid requests, each run once for its request ID.
    requests: Arc<Gate>,
    /// Where the helper's uploads arrive; `None` without an endpoint.
    inbox: Option<AgentInbox>,
    app_version: String,
    /// What the tray panel shows of agents besides their jobs.
    activity: std::sync::Mutex<AgentActivity>,
    /// The exports agents started (`export`, `delivery_set` export), followed with `jobs`.
    operations: jutsu_agent::AgentOperations,
    /// The sheet the main window shows, which `show` doesn't put another over.
    window_sheet: std::sync::Mutex<Option<WindowSheet>>,
}

impl std::fmt::Debug for AgentService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AgentService").finish_non_exhaustive()
    }
}

/// `"Jutsu Studio isn't connected to Higgsfield."`
pub fn not_connected() -> AgentError {
    AgentError::new(
        ErrorCode::NOT_CONNECTED,
        "Jutsu Studio isn't connected to Higgsfield.",
        true,
        Some("Ask the person to connect Higgsfield in Settings, then send the request again."),
    )
}

fn requests_unreadable() -> AgentError {
    AgentError::new(
        ErrorCode::FAILED,
        "Jutsu Studio can't read its record of agent requests, so nothing is queued.",
        true,
        Some("Ask the person to check the app's data folder, then send the request again."),
    )
}

/// What the request gate answers a paid command with instead of running it.
fn refusals(command: &str, made: &str) -> Refusals<AgentError> {
    Refusals {
        conflict: AgentError::request_conflict(),
        unreadable: requests_unreadable(),
        never_queued: AgentError::new(
            ErrorCode::FAILED,
            format!("The request was recorded but {made}."),
            true,
            Some(&format!("Send the same {command} again.")),
        ),
    }
}

/// A model's `available`: connected, and among the models the connection last offered (Genjutsu alone until its
/// capabilities are read).
pub fn model_available(
    connection: Option<ProviderKind>,
    offered: &std::collections::BTreeSet<VideoModel>,
    model: &VideoModel,
) -> bool {
    connection.is_some() && offered.contains(model)
}

/// Refuses a `max_cost` in another unit than the connection's.
pub fn check_unit(max_cost: &jutsu_agent::AgentAmount, kind: ProviderKind) -> Result<(), AgentError> {
    let unit = unit_for(kind);
    if unit == max_cost.unit {
        return Ok(());
    }
    let name = if unit == jutsu_agent::AgentBudgetUnit::UsdCents {
        "usd"
    } else {
        "credits"
    };
    Err(AgentError::bad_request(format!(
        "max_cost is in {name} on this connection"
    )))
}

/// What enqueueing answers the agent with.
pub fn enqueue_error(error: &EngineError) -> AgentError {
    match error {
        EngineError::OverCap(over) => AgentError::new(
            ErrorCode::OVER_CAP,
            over_cap_message(&over.cap, over.committed_today, &over.price),
            false,
            Some("Ask the person to raise the cap in Settings or queue it themselves."),
        ),
        EngineError::NotConnected => not_connected(),
        other => AgentError::new(ErrorCode::FAILED, other.to_string(), true, None),
    }
}

impl AgentService {
    /// `endpoint` is where the server listens, whose inbox the uploads arrive in.
    pub fn new(
        environment: Arc<AppEnvironment>,
        window: Arc<dyn AgentWindow>,
        endpoint: Option<&AgentEndpoint>,
        app_version: &str,
    ) -> Arc<Self> {
        let preferences = AgentPreferences::new(environment.preferences.clone());
        let requests = Gate::open(Some(&environment.layout.root));
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            environment,
            preferences,
            window,
            requests,
            inbox: endpoint.map(|endpoint| AgentInbox::new(endpoint.inbox())),
            app_version: app_version.to_owned(),
            activity: std::sync::Mutex::default(),
            operations: jutsu_agent::AgentOperations::new(),
            window_sheet: std::sync::Mutex::default(),
        })
    }

    pub fn preferences(&self) -> &AgentPreferences {
        &self.preferences
    }

    /// What the window shows over itself now; `None` once it closed.
    pub fn set_window_sheet(&self, sheet: Option<WindowSheet>) {
        *self
            .window_sheet
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = sheet;
    }

    fn window_sheet(&self) -> Option<WindowSheet> {
        *self
            .window_sheet
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn strong(&self) -> Result<Arc<Self>, AgentError> {
        self.me.upgrade().ok_or_else(AgentError::agents_off)
    }

    fn kind(&self) -> Option<ProviderKind> {
        let state = self.environment.connection.state();
        if state.is_connected() { state.kind() } else { None }
    }

    /// The clients that sent requests lately and the open asks, as the tray panel shows them.
    pub fn activity(&self) -> AgentActivity {
        self.with_activity(|activity| activity.clone())
    }

    fn with_activity<T>(&self, change: impl FnOnce(&mut AgentActivity) -> T) -> T {
        let mut activity = self
            .activity
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        change(&mut activity)
    }

    fn note_request(&self, client: Option<&str>) {
        self.with_activity(|activity| activity.note_request(client, OffsetDateTime::now_utc()));
    }

    async fn run(&self, command: AgentCommand, client: Option<String>) -> Result<AgentResponse, AgentError> {
        let mut response = AgentResponse::default();
        match command {
            AgentCommand::Capabilities(fields) => {
                response.capabilities = Some(self.capabilities(&fields).await?);
            }
            AgentCommand::Show(fields) => response.shown = Some(self.show(&fields).await?),
            // The server takes uploads itself; one never reaches the handler.
            AgentCommand::Upload(_) => return Err(AgentError::bad_request("upload is sent with its bytes")),
            AgentCommand::Quote(fields) => response.quote = Some(self.quote(&fields).await?),
            AgentCommand::CreateVideo(fields) => {
                let created = self.create_video(fields, client.clone()).await?;
                tracing::info!(
                    jobs = created.jobs.len(),
                    known = created.known,
                    "an agent queued jobs"
                );
                response.created = Some(created);
            }
            AgentCommand::Jobs(fields) => response.jobs = Some(self.jobs(&fields).await),
            AgentCommand::JobAction(fields) => {
                let acted = Box::pin(self.act(&fields)).await?;
                tracing::info!(action = fields.action.as_str(), "an agent acted on its jobs");
                response.acted = Some(acted);
            }
            AgentCommand::Enhance(fields) => {
                if fields.is_on_device() {
                    response.created = Some(Box::pin(self.enhance_on_device(&fields, client)).await?);
                } else {
                    response.created = Some(Box::pin(self.enhance_on_higgsfield(&fields, client)).await?);
                }
            }
            AgentCommand::Subtitles(fields) => {
                let answer = Box::pin(self.subtitles(&fields, client)).await?;
                response.created = answer.created;
                response.subtitles = answer.subtitles;
            }
            AgentCommand::CreateImage(fields) => {
                response.created = Some(Box::pin(self.create_image(fields, client)).await?);
            }
            AgentCommand::LibrarySearch(fields) => {
                response.library = Some(self.library_search(&fields).await?);
            }
            AgentCommand::LibraryGet(fields) => response.details = Some(self.library_get(&fields).await?),
            AgentCommand::LibraryMark(fields) => response.marked = Some(self.library_mark(&fields).await?),
            AgentCommand::LibraryDownload(fields) => {
                response.downloads = Some(self.library_download(&fields).await?);
            }
            AgentCommand::Elements(fields) => response.elements = Some(self.elements(&fields).await?),
            AgentCommand::CreateElement(fields) => {
                response.created = Some(Box::pin(self.create_element(&fields, client)).await?);
            }
            AgentCommand::Export(fields) => {
                response.exported = Some(Box::pin(self.export_for_editing(&fields)).await?);
            }
            AgentCommand::DeliverySet(fields) => {
                response.delivery = Some(Box::pin(self.delivery_set(&fields)).await?);
            }
            AgentCommand::Frames(fields) => response.frames = Some(self.frames(&fields).await?),
        }
        Ok(response)
    }

    // MARK: capabilities

    async fn capabilities(&self, fields: &CapabilitiesCommand) -> Result<AgentCapabilities, AgentError> {
        let state = self.environment.connection.state();
        let kind = self.kind();
        let connection = match kind {
            None => Connection {
                kind: ConnectionKind::None,
                plan: None,
                credits: None,
            },
            Some(ProviderKind::ApiKey) => Connection {
                kind: ConnectionKind::ApiKey,
                plan: None,
                credits: None,
            },
            Some(ProviderKind::Account) => Connection {
                kind: ConnectionKind::Account,
                plan: state.account_info().and_then(|info| info.plan.clone()),
                credits: state.account_info().and_then(|info| info.credits),
            },
        };
        let mut answer = AgentCapabilities::new(&self.app_version, connection, self.budget_today().await?);
        answer.agents_allowed = self.preferences.is_allowed();
        answer.window = Some(Window {
            open: self.window.is_open(),
            menu_bar_mode: self.window.runs_in_tray(),
        });
        let mut on_device = self.on_device_capabilities();
        on_device.subtitles = Some(self.environment.subtitles.agent().capabilities());
        answer.on_device = Some(on_device);
        if fields.models {
            answer.models = Some(self.models(fields.model.as_deref(), kind).await?);
        }
        if fields.image_models {
            let (models, note) = self.image_models().await;
            answer.image_models = Some(models);
            answer.image_models_note = note;
        }
        if fields.styles {
            answer.styles = Some(self.styles());
        }
        if fields.voices {
            answer.voices = Some(self.voices().await);
        }
        Ok(answer)
    }

    /// Today's agent budget on the connection, with the person's own cap; `None` without a connection.
    pub async fn budget_today(&self) -> Result<Option<jutsu_agent::Budget>, AgentError> {
        let Some(kind) = self.kind() else {
            return Ok(None);
        };
        let taken = self.spend_today(kind).await?;
        let context = self.environment.create.agent_context().await;
        let daily_cap = context
            .cap
            .as_ref()
            .and_then(|cap| cap_money(cap.amount, cap.unit))
            .map(|money| money.amount);
        // Read on its own: the context counts a spend it can't read as 0.
        let cap_taken = match &context.cap {
            Some(cap) => budget::cap_taken(
                Some(cap),
                crate::settings::spending::committed_today(
                    &self.environment.store,
                    self.environment.images.spend_log(),
                    cap.unit,
                    OffsetDateTime::now_utc(),
                    self.environment.utc_offset,
                )
                .await,
            ),
            None => None,
        };
        Ok(Some(budget::budget(
            kind,
            self.preferences.daily_budget(kind),
            taken,
            self.preferences.allows_unlimited(),
            daily_cap,
            cap_taken,
        )))
    }

    /// What agents took today on `kind`'s connection.
    async fn spend_today(&self, kind: ProviderKind) -> Result<AgentSpendTotal, AgentError> {
        self.environment
            .agent_spend
            .total(OffsetDateTime::now_utc(), AgentMoney::unit_for(kind))
            .await
            .map_err(|error| {
                tracing::error!(%error, "can't read the agent spend");
                AgentError::new(
                    ErrorCode::FAILED,
                    "Jutsu Studio can't read what agents spent today, so agents can't spend.",
                    true,
                    Some("Ask the person to check the app's data folder."),
                )
            })
    }

    /// Every model, or the one asked for; whether each runs is what the provider last said.
    async fn models(
        &self,
        id: Option<&str>,
        connection: Option<ProviderKind>,
    ) -> Result<Vec<Model>, AgentError> {
        let models = match id {
            Some(id) => vec![VideoModel::from_raw(id).ok_or_else(|| {
                AgentError::new(
                    ErrorCode::NOT_FOUND,
                    format!("There's no model {id}."),
                    false,
                    Some("Call capabilities with models true to list them."),
                )
            })?],
            None => VideoModel::all(),
        };
        let context = self.environment.create.agent_context().await;
        let constraints = self.environment.create.agent_constraints();
        let capabilities = context.capabilities.as_ref().filter(|_| connection.is_some());
        let offered = context.offered_models();
        Ok(models
            .into_iter()
            .map(|model| {
                // The modes it runs on this connection: an API key has no Genjutsu Lite Object Swap.
                let unserved = capabilities
                    .and_then(|capabilities| capabilities.unserved_modes.get(&model).cloned())
                    .unwrap_or_default();
                let served: Vec<_> = model
                    .modes()
                    .into_iter()
                    .filter(|mode| !unserved.contains(mode))
                    .collect();
                let details = id.map(|_| {
                    let mode_details = served
                        .iter()
                        .map(|&mode| {
                            let has_source = !model.makes_clips_without_source()
                                || mode != jutsu_core::Mode::MotionTransfer;
                            let limits = model.limits(mode, has_source, &constraints);
                            let variants = variants(&model, mode, capabilities, context.unlimited_opt_in);
                            ModeDetails {
                                mode: mode_name(mode).to_owned(),
                                min_seconds: limits.duration.lower(),
                                max_seconds: limits.duration.upper(),
                                min_images: count(limits.reference_images.lower()),
                                max_images: count(limits.reference_images.upper()),
                                needs_style: limits.requires_preset,
                                needs_source: has_source,
                                resolutions: model
                                    .resolutions_for(mode)
                                    .iter()
                                    .map(|resolution| resolution.raw_value().to_owned())
                                    .collect(),
                                max_variants: variants.maximum,
                                variants_unavailable: variants.reason,
                            }
                        })
                        .collect();
                    ModelDetails {
                        modes: mode_details,
                        has_draft: model.offers_draft(),
                        makes_clips_without_source: model.makes_clips_without_source(),
                        needs_prompt: model.descriptor().requires_prompt,
                        takes_speech: model.declares(VideoControl::Speech).then_some(true),
                    }
                });
                Model {
                    id: model.raw_value().to_owned(),
                    title: model.title(),
                    modes: served.iter().map(|mode| mode_name(*mode).to_owned()).collect(),
                    resolutions: model
                        .resolutions()
                        .iter()
                        .map(|resolution| resolution.raw_value().to_owned())
                        .collect(),
                    available: model_available(connection, &offered, &model),
                    details,
                }
            })
            .collect())
    }

    // MARK: show

    async fn show(&self, fields: &ShowCommand) -> Result<AgentShown, AgentError> {
        let (target, shown) = if let (Some(item), Some(sheet)) = (fields.item, fields.sheet) {
            if let Some(refusal) = self.sheet_refusal(sheet, item).await {
                return Err(refusal);
            }
            if let Some(refusal) = on_device::sheet_open(sheet, self.window_sheet()) {
                return Err(refusal);
            }
            (
                AgentShowTarget::Sheet(app_id(item), sheet),
                AgentShown {
                    page: AgentPage::Library,
                    job: None,
                    item: Some(item),
                    sheet: Some(sheet),
                },
            )
        } else if let Some(id) = fields.job
            && self.job(app_id(id)).await.is_none()
            && let Some(image) = self
                .environment
                .images
                .agent_images()
                .and_then(|queue| queue.record(app_id(id)))
        {
            // An image request: the Queue lists it while it runs or waits, Library its images once over.
            let page = if image.is_over() {
                AgentPage::Library
            } else {
                AgentPage::Queue
            };
            (AgentShowTarget::Page(page), shown(page, Some(id)))
        } else if let Some(id) = fields.job {
            let Some(job) = self.job(app_id(id)).await else {
                return Err(AgentError::new(
                    ErrorCode::NOT_FOUND,
                    format!("There's no job {id}."),
                    false,
                    Some("Show the queue or the library instead."),
                ));
            };
            if job.state == JobState::Done {
                (
                    AgentShowTarget::LibraryJob(job.id),
                    shown(AgentPage::Library, Some(id)),
                )
            } else {
                (
                    AgentShowTarget::QueueJob(job.id),
                    shown(AgentPage::Queue, Some(id)),
                )
            }
        } else {
            let page = fields.page.unwrap_or(AgentPage::Create);
            (AgentShowTarget::Page(page), shown(page, None))
        };
        if !self.window.show(&target) {
            return Err(AgentError::new(
                ErrorCode::FAILED,
                "Jutsu Studio couldn't open its window.",
                true,
                Some(
                    "Ask the person to open Jutsu Studio from the taskbar or the system tray, then call show again.",
                ),
            ));
        }
        Ok(shown)
    }

    // MARK: quote

    async fn quote(&self, fields: &QuoteCommand) -> Result<AgentQuote, AgentError> {
        let video = &fields.video;
        // An element's images count as the references they become; reading them is free.
        let element_images: i64 = self
            .named_elements(&video.elements)
            .await?
            .iter()
            .map(|element| count(element.images.len()))
            .sum();
        let source = match &video.source {
            Some(given) => {
                let Some(end) = given.end else {
                    return Err(AgentError::bad_request(
                        "quote: source needs end, where the job's range ends in seconds",
                    ));
                };
                Some(AgentVideoOrder::quote_source(given.start, end))
            }
            None => None,
        };
        let images = AgentVideoOrder::quote_images(
            usize::try_from(video.reference_count() + element_images).unwrap_or(0),
        );
        let speech = video.speech_length.map(AgentVideoOrder::quote_speech);
        let (spec, pays_unlimited) = self.spec_for(&order(video, source, images, speech)).await?;
        let kind = self.kind();
        let taken = match kind {
            Some(kind) => Some(self.spend_today(kind).await?),
            None => None,
        };
        // The plan's unlimited allowance pays it: nothing counts against the budget or the cap.
        let unlimited = AgentSpendDecision::pays_unlimited(
            pays_unlimited,
            self.preferences.allows_unlimited(),
            taken.is_some_and(|taken| taken.unlimited_charged),
        );
        let cost = if unlimited {
            Some(Cost::credits(rust_decimal::Decimal::ZERO, false))
        } else {
            self.environment.create.agent_estimate(&spec).await
        };
        let price = cost.as_ref().and_then(AgentMoney::from_cost);
        let remaining = kind
            .zip(taken)
            .map(|(kind, taken)| taken.remaining(self.preferences.daily_budget(kind)));
        let is_exact = cost
            .as_ref()
            .is_some_and(|cost| !cost.is_estimate() && !cost.is_unknown_price());
        let context = self.environment.create.agent_context().await;
        let limits = spec
            .model
            .limits(
                spec.mode,
                spec.source.is_some(),
                &self.environment.create.agent_constraints(),
            )
            .reference_images;
        let variants = variants(
            &spec.model,
            spec.mode,
            context.capabilities.as_ref(),
            context.unlimited_opt_in,
        );
        Ok(AgentQuote {
            price: price.map(amount),
            is_exact,
            mode: video.mode.clone(),
            model: spec.model.raw_value().to_owned(),
            resolution: spec.resolution.raw_value().to_owned(),
            duration: spec.duration(),
            remaining_today: remaining,
            pays_unlimited,
            blocker: AgentQuote::blocker(
                price.map(|price| price.amount),
                remaining,
                unlimited,
                self.preferences.allows_unlimited(),
            ),
            price_note: AgentQuote::price_note(!is_exact, price.is_some(), unlimited),
            max_variants: variants.maximum,
            variants_unavailable: variants.reason,
            images: count(spec.references.len()),
            min_images: count(limits.lower()),
            max_images: count(limits.upper()),
            aspect_ratio: AgentVideoOrder::frame(&spec),
            frame_note: AgentVideoOrder::frame_note(&spec),
        })
    }

    /// The one job `order` makes, built and checked as Create does, and whether the plan's unlimited allowance would
    /// pay for it as things stand (the engine decides again when it is queued).
    async fn spec_for(&self, order: &AgentVideoOrder) -> Result<(JobSpec, bool), AgentError> {
        let create = &self.environment.create;
        let context = create.agent_context().await;
        if context.connection.is_none() {
            return Err(not_connected());
        }
        let spec = order
            .spec(
                create.agent_base_form(),
                &context,
                Some(create.agent_constraints()),
            )
            .map_err(AgentError::invalid_request)?;
        let pays_unlimited = context.pays_unlimited(&spec);
        Ok((spec, pays_unlimited))
    }

    // MARK: create_video

    async fn create_video(
        &self,
        fields: CreateVideoCommand,
        client: Option<String>,
    ) -> Result<AgentCreated, AgentError> {
        let started = Instant::now();
        let video = &fields.video;
        let files: Vec<String> = video
            .source
            .iter()
            .filter_map(|source| source.path.clone())
            .chain(video.images.iter().cloned())
            .chain(video.speech.iter().cloned())
            .collect();
        let me = self.strong()?;
        let raising = Arc::clone(&me);
        let raised_fields = fields.clone();
        let queued_fields = fields.clone();
        let request_id = &fields.request_id;
        let created = self
            .requests
            .run(
                &fields.request_id,
                &fields.fingerprint,
                fields.parameters.as_deref(),
                refusals("create_video", "its job was never queued"),
                || self.discard(&files),
                |ids| async move {
                    let jobs = self.known_jobs(&ids).await?;
                    Ok((!jobs.is_empty())
                        .then(|| AgentCreated::new(request_id, true, jobs.iter().map(agent_job).collect())))
                },
                Some(move |ids: Vec<Uuid>| async move { raising.raise_max_cost(&raised_fields, &ids).await }),
                move || async move { me.queue(&queued_fields, client).await },
            )
            .await;
        self.sweep_inbox().await;
        let created = created?;
        if fields.wait_seconds <= 0 {
            return Ok(created);
        }
        let seconds = jutsu_agent::wait_seconds(fields.wait_seconds, started.elapsed());
        Ok(self.wait_for_price(created, seconds).await)
    }

    /// The jobs of a known request. Fails when the store can't be read: a job not found there could still exist.
    async fn known_jobs(&self, ids: &[Uuid]) -> Result<Vec<Job>, AgentError> {
        let mut jobs = Vec::new();
        for &id in ids {
            if let Some(live) = self.environment.queue.live_job(id) {
                jobs.push(live);
                continue;
            }
            match self.environment.store.job(id).await {
                Ok(Some(stored)) => jobs.push(stored),
                Ok(None) => {}
                Err(_) => {
                    return Err(AgentError::new(
                        ErrorCode::FAILED,
                        "Jutsu Studio can't read its jobs right now.",
                        true,
                        Some("Send the same create_video again in a moment."),
                    ));
                }
            }
        }
        Ok(jobs)
    }

    /// Builds the job as Create would, records the request with the job's ID, then queues it.
    async fn queue(
        &self,
        fields: &CreateVideoCommand,
        client: Option<String>,
    ) -> Result<AgentCreated, AgentError> {
        let video = &fields.video;
        let Some(kind) = self.kind() else {
            return Err(not_connected());
        };
        check_unit(&fields.max_cost, kind)?;
        let source = match &video.source {
            Some(given) => {
                let Some(path) = &given.path else {
                    return Err(AgentError::bad_request(
                        "create_video: source needs path, the clip on this computer",
                    ));
                };
                let file = self.inbox_file(path)?;
                let probed = self
                    .environment
                    .create
                    .agent_source(&file)
                    .await
                    .map_err(|error| {
                        AgentError::invalid_request(format!(
                            "The source clip can't be read as a video: {error}"
                        ))
                    })?;
                Some(OrderSource {
                    video: probed,
                    start: given.start,
                    end: given.end,
                })
            }
            None => None,
        };
        let mut files = Vec::new();
        for path in &video.images {
            files.push(LocalImage::new(file_url(&self.inbox_file(path)?)?));
        }
        let elements = self.element_images(&video.elements).await?;
        let images: Vec<LocalImage> = elements
            .iter()
            .flat_map(|element| element.images.iter().cloned())
            .chain(files.iter().cloned())
            .collect();
        let speech = match &video.speech {
            Some(path) => {
                let file = self.inbox_file(path)?;
                let read = self.environment.create.agent_speech(&file).await;
                match read {
                    Ok(Some(speech)) => Some(speech),
                    Ok(None) => {
                        return Err(AgentError::invalid_request(
                            "The speech can't be read as audio: use an MP3, WAV or M4A recording.",
                        ));
                    }
                    Err(error) => {
                        return Err(AgentError::invalid_request(format!(
                            "The speech can't be read as audio: {error}"
                        )));
                    }
                }
            }
            None => None,
        };
        let (spec, _) = self.spec_for(&order(video, source, images, speech)).await?;
        let job_id = Uuid::new_v4();
        self.record_ids(
            &fields.request_id,
            &fields.fingerprint,
            fields.parameters.as_deref(),
            vec![job_id],
        )?;
        let origin = AgentOrigin::new(client, fields.request_id.clone(), money(fields.max_cost));
        let options = EnqueueOptions {
            id: Some(job_id),
            agent: Some(origin),
            ..EnqueueOptions::default()
        };
        let job = self
            .environment
            .engine
            .enqueue(spec, options)
            .await
            .map_err(|error| enqueue_error(&error))?;
        let mut created = AgentCreated::new(&fields.request_id, false, vec![agent_job(&job)]);
        created.references = images::references(&elements, &files);
        Ok(created)
    }

    /// Records the request with the IDs of what it makes, before anything is queued or sent.
    fn record_ids(
        &self,
        request_id: &str,
        fingerprint: &str,
        parameters: Option<&str>,
        ids: Vec<Uuid>,
    ) -> Result<(), AgentError> {
        self.requests
            .record(
                request_id,
                fingerprint,
                parameters,
                ids,
                OffsetDateTime::now_utc(),
                None,
            )
            .map_err(|_| {
                AgentError::new(
                    ErrorCode::FAILED,
                    "Jutsu Studio can't record the request, so nothing is queued or sent.",
                    true,
                    Some("Ask the person to check the computer has room, then send the request again."),
                )
            })
    }

    /// The request sent again with a higher `max_cost`: the engine lifts its job's `over_max_cost` hold when the price
    /// now fits every limit, and the request then stands for the new `max_cost`.
    async fn raise_max_cost(
        &self,
        fields: &CreateVideoCommand,
        ids: &[Uuid],
    ) -> Result<AgentRaiseOutcome<AgentCreated>, AgentError> {
        let engine = &self.environment.engine;
        let outcome = raising::answer(
            ids,
            money(fields.max_cost),
            |id, max_cost| async move {
                match engine.raise_agent_max_cost(id, max_cost).await {
                    Ok(raise) => Ok(Some(raise)),
                    Err(EngineError::UnknownJob(_)) => Ok(None),
                    Err(error) => Err(AgentError::new(
                        ErrorCode::FAILED,
                        format!("Jutsu Studio can't check the job again: {error}"),
                        true,
                        Some("Send the same create_video again in a moment."),
                    )),
                }
            },
            |id| async move { Ok(self.known_jobs(&[id]).await?.into_iter().next()) },
            || self.requests.update(&fields.request_id, &fields.fingerprint),
        )
        .await?;
        Ok(outcome.map(|AgentRaisedJob { job, note }| {
            let mut answer = agent_job(&job);
            if let Some(note) = note {
                answer.next = Some(note);
            }
            AgentCreated::new(&fields.request_id, true, vec![answer])
        }))
    }

    /// Waits until every job's price is known or it is held, then answers with the jobs as they are now.
    async fn wait_for_price(&self, mut created: AgentCreated, seconds: i64) -> AgentCreated {
        let ids: Vec<Uuid> = created.jobs.iter().map(|job| app_id(job.id)).collect();
        let timed_out = agent_wait(seconds, || self.all_jobs(&ids, is_priced)).await;
        let mut jobs = Vec::new();
        for earlier in &created.jobs {
            let now = self.job(app_id(earlier.id)).await;
            jobs.push(now.map_or_else(|| earlier.clone(), |job| agent_job(&job).keeping_next(earlier)));
        }
        created.jobs = jobs;
        created.timed_out = Some(timed_out);
        created
    }

    // MARK: jobs

    async fn jobs(&self, fields: &JobsCommand) -> AgentJobs {
        let started = Instant::now();
        let (ids, next) = if fields.ids.is_empty() {
            let mut entries: Vec<_> = self
                .environment
                .queue
                .agent_jobs()
                .iter()
                .map(|job| (job.created_at, job.id))
                .collect();
            entries.extend(self.image_entries());
            entries.extend(self.on_device_entries());
            entries.extend(self.subtitle_entries());
            entries.extend(self.operation_entries());
            let limit = fields
                .limit
                .and_then(|limit| usize::try_from(limit).ok())
                .unwrap_or(10);
            page(entries, limit, fields.cursor.as_deref())
        } else {
            (fields.ids.iter().copied().map(app_id).collect(), None)
        };
        let seconds = jutsu_agent::wait_seconds(fields.wait_seconds, started.elapsed());
        let timed_out = if fields.wait_seconds > 0 {
            agent_wait(seconds, || self.all_settled(&ids)).await
        } else {
            false
        };
        let mut jobs = Vec::new();
        for id in &ids {
            let job = match self.job(*id).await {
                Some(job) => agent_job(&job),
                None => match self
                    .on_device_job(*id)
                    .or_else(|| self.subtitle_job(*id))
                    .or_else(|| self.operation_job(*id))
                {
                    Some(job) => job,
                    None => self
                        .image_job(*id)
                        .await
                        .unwrap_or_else(|| AgentJob::unknown(wire_id(*id))),
                },
            };
            jobs.push(job);
        }
        AgentJobs {
            jobs,
            timed_out,
            next_cursor: next,
        }
    }

    /// Whether every job of `ids` the app knows is `done`; one it doesn't know counts as done.
    async fn all_jobs(&self, ids: &[Uuid], done: impl Fn(&Job) -> bool) -> bool {
        for id in ids {
            if let Some(job) = self.job(*id).await
                && !done(&job)
            {
                return false;
            }
        }
        true
    }

    /// Whether every video job and image request of `ids` is settled; one the app doesn't know counts as settled.
    async fn all_settled(&self, ids: &[Uuid]) -> bool {
        for &id in ids {
            let settled = match self.job(id).await {
                Some(job) => is_settled(&job.state),
                None => {
                    self.image_is_settled(id)
                        && self.on_device_is_settled(id)
                        && self.subtitles_settled(id)
                        && self.operation_is_settled(id)
                }
            };
            if !settled {
                return false;
            }
        }
        true
    }

    async fn job(&self, id: Uuid) -> Option<Job> {
        if let Some(job) = self.environment.queue.live_job(id) {
            return Some(job);
        }
        self.environment.store.job(id).await.ok().flatten()
    }

    // MARK: job_action

    async fn act(&self, fields: &JobActionCommand) -> Result<AgentActed, AgentError> {
        let engine = &self.environment.engine;
        let Some(id) = fields.job.map(app_id) else {
            match fields.action {
                // Agents' jobs only: an agent never pauses the person's jobs or lifts their own Pause all.
                AgentJobAction::PauseQueue => engine.pause_all(true).await,
                AgentJobAction::ResumeQueue => engine.resume_all(true).await,
                AgentJobAction::Cancel | AgentJobAction::Retry | AgentJobAction::RunNext => {
                    return Err(AgentError::bad_request(format!(
                        "{} needs job",
                        fields.action.as_str()
                    )));
                }
            }
            return Ok(AgentActed {
                action: fields.action,
                job: None,
            });
        };
        let Some(job) = self.job(id).await else {
            if let Some(acted) = self.act_on_device(fields.action, id).await? {
                return Ok(acted);
            }
            if let Some(acted) = self.act_on_subtitles(fields.action, id).await? {
                return Ok(acted);
            }
            let is_image = self
                .environment
                .images
                .agent_images()
                .is_some_and(|queue| queue.record(id).is_some());
            if is_image {
                return self.act_on_image(fields.action, id).await;
            }
            return Err(AgentError::new(
                ErrorCode::NOT_FOUND,
                format!("There's no job {}.", wire_id(id)),
                false,
                None,
            ));
        };
        let refusal = match fields.action {
            AgentJobAction::Cancel => AgentJobRules::cancel_refusal(&job),
            AgentJobAction::Retry => {
                AgentJobRules::retry_refusal(&job, |url| url.to_file_path().is_ok_and(|path| path.exists()))
            }
            AgentJobAction::RunNext => AgentJobRules::run_next_refusal(&job),
            AgentJobAction::PauseQueue | AgentJobAction::ResumeQueue => None,
        };
        if let Some(refusal) = refusal {
            return Err(AgentError::new(
                ErrorCode::NEEDS_PERSON,
                refusal,
                false,
                Some("Ask the person to decide in the Queue."),
            ));
        }
        let result = match fields.action {
            AgentJobAction::Cancel => engine.cancel(id).await,
            AgentJobAction::Retry => engine.retry(id).await,
            AgentJobAction::RunNext => engine.run_next(id).await,
            AgentJobAction::PauseQueue | AgentJobAction::ResumeQueue => Ok(()),
        };
        if let Err(error) = result {
            return Err(AgentError::new(
                ErrorCode::NOT_APPLICABLE,
                error.to_string(),
                matches!(error, EngineError::SubmitInProgress),
                Some("Call jobs to see the job's state now."),
            ));
        }
        Ok(AgentActed {
            action: fields.action,
            job: self.job(id).await.map(|job| agent_job(&job)),
        })
    }

    // MARK: Uploads and the inbox

    /// Checks an upload's request before a byte of it is read: only commands with files take one, a request ID taken
    /// by other parameters is refused (`request_conflict`), and one already queued with these needs no file again
    /// (`request_known`).
    pub fn admit(&self, upload: &UploadCommand) -> Result<(), AgentError> {
        const COMMANDS: [&str; 4] = ["create_video", "create_image", "create_element", "frames"];
        let Some(request) = upload
            .request
            .as_ref()
            .filter(|request| COMMANDS.contains(&request.command.as_str()))
        else {
            return Err(AgentError::new(
                ErrorCode::FILE_REFUSED,
                "Only create_video, create_image, create_element and frames take files.",
                false,
                Some("Send files with create_video, create_image, create_element or frames."),
            ));
        };
        // Free and never logged as a request: its files are removed once its frames are written.
        if request.command == "frames" {
            return Ok(());
        }
        let state = self
            .requests
            .state(
                &request.request_id,
                &request.fingerprint,
                request.parameters.as_deref(),
            )
            .map_err(|error| {
                tracing::error!(%error, "can't read the agent requests");
                requests_unreadable()
            })?;
        match state {
            AgentRequestState::New | AgentRequestState::Running => Ok(()),
            AgentRequestState::Conflict => Err(AgentError::request_conflict()),
            // Sent again with another max_cost: its job has the files already, whatever the new one decides.
            AgentRequestState::Known(_) | AgentRequestState::OtherMaxCost(_) => Err(AgentError::new(
                ErrorCode::REQUEST_KNOWN,
                "This request was already queued; its files aren't needed again.",
                false,
                Some(&format!(
                    "Send {} without uploading to get its job.",
                    request.command
                )),
            )),
        }
    }

    /// A path the helper's upload answered with: a file in the app's own inbox, and nowhere else.
    fn inbox_file(&self, path: &str) -> Result<PathBuf, AgentError> {
        let inside = self
            .inbox
            .as_ref()
            .is_some_and(|inbox| inbox.upload_folder(path).is_some());
        if inside && Path::new(path).is_file() {
            return Ok(PathBuf::from(path));
        }
        Err(AgentError::new(
            ErrorCode::FILE_REFUSED,
            format!("{path} isn't a file the helper sent to Jutsu Studio."),
            false,
            Some("Pass the file's path on this computer; the jutsu helper sends it to the app."),
        ))
    }

    fn discard(&self, files: &[String]) {
        if let Some(inbox) = &self.inbox {
            inbox.remove(files);
        }
    }

    /// Removes the inbox's files that no job still reads, a day after they came: refused requests, and jobs gone
    /// from the Queue. The jobs the Queue holds keep theirs.
    pub async fn sweep_inbox(&self) {
        let Some(inbox) = self.inbox.clone() else {
            return;
        };
        let week_ago = OffsetDateTime::now_utc() - time::Duration::days(7);
        let jobs = match self.environment.store.jobs_unfinished_or_finished(week_ago).await {
            Ok(jobs) => jobs,
            Err(error) => {
                tracing::error!(%error, "can't check the agent inbox");
                return;
            }
        };
        let records = self
            .environment
            .images
            .agent_images()
            .and_then(|queue| queue.records().ok())
            .unwrap_or_default();
        let kept = kept_inputs(&jobs, &records);
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_hours(24))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let removed = tokio::task::spawn_blocking(move || inbox.sweep(&kept, cutoff))
            .await
            .unwrap_or(0);
        if removed > 0 {
            tracing::info!(removed, "removed old agent uploads");
        }
    }
}

/// The inbox files a job or an image request still reads: the source and images of `jobs` (the store's unfinished
/// ones and those finished within the week), and the inputs of image requests not over.
pub fn kept_inputs(jobs: &[Job], records: &[super::image_record::AgentImageRecord]) -> HashSet<String> {
    let job_files = jobs.iter().flat_map(|job| {
        job.spec
            .source
            .iter()
            .map(|source| source.file_url.clone())
            .chain(job.spec.references.iter().map(|image| image.file_url.clone()))
            .collect::<Vec<_>>()
    });
    let image_files = records
        .iter()
        .filter(|record| !record.is_over())
        .flat_map(|record| record.inputs().into_iter().map(|image| image.file_url));
    job_files
        .chain(image_files)
        .filter_map(|url| url.to_file_path().ok())
        .map(|path| path.display().to_string())
        .collect()
}

#[async_trait]
impl AgentHandler for AgentService {
    fn agents_allowed(&self) -> bool {
        self.preferences.is_allowed()
    }

    fn check_upload(&self, upload: &UploadCommand, _client: Option<&str>) -> Result<(), AgentError> {
        self.admit(upload)
    }

    async fn handle(&self, request: AgentRequest) -> Result<AgentResponse, AgentError> {
        self.note_request(request.client.as_deref());
        let asked = asked_item(&request.command);
        let result = Box::pin(self.run(request.command, request.client)).await;
        match (&result, asked) {
            (Ok(_), Some((item, sheet))) => self.with_activity(|activity| activity.clear_ask(item, sheet)),
            (Err(error), Some((item, sheet))) if error.code == ErrorCode::NEEDS_PERSON => {
                // note: the title comes from the on-device item (`itemTitle`), which this version has no engine for, so it
                // is Swift's fallback; upgrade once on-device enhance and subtitles are ported.
                self.with_activity(|activity| {
                    activity.note_ask(Ask {
                        item,
                        sheet,
                        title: "Library item".to_owned(),
                        asked_at: OffsetDateTime::now_utc(),
                    });
                });
            }
            _ => {}
        }
        result
    }

    fn free_space(&self, path: &Path) -> Option<i64> {
        VolumeFreeSpace
            .free_bytes(path)
            .and_then(|bytes| i64::try_from(bytes).ok())
    }
}

/// The item and sheet a request asks the person about when it is refused with `needs_person` (`askedItem`).
pub fn asked_item(command: &AgentCommand) -> Option<(Uuid, AgentSheet)> {
    match command {
        AgentCommand::Enhance(fields) => Some((app_id(fields.item), AgentSheet::UpscaleOnMac)),
        AgentCommand::Subtitles(fields) if fields.action == AgentSubtitlesAction::Make => {
            Some((app_id(fields.item), AgentSheet::Subtitles))
        }
        _ => None,
    }
}

fn shown(page: AgentPage, job: Option<jutsu_agent::Uuid>) -> AgentShown {
    AgentShown {
        page,
        job,
        item: None,
        sheet: None,
    }
}

fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn file_url(path: &Path) -> Result<Url, AgentError> {
    Url::from_file_path(path)
        .map_err(|()| AgentError::invalid_request(format!("Can't open {}.", path.display())))
}

/// The request's fields as an order, with its files as the app holds them.
fn order(
    video: &AgentVideoRequest,
    source: Option<OrderSource>,
    images: Vec<LocalImage>,
    speech: Option<jutsu_core::LocalAudio>,
) -> AgentVideoOrder {
    AgentVideoOrder {
        mode: video.mode.clone(),
        model: video.model.clone(),
        prompt: video.prompt.clone().unwrap_or_default(),
        source,
        images,
        style: video.style.map(app_id),
        resolution: video.resolution.clone(),
        is_draft: video.draft,
        length: video.length,
        aspect_ratio: video.aspect_ratio.clone(),
        audio: video.audio.clone(),
        variants: video.variants,
        title: video.title.clone(),
        speech,
    }
}

#[path = "service_enhance.rs"]
mod enhance;
#[path = "service_exports.rs"]
mod exports;
#[path = "service_frames.rs"]
mod frames;
#[path = "service_images.rs"]
mod images;
#[path = "service_library.rs"]
mod library;
#[path = "service_on_device.rs"]
mod on_device;
#[path = "service_subtitles.rs"]
mod subtitles;

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
