//! Create Character (`CharacterBuilder.swift`, `CharacterSheetLog.swift`, the quote's price in `CostText.swift`):
//! a person or an animal, its traits or a preset, a description, an optional face and style references, Randomize
//! and the number of sheets; the exact price before Generate; and the sheets it starts. The price is the provider's
//! quote of the exact request, and Generate sends that quoted request, never the form again: a randomized design
//! differs each time it is quoted.
//!
//! Money: one click sends one submit, and a submit is never sent again. Its price is written to today's spend before
//! it is sent, and nothing is sent when that write fails; it is taken off when the provider refused it, and kept when
//! the outcome is unknown. Every job ID of the answer goes into `CharacterSheetLog` before anything else happens, and
//! is polled until it ends, after a relaunch too (`resume`).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use jutsu_core::{
    Backoff, CharacterDesign, CharacterPreset, CharacterQuote, CharacterSheet, CharacterSheetMaker,
    CharacterTier, CharacterTierKind, CharacterTraitGroup, ImageCharge, ImageChargePurpose, ImageJobStatus,
    LocalImage, PortError, ProviderError, ProviderKind, RemoteAsset, RemoteJob,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use super::form::cap_confirmation;
use super::generator::{PaidStart, SleepFn, SpendStatusReader, UploadFn, interrupted, was_refused};
use super::log_file::{LogFile, LogWriteError};
use super::spend_log::ImageSpendLog;
use crate::connection::UserMessage;
use crate::queue::format;

pub const CHARGE_TITLE: &str = "Character sheets";
pub const COULD_NOT_MAKE: &str = "Higgsfield couldn't make the character sheet.";
pub const CANT_PRICE: &str = "Can't get the price of this character.";
pub const CANT_LOAD_TRAITS: &str = "Can't load the traits.";
pub const MAY_HAVE_STARTED: &str = "Higgsfield may have started these sheets, so they weren't sent again. Check your history on higgsfield.ai before trying again.";
const REFUSED_BY_CHECK: &str = "Higgsfield's content check refused the character.";
const BRIEF_TOO_LONG: &str = "Keep the description under 4,000 characters.";

/// The quote in fractional credits, never rounded down: "1.125 cr".
pub fn quote_price(quote: &CharacterQuote) -> String {
    format::credits(quote.credits)
}

/// "4.5 cr · Price from Higgsfield".
pub fn quote_price_text(quote: &CharacterQuote) -> String {
    format!("{} · Price from Higgsfield", quote_price(quote))
}

/// "Natural", "Bold"…
pub fn tier_title(tier: CharacterTier) -> &'static str {
    match tier {
        CharacterTier::Normal => "Natural",
        CharacterTier::Freak => "Bold",
        CharacterTier::Total => "Wild",
        CharacterTier::Insects => "Insect",
        CharacterTier::Frogs => "Frog",
        CharacterTier::Cats => "Cat",
        CharacterTier::Dogs => "Dog",
        CharacterTier::Capybaras => "Capybara",
        CharacterTier::Birds => "Bird",
    }
}

/// "Human" or "Animal".
pub fn kind_title(kind: CharacterTierKind) -> &'static str {
    match kind {
        CharacterTierKind::Human => "Human",
        CharacterTierKind::Animal => "Animal",
    }
}

// MARK: Log

/// Where a sheet job is, coded as Swift codes the enum: `{"failed":{"_0":"…"}}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CharacterSheetState {
    Pending {},
    Completed {},
    Failed {
        #[serde(rename = "_0", default)]
        message: Option<String>,
    },
    Rejected {},
}

/// One character sheet job the app started: kept from the moment its submit was answered, so a job that is paid for
/// is never lost track of, through a quit or a sign-out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSheetRecord {
    /// The provider's job ID.
    pub id: String,
    /// `GenerationProvider::owner` of the account that started it.
    #[serde(default)]
    pub owner: Option<String>,
    /// The request it came from, which is also its `ImageCharge`'s ID; one request makes up to four sheets.
    #[serde(rename = "requestID")]
    pub request_id: Uuid,
    #[serde(with = "jutsu_core::support::date::rfc3339")]
    pub created_at: OffsetDateTime,
    pub brief: String,
    pub state: CharacterSheetState,
    /// Where the provider serves the finished sheet; its links expire.
    #[serde(rename = "imageURL", default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<Url>,
    /// The copy saved in Library › Images.
    #[serde(rename = "fileURL", default, skip_serializing_if = "Option::is_none")]
    pub file_url: Option<Url>,
}

impl CharacterSheetRecord {
    /// A pending job.
    pub fn new(
        id: &str,
        owner: Option<&str>,
        request_id: Uuid,
        created_at: OffsetDateTime,
        brief: &str,
    ) -> Self {
        Self {
            id: id.to_owned(),
            owner: owner.map(str::to_owned),
            request_id,
            created_at,
            brief: brief.to_owned(),
            state: CharacterSheetState::Pending {},
            image_url: None,
            file_url: None,
        }
    }

    fn is_pending(&self) -> bool {
        self.state == CharacterSheetState::Pending {}
    }

    /// The finished sheet as the Characters source lists it; `None` until it is finished.
    pub fn sheet(&self) -> Option<CharacterSheet> {
        if self.state != (CharacterSheetState::Completed {}) {
            return None;
        }
        Some(CharacterSheet {
            id: self.id.clone(),
            image_url: self.image_url.clone()?,
            thumbnail_url: None,
            created_at: Some(self.created_at),
            file_url: self.file_url.clone(),
        })
    }
}

/// The character sheet jobs the app started, kept in `character-sheets.json` next to the store. The Characters
/// source lists these first, before the account's own list, and a job still running after a quit is followed again
/// from here. A write that fails keeps the records in memory, says why in `save_error` and lists the job IDs only
/// this launch knows in `unsaved_ids`.
#[derive(Debug)]
pub struct CharacterSheetLog {
    file: LogFile<CharacterSheetRecord>,
    state: Mutex<SheetLogState>,
}

#[derive(Debug, Default)]
struct SheetLogState {
    records: Vec<CharacterSheetRecord>,
    save_error: Option<LogWriteError>,
    unsaved_ids: Vec<String>,
}

impl CharacterSheetLog {
    pub const FILE_NAME: &'static str = "character-sheets.json";

    /// `None` keeps everything in memory.
    pub fn open(folder: Option<&Path>) -> Self {
        let file = LogFile::new(folder, Self::FILE_NAME);
        let records = file.read();
        Self {
            file,
            state: Mutex::new(SheetLogState {
                records,
                ..SheetLogState::default()
            }),
        }
    }

    fn locked(&self) -> MutexGuard<'_, SheetLogState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Oldest first.
    pub fn records(&self) -> Vec<CharacterSheetRecord> {
        self.locked().records.clone()
    }

    pub fn save_error(&self) -> Option<LogWriteError> {
        self.locked().save_error.clone()
    }

    pub fn unsaved_ids(&self) -> Vec<String> {
        self.locked().unsaved_ids.clone()
    }

    /// The finished sheets of `owner`, newest first.
    pub fn sheets(&self, owner: Option<&str>) -> Vec<CharacterSheet> {
        self.locked()
            .records
            .iter()
            .rev()
            .filter(|record| record.owner.as_deref() == owner)
            .filter_map(CharacterSheetRecord::sheet)
            .collect()
    }

    /// The jobs of `owner` still running.
    pub fn pending(&self, owner: Option<&str>) -> Vec<CharacterSheetRecord> {
        self.locked()
            .records
            .iter()
            .filter(|record| record.owner.as_deref() == owner && record.is_pending())
            .cloned()
            .collect()
    }

    pub fn add(&self, new: Vec<CharacterSheetRecord>) {
        let mut state = self.locked();
        for record in new {
            if state.records.iter().any(|old| old.id == record.id) {
                continue;
            }
            state.unsaved_ids.push(record.id.clone());
            state.records.push(record);
        }
        self.save(&mut state);
    }

    pub fn update(&self, id: &str, new: CharacterSheetState, image_url: Option<Url>) {
        let mut state = self.locked();
        let Some(record) = state.records.iter_mut().find(|record| record.id == id) else {
            return;
        };
        record.state = new;
        if let Some(url) = image_url {
            record.image_url = Some(url);
        }
        self.save(&mut state);
    }

    /// Where the finished sheet was saved.
    pub fn set_file(&self, file: Url, id: &str) {
        let mut state = self.locked();
        let Some(record) = state.records.iter_mut().find(|record| record.id == id) else {
            return;
        };
        record.file_url = Some(file);
        self.save(&mut state);
    }

    /// Writes the records again after `save_error`.
    pub fn retry_save(&self) {
        let mut state = self.locked();
        self.save(&mut state);
    }

    fn save(&self, state: &mut SheetLogState) {
        match self.file.write(&state.records) {
            Ok(()) => {
                state.save_error = None;
                state.unsaved_ids.clear();
            }
            Err(error) => state.save_error = Some(error),
        }
    }
}

// MARK: Builder

/// Where one Generate of Create Character is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CharacterRunState {
    Sending,
    Running,
    Finished,
    Failed(String),
    /// Following it stopped before it ended: the sign-in expired or its status couldn't be read. Its sheets and
    /// price are kept, and it is followed again (`resume`).
    Interrupted(String),
}

/// One Generate of Create Character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRun {
    pub id: Uuid,
    pub count: i64,
    pub state: CharacterRunState,
}

/// Called with each sheet as it finishes, to save it.
pub type SheetsHandler = Arc<dyn Fn(Vec<CharacterSheetRecord>) + Send + Sync>;
/// Called when Higgsfield turned a poll down because the sign-in no longer works.
pub type SignInHandler = Arc<dyn Fn() + Send + Sync>;

/// What the form holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterForm {
    pub tier: CharacterTier,
    /// Trait IDs by group ID.
    pub traits: BTreeMap<String, Vec<String>>,
    pub brief: String,
    /// The seed of the preset the form came from.
    pub seed: Option<i64>,
    pub preset: Option<CharacterPreset>,
    /// The face the sheets keep.
    pub face: Option<LocalImage>,
    pub styles: Vec<LocalImage>,
    /// Fill the traits left unset at random.
    pub randomize: bool,
    /// Sheets to make, 1–4.
    pub count: i64,
}

impl Default for CharacterForm {
    fn default() -> Self {
        Self {
            tier: CharacterTier::Normal,
            traits: BTreeMap::new(),
            brief: String::new(),
            seed: None,
            preset: None,
            face: None,
            styles: Vec::new(),
            randomize: true,
            count: 1,
        }
    }
}

#[derive(Default)]
struct BuilderState {
    form: CharacterForm,
    trait_groups: Vec<CharacterTraitGroup>,
    traits_failure: Option<String>,
    traits_by_tier: HashMap<CharacterTier, Vec<CharacterTraitGroup>>,
    /// The last quote; `current_quote` says whether it is still the form's.
    quote: Option<CharacterQuote>,
    quoted_key: Option<String>,
    failure: Option<(String, String)>,
    /// Goes up after each submit, so the next Generate is quoted afresh.
    round: u64,
    /// A submit is out; Generate waits for its answer.
    is_submitting: bool,
    runs: Vec<CharacterRun>,
    uploads: HashMap<LocalImage, RemoteAsset>,
    /// Goes up with each connection (`reset`), so an upload that outlived the old one isn't kept.
    generation: u64,
    /// The polls, by request.
    followers: HashMap<Uuid, CancellationToken>,
}

/// Create Character's form, price and runs.
pub struct CharacterBuilder {
    log: Arc<CharacterSheetLog>,
    spend: Arc<ImageSpendLog>,
    spend_status: SpendStatusReader,
    sleep: SleepFn,
    state: Mutex<BuilderState>,
    on_sheets: Mutex<Option<SheetsHandler>>,
    on_sign_in_expired: Mutex<Option<SignInHandler>>,
    submitted: Notify,
    changes: watch::Sender<u64>,
}

impl std::fmt::Debug for CharacterBuilder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("CharacterBuilder").finish_non_exhaustive()
    }
}

/// A poll that failed for a reason waiting can fix.
fn is_transient(error: &PortError) -> bool {
    matches!(
        error,
        PortError::Provider(
            ProviderError::Network
                | ProviderError::Server { .. }
                | ProviderError::ModelUnavailable
                | ProviderError::ConcurrencyLimit { .. }
        )
    )
}

fn message(error: &PortError, fallback: &str) -> String {
    UserMessage::from_error(error, Some(ProviderKind::Account))
        .map_or_else(|| fallback.to_owned(), |message| message.message)
}

impl CharacterBuilder {
    pub fn new(
        log: Arc<CharacterSheetLog>,
        spend: Arc<ImageSpendLog>,
        spend_status: SpendStatusReader,
        sleep: SleepFn,
    ) -> Arc<Self> {
        Arc::new(Self {
            log,
            spend,
            spend_status,
            sleep,
            state: Mutex::new(BuilderState::default()),
            on_sheets: Mutex::new(None),
            on_sign_in_expired: Mutex::new(None),
            submitted: Notify::new(),
            changes: watch::Sender::new(0),
        })
    }

    fn locked(&self) -> MutexGuard<'_, BuilderState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn log(&self) -> &Arc<CharacterSheetLog> {
        &self.log
    }

    /// Bumps whenever the form, the quote or the runs change.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn changed(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    pub fn set_on_sheets(&self, handler: SheetsHandler) {
        *self.on_sheets.lock().unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }

    pub fn set_on_sign_in_expired(&self, handler: SignInHandler) {
        *self
            .on_sign_in_expired
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(handler);
    }

    // MARK: Reads

    pub fn form(&self) -> CharacterForm {
        self.locked().form.clone()
    }

    pub fn trait_groups(&self) -> Vec<CharacterTraitGroup> {
        self.locked().trait_groups.clone()
    }

    pub fn traits_failure(&self) -> Option<String> {
        self.locked().traits_failure.clone()
    }

    pub fn runs(&self) -> Vec<CharacterRun> {
        self.locked().runs.clone()
    }

    pub fn is_submitting(&self) -> bool {
        self.locked().is_submitting
    }

    /// Requests being sent or followed until their sheets arrive.
    pub fn active_count(&self) -> usize {
        let state = self.locked();
        usize::from(state.is_submitting) + state.followers.len()
    }

    /// The sheets of the requests running now, for placeholders.
    pub fn pending_count(&self, owner: Option<&str>) -> usize {
        self.log.pending(owner).len()
    }

    // MARK: Traits

    /// The trait groups of the tier, read once per tier and connection.
    pub async fn load_traits(&self, maker: &dyn CharacterSheetMaker) {
        let tier = {
            let mut state = self.locked();
            let tier = state.form.tier;
            if let Some(cached) = state.traits_by_tier.get(&tier).cloned() {
                state.trait_groups = cached;
                drop(state);
                self.changed();
                return;
            }
            state.traits_failure = None;
            tier
        };
        let result = maker.character_traits(tier).await;
        {
            let mut state = self.locked();
            match result {
                Ok(groups) => {
                    state.traits_by_tier.insert(tier, groups.clone());
                    if state.form.tier == tier {
                        state.trait_groups = groups;
                    }
                }
                Err(PortError::Cancelled) => return,
                Err(error) => {
                    if state.form.tier == tier {
                        state.traits_failure = Some(message(&error, CANT_LOAD_TRAITS));
                    }
                }
            }
        }
        self.changed();
    }

    /// A new connection: its traits are read again, and its uploads aren't the old one's.
    pub fn reset(&self) {
        self.stop_following();
        {
            let mut state = self.locked();
            state.generation += 1;
            state.traits_by_tier.clear();
            state.trait_groups.clear();
            state.uploads.clear();
            state.quote = None;
            state.quoted_key = None;
            state.failure = None;
            state.form.preset = None;
            state.form.seed = None;
        }
        self.changed();
    }

    // MARK: Price

    /// Why there is no price or Generate; `None` when the form can be quoted.
    pub fn blocker(&self) -> Option<String> {
        blocker(&self.locked().form)
    }

    /// Everything the request is made of, and the round.
    pub fn quote_key(&self) -> String {
        let state = self.locked();
        quote_key(&state.form, state.round)
    }

    /// The quote of the form as it stands; `None` while it is being read, or when it can't be.
    pub fn current_quote(&self) -> Option<CharacterQuote> {
        let state = self.locked();
        current_quote(&state)
    }

    /// Why the form as it stands has no price.
    pub fn quote_failure(&self) -> Option<String> {
        let state = self.locked();
        let key = quote_key(&state.form, state.round);
        state
            .failure
            .as_ref()
            .filter(|(failed, _)| *failed == key)
            .map(|(_, message)| message.clone())
    }

    pub fn is_quoting(&self) -> bool {
        self.blocker().is_none() && self.current_quote().is_none() && self.quote_failure().is_none()
    }

    /// Asks the provider for the exact request and price of the form, uploading the face and styles first unless
    /// they already were. Free: nothing is generated.
    pub async fn refresh_quote(&self, maker: &dyn CharacterSheetMaker, upload: &UploadFn) {
        if self.blocker().is_some() {
            return;
        }
        let key = self.quote_key();
        let quote = match self.design(upload).await {
            Ok(design) => maker.quote_characters(design).await,
            Err(error) => Err(error),
        };
        {
            let mut state = self.locked();
            if quote_key(&state.form, state.round) != key {
                return;
            }
            match quote {
                Ok(quote) => {
                    state.quote = Some(quote);
                    state.quoted_key = Some(key);
                }
                Err(PortError::Cancelled) => return,
                Err(error) => state.failure = Some((key, message(&error, CANT_PRICE))),
            }
        }
        self.changed();
    }

    async fn asset(&self, image: &LocalImage, upload: &UploadFn) -> Result<RemoteAsset, PortError> {
        let (cached, generation) = {
            let state = self.locked();
            (state.uploads.get(image).cloned(), state.generation)
        };
        if let Some(asset) = cached {
            return Ok(asset);
        }
        let asset = upload(image.clone()).await?;
        // An upload that outlived its connection is the old account's media: never offered to the new one.
        let mut state = self.locked();
        if state.generation == generation {
            state.uploads.insert(image.clone(), asset.clone());
        }
        Ok(asset)
    }

    async fn design(&self, upload: &UploadFn) -> Result<CharacterDesign, PortError> {
        let form = self.form();
        let mut styles = Vec::with_capacity(form.styles.len());
        for style in &form.styles {
            styles.push(self.asset(style, upload).await?);
        }
        let face = match &form.face {
            Some(face) => Some(self.asset(face, upload).await?),
            None => None,
        };
        Ok(CharacterDesign {
            tier: Some(form.tier),
            traits: form.traits,
            brief: form.brief.trim().to_owned(),
            seed: form.seed,
            count: form.count,
            face,
            styles,
            randomize: form.randomize,
        })
    }

    // MARK: Generate

    /// Sends the current quote once and follows its sheets. Refused while a submit is out or without a quote of the
    /// form as it stands, and when its price can't be written first (the run says so). Unless `over_cap_approved`,
    /// today's spend is read first, and a quote that takes today past the cap sends nothing and asks.
    pub async fn generate(
        self: &Arc<Self>,
        maker: Arc<dyn CharacterSheetMaker>,
        over_cap_approved: bool,
    ) -> PaidStart {
        let quote = {
            let state = self.locked();
            if state.is_submitting {
                return PaidStart::NotStarted;
            }
            let Some(quote) = current_quote(&state) else {
                return PaidStart::NotStarted;
            };
            quote
        };
        match (self.spend_status)(ProviderKind::Account).await {
            Ok(status) => {
                if !over_cap_approved
                    && let Some(status) = status
                    && let Some(question) =
                        cap_confirmation(Some(&quote.cost()), status.committed_today, status.cap.as_ref())
                {
                    return PaidStart::NeedsConfirmation(question);
                }
            }
            Err(error) => {
                self.locked().runs.insert(
                    0,
                    CharacterRun {
                        id: Uuid::new_v4(),
                        count: quote.count,
                        state: CharacterRunState::Failed(error.to_string()),
                    },
                );
                self.changed();
                return PaidStart::NotStarted;
            }
        }
        let id = Uuid::new_v4();
        let brief = {
            let mut state = self.locked();
            // While today's spend was read, another click may have sent it, or the form moved on.
            if state.is_submitting || current_quote(&state).as_ref() != Some(&quote) {
                return PaidStart::NotStarted;
            }
            state.runs.retain(|run| run.state != CharacterRunState::Finished);
            state.form.brief.trim().to_owned()
        };
        let charge = ImageCharge::new(
            id,
            OffsetDateTime::now_utc(),
            quote.cost(),
            ProviderKind::Account,
            CHARGE_TITLE,
            quote.count,
            Some(ImageChargePurpose::CharacterSheets),
        );
        if let Err(error) = self.spend.add(charge) {
            self.locked().runs.insert(
                0,
                CharacterRun {
                    id,
                    count: quote.count,
                    state: CharacterRunState::Failed(ImageSpendLog::not_sent(&error)),
                },
            );
            self.changed();
            return PaidStart::NotStarted;
        }
        {
            let mut state = self.locked();
            state.is_submitting = true;
            state.runs.insert(
                0,
                CharacterRun {
                    id,
                    count: quote.count,
                    state: CharacterRunState::Sending,
                },
            );
        }
        self.changed();
        // Not kept with the polls: a sign-out mustn't cut off a submit that may already be paid for.
        let builder = self.clone();
        tokio::spawn(async move {
            builder.submit(quote, id, brief, maker).await;
            builder.submitted.notify_waiters();
        });
        PaidStart::Started
    }

    async fn submit(
        self: &Arc<Self>,
        quote: CharacterQuote,
        id: Uuid,
        brief: String,
        maker: Arc<dyn CharacterSheetMaker>,
    ) {
        let jobs = match maker.generate_characters(quote).await {
            Ok(jobs) => jobs,
            Err(error) => {
                {
                    let mut state = self.locked();
                    state.is_submitting = false;
                    state.round += 1;
                }
                if was_refused(&error) {
                    self.spend.remove(id);
                    self.finish(id, CharacterRunState::Failed(message(&error, COULD_NOT_MAKE)));
                } else {
                    // It may have started and been charged: counted, and never sent again.
                    self.spend.settle(id, None);
                    self.finish(id, CharacterRunState::Failed(MAY_HAVE_STARTED.to_owned()));
                }
                return;
            }
        };
        let date = OffsetDateTime::now_utc();
        let owner = maker.owner();
        self.log.add(
            jobs.iter()
                .map(|job| CharacterSheetRecord::new(&job.request_id, owner.as_deref(), id, date, &brief))
                .collect(),
        );
        {
            let mut state = self.locked();
            state.round += 1;
            state.is_submitting = false;
        }
        self.finish(id, CharacterRunState::Running);
        self.follow(id, maker);
    }

    fn finish(&self, id: Uuid, new: CharacterRunState) {
        if let Some(run) = self.locked().runs.iter_mut().find(|run| run.id == id) {
            run.state = new;
        }
        self.changed();
    }

    /// Waits for the submit already sent to be answered, so the jobs it starts are in the log before a quit.
    pub async fn finish_submit(&self) {
        loop {
            let notified = self.submitted.notified();
            if !self.locked().is_submitting {
                return;
            }
            notified.await;
        }
    }

    // MARK: Following

    /// Follows the sheets of `maker`'s account still running, as after a relaunch, a new sign-in or a poll that
    /// couldn't be read.
    pub fn resume(self: &Arc<Self>, maker: &Arc<dyn CharacterSheetMaker>) {
        let mut requests: Vec<Uuid> = Vec::new();
        for record in self.log.pending(maker.owner().as_deref()) {
            if !requests.contains(&record.request_id) {
                requests.push(record.request_id);
            }
        }
        for request in requests {
            if self.locked().followers.contains_key(&request) {
                continue;
            }
            self.finish(request, CharacterRunState::Running);
            self.follow(request, maker.clone());
        }
    }

    /// Stops polling; the sheets stay pending in the log and are followed again by `resume`.
    pub fn stop_following(&self) {
        let followers: Vec<CancellationToken> =
            self.locked().followers.drain().map(|(_, token)| token).collect();
        for token in followers {
            token.cancel();
        }
        self.changed();
    }

    fn follow(self: &Arc<Self>, request: Uuid, maker: Arc<dyn CharacterSheetMaker>) {
        let token = CancellationToken::new();
        self.locked().followers.insert(request, token.clone());
        let builder = self.clone();
        tokio::spawn(async move { builder.poll(request, maker, token).await });
    }

    fn pending_of(&self, request: Uuid) -> Vec<CharacterSheetRecord> {
        self.log
            .records()
            .into_iter()
            .filter(|record| record.request_id == request && record.is_pending())
            .collect()
    }

    /// Polls until every sheet of the request ended, backing off like the engine. Errors that waiting can fix are
    /// polled through. Any other stops following and leaves the sheets pending and the price counted: a 401 also
    /// asks for the sign-in again, and `resume` follows them again after it.
    async fn poll(
        self: &Arc<Self>,
        request: Uuid,
        maker: Arc<dyn CharacterSheetMaker>,
        token: CancellationToken,
    ) {
        let mut delay = Backoff::polling();
        let mut interruption: Option<PortError> = None;
        while !token.is_cancelled() {
            let pending = self.pending_of(request);
            if pending.is_empty() {
                break;
            }
            let jobs = pending.iter().map(|record| RemoteJob::new(&record.id)).collect();
            let statuses = tokio::select! {
                statuses = maker.character_sheet_statuses(jobs) => statuses,
                () = token.cancelled() => return,
            };
            match statuses {
                Ok(statuses) => {
                    if self.apply(&statuses, &pending) {
                        delay.reset();
                    }
                }
                Err(error) if is_transient(&error) => {}
                Err(PortError::Cancelled) => return,
                Err(error) => {
                    interruption = Some(error);
                    break;
                }
            }
            if self.pending_of(request).is_empty() {
                break;
            }
            tokio::select! {
                () = (self.sleep)(delay.next_wait().unsigned_abs()) => {}
                () = token.cancelled() => return,
            }
        }
        if token.is_cancelled() {
            return;
        }
        self.locked().followers.remove(&request);
        let Some(error) = interruption else {
            return self.end(request);
        };
        if error == PortError::Provider(ProviderError::Unauthorized) {
            let handler = self
                .on_sign_in_expired
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(handler) = handler {
                handler();
            }
        }
        self.finish(
            request,
            CharacterRunState::Interrupted(interrupted(&error, ProviderKind::Account)),
        );
    }

    /// Every sheet of the request ended: charged when one was made, else taken off.
    fn end(&self, request: Uuid) {
        let records: Vec<CharacterSheetRecord> = self
            .log
            .records()
            .into_iter()
            .filter(|record| record.request_id == request)
            .collect();
        if records
            .iter()
            .any(|record| record.state == CharacterSheetState::Completed {})
        {
            self.spend.settle(request, None);
            self.finish(request, CharacterRunState::Finished);
        } else {
            self.spend.remove(request);
            self.finish(request, CharacterRunState::Failed(failure_message(&records)));
        }
    }

    /// Writes down what the poll answered and hands the finished sheets on; returns whether any finished.
    fn apply(&self, statuses: &BTreeMap<String, ImageJobStatus>, pending: &[CharacterSheetRecord]) -> bool {
        let mut finished: Vec<CharacterSheetRecord> = Vec::new();
        for record in pending {
            match statuses.get(&record.id) {
                Some(ImageJobStatus::Completed(urls)) => {
                    self.log.update(
                        &record.id,
                        CharacterSheetState::Completed {},
                        urls.first().cloned(),
                    );
                    if let Some(done) = self.log.records().into_iter().find(|done| done.id == record.id) {
                        finished.push(done);
                    }
                }
                Some(ImageJobStatus::Failed { message }) => self.log.update(
                    &record.id,
                    CharacterSheetState::Failed {
                        message: message.clone(),
                    },
                    None,
                ),
                Some(ImageJobStatus::Rejected(_)) => {
                    self.log
                        .update(&record.id, CharacterSheetState::Rejected {}, None);
                }
                Some(ImageJobStatus::Canceled) => {
                    self.log
                        .update(&record.id, CharacterSheetState::Failed { message: None }, None);
                }
                Some(ImageJobStatus::Pending) | None => {}
            }
        }
        if finished.is_empty() {
            return false;
        }
        let handler = self
            .on_sheets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(handler) = handler {
            handler(finished);
        }
        self.changed();
        true
    }

    // MARK: Form

    /// Changes the form; the quote is read again by whoever shows it.
    pub fn edit_form<T>(&self, edit: impl FnOnce(&mut CharacterForm) -> T) -> T {
        let result = edit(&mut self.locked().form);
        self.changed();
        result
    }

    /// A person or an animal; picking the other kind starts from its first tier.
    pub fn select_kind(&self, kind: CharacterTierKind) {
        let tier = {
            let state = self.locked();
            if kind == state.form.tier.kind() {
                return;
            }
            kind.tiers().first().copied()
        };
        if let Some(tier) = tier {
            self.select_tier(tier);
        }
    }

    /// A new tier drops the traits and the preset, which belong to the old one.
    pub fn select_tier(&self, tier: CharacterTier) {
        {
            let mut state = self.locked();
            if tier == state.form.tier {
                return;
            }
            select_tier(&mut state, tier);
        }
        self.changed();
    }

    /// Picks the trait, or unpicks it. Over the group's limit, the earliest pick goes. Nothing for a group or a trait
    /// the tier doesn't list.
    pub fn toggle(&self, group_id: &str, trait_id: &str) {
        {
            let mut state = self.locked();
            let Some(group) = state
                .trait_groups
                .iter()
                .find(|group| group.id == group_id)
                .cloned()
            else {
                return;
            };
            if !group.traits.iter().any(|item| item.id == trait_id) {
                return;
            }
            let mut picked = state.form.traits.get(group_id).cloned().unwrap_or_default();
            if let Some(index) = picked.iter().position(|id| id == trait_id) {
                picked.remove(index);
            } else {
                picked.push(trait_id.to_owned());
                let max = usize::try_from(group.max_picks).unwrap_or(1);
                if picked.len() > max {
                    picked.drain(..picked.len() - max);
                }
            }
            if picked.is_empty() {
                state.form.traits.remove(group_id);
            } else {
                state.form.traits.insert(group_id.to_owned(), picked);
            }
        }
        self.changed();
    }

    /// Fills the form with what makes `preset` again, and turns Randomize off so it stays that character.
    pub fn apply_preset(&self, preset: &CharacterPreset) {
        let Some(settings) = &preset.settings else {
            return;
        };
        {
            let mut state = self.locked();
            if settings.tier != state.form.tier {
                select_tier(&mut state, settings.tier);
            }
            state.form.traits.clone_from(&settings.traits);
            state.form.brief.clone_from(&settings.brief);
            state.form.seed = settings.seed;
            state.form.randomize = false;
            state.form.preset = Some(preset.clone());
        }
        self.changed();
    }

    /// Keeps the form, without the preset's seed.
    pub fn clear_preset(&self) {
        self.edit_form(|form| {
            form.preset = None;
            form.seed = None;
        });
    }

    /// Adds style references up to `CharacterDesign::MAX_STYLES`; returns how many were left out.
    pub fn add_styles(&self, images: Vec<LocalImage>) -> usize {
        self.edit_form(|form| add_styles(form, images))
    }

    pub fn remove_style(&self, index: usize) {
        self.edit_form(|form| {
            if index < form.styles.len() {
                form.styles.remove(index);
            }
        });
    }
}

fn select_tier(state: &mut BuilderState, tier: CharacterTier) {
    state.form.tier = tier;
    state.form.traits.clear();
    state.form.seed = None;
    state.form.preset = None;
    state.trait_groups = state.traits_by_tier.get(&tier).cloned().unwrap_or_default();
    state.traits_failure = None;
}

fn add_styles(form: &mut CharacterForm, images: Vec<LocalImage>) -> usize {
    let room = CharacterDesign::MAX_STYLES.saturating_sub(form.styles.len());
    let left_out = images.len().saturating_sub(room);
    form.styles.extend(images.into_iter().take(room));
    left_out
}

fn blocker(form: &CharacterForm) -> Option<String> {
    (form.brief.chars().count() > CharacterDesign::MAX_BRIEF).then(|| BRIEF_TOO_LONG.to_owned())
}

fn quote_key(form: &CharacterForm, round: u64) -> String {
    let picked: Vec<String> = form
        .traits
        .iter()
        .map(|(group, traits)| format!("{group}={}", traits.join(",")))
        .collect();
    [
        form.tier.raw_value().to_owned(),
        picked.join(";"),
        form.brief.clone(),
        form.seed.map(|seed| seed.to_string()).unwrap_or_default(),
        form.count.to_string(),
        form.face
            .as_ref()
            .map(|face| face.file_url.to_string())
            .unwrap_or_default(),
        form.styles
            .iter()
            .map(|style| style.file_url.to_string())
            .collect::<Vec<_>>()
            .join(","),
        form.randomize.to_string(),
        round.to_string(),
    ]
    .join("\u{1F}")
}

fn current_quote(state: &BuilderState) -> Option<CharacterQuote> {
    let key = quote_key(&state.form, state.round);
    (state.quoted_key.as_deref() == Some(key.as_str()))
        .then(|| state.quote.clone())
        .flatten()
}

fn failure_message(records: &[CharacterSheetRecord]) -> String {
    records
        .iter()
        .find_map(|record| match &record.state {
            CharacterSheetState::Rejected {} => Some(REFUSED_BY_CHECK.to_owned()),
            CharacterSheetState::Failed {
                message: Some(message),
            } => Some(message.clone()),
            _ => None,
        })
        .unwrap_or_else(|| COULD_NOT_MAKE.to_owned())
}

#[cfg(test)]
#[path = "characters_tests.rs"]
mod tests;
