//! Character sheets an account makes.

use std::collections::BTreeMap;

use async_trait::async_trait;
use rust_decimal::Decimal;
use time::OffsetDateTime;
use url::Url;

use crate::models::Cost;
use crate::ports::PortError;
use crate::provider::{HistoryCursor, ImageJobStatus, PreparedFile, RemoteAsset, RemoteJob};
use crate::support::range::ClosedRange;
use crate::support::string_enum;

/// A provider whose account makes character sheets: a generated image of one character from several sides, which a
/// video job then uses as a reference image. A small service beside `GenerationProvider`, like `ImageProvider`: its
/// jobs never enter the job engine; whoever submits them tracks every job and polls it.
#[async_trait]
pub trait CharacterSheetMaker: Send + Sync {
    /// `GenerationProvider::owner` of the account, which a sheet's media belongs to.
    fn owner(&self) -> Option<String>;
    /// The trait groups of `tier`, each with the options that tier offers.
    async fn character_traits(&self, tier: CharacterTier) -> Result<Vec<CharacterTraitGroup>, PortError>;
    /// One page of ready-made characters.
    async fn character_presets(&self, after: Option<HistoryCursor>)
    -> Result<CharacterPresetPage, PortError>;
    /// The account's sheets, newest first, wherever they were made. Fails when the provider can't list them.
    async fn character_sheets(&self, after: Option<HistoryCursor>) -> Result<CharacterSheetPage, PortError>;
    /// A face or style image of a design.
    async fn upload(&self, file: PreparedFile) -> Result<RemoteAsset, PortError>;
    /// The exact request and its price; free, starts nothing. A design left to chance comes back different each
    /// time.
    async fn quote_characters(&self, design: CharacterDesign) -> Result<CharacterQuote, PortError>;
    /// Starts `quote`'s sheets, one job each, refused when the price went above the quote's. Not idempotent: a
    /// submit that may have reached the provider fails with `ProviderError::OutcomeUnknown` and must never be sent
    /// again.
    async fn generate_characters(&self, quote: CharacterQuote) -> Result<Vec<RemoteJob>, PortError>;
    /// The statuses of `jobs` by request ID; a job left out is still running as far as is known.
    async fn character_sheet_statuses(
        &self,
        jobs: Vec<RemoteJob>,
    ) -> Result<BTreeMap<String, ImageJobStatus>, PortError>;
}

string_enum! {
    /// Who a character is: a person, or an animal of one kind.
    pub enum CharacterTier {
        /// A person.
        Normal = "normal",
        /// An unusual person.
        Freak = "freak",
        /// Anything human.
        Total = "total",
        /// Insects.
        Insects = "insects",
        /// Frogs.
        Frogs = "frogs",
        /// Cats.
        Cats = "cats",
        /// Dogs.
        Dogs = "dogs",
        /// Capybaras.
        Capybaras = "capybaras",
        /// Birds.
        Birds = "birds",
    }
}

string_enum! {
    /// Human or animal.
    pub enum CharacterTierKind {
        /// A person.
        Human = "human",
        /// An animal.
        Animal = "animal",
    }
}

impl CharacterTierKind {
    /// The tiers of this kind, in order.
    pub fn tiers(self) -> Vec<CharacterTier> {
        CharacterTier::ALL
            .iter()
            .copied()
            .filter(|tier| tier.kind() == self)
            .collect()
    }
}

impl CharacterTier {
    /// Human or animal.
    pub fn kind(self) -> CharacterTierKind {
        match self {
            Self::Normal | Self::Freak | Self::Total => CharacterTierKind::Human,
            Self::Insects | Self::Frogs | Self::Cats | Self::Dogs | Self::Capybaras | Self::Birds => {
                CharacterTierKind::Animal
            }
        }
    }
}

/// A group of traits a design picks from: "Hair", "Eyes"…
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterTraitGroup {
    /// The provider's ID.
    pub id: String,
    /// Its title.
    pub title: String,
    /// How many of `traits` one design takes, at least 1.
    pub max_picks: i64,
    /// The options.
    pub traits: Vec<CharacterTrait>,
}

impl CharacterTraitGroup {
    /// A group taking at least one pick.
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        max_picks: i64,
        traits: Vec<CharacterTrait>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            max_picks: max_picks.max(1),
            traits,
        }
    }
}

/// One option of a trait group.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterTrait {
    /// The provider's ID.
    pub id: String,
    /// Its title.
    pub title: String,
}

/// A ready-made character, with the settings that make it again when the provider gives them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterPreset {
    /// The provider's ID.
    pub id: String,
    /// Its name.
    pub name: String,
    /// A picture of it.
    pub preview_url: Option<Url>,
    /// `None` when it can't be made again; `unavailable_reason` says why.
    pub settings: Option<CharacterPresetSettings>,
    /// Why it can't be made again.
    pub unavailable_reason: Option<String>,
}

/// What makes a ready-made character again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterPresetSettings {
    /// Who it is.
    pub tier: CharacterTier,
    /// Trait IDs by group ID.
    pub traits: BTreeMap<String, Vec<String>>,
    /// A description.
    pub brief: String,
    /// The seed.
    pub seed: Option<i64>,
}

/// One page of ready-made characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterPresetPage {
    /// The characters.
    pub items: Vec<CharacterPreset>,
    /// `None` on the last page.
    pub next_cursor: Option<HistoryCursor>,
}

/// What a character is to look like, as the provider is asked to price it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterDesign {
    /// `None` leaves it to chance.
    pub tier: Option<CharacterTier>,
    /// Trait IDs by group ID.
    pub traits: BTreeMap<String, Vec<String>>,
    /// A description.
    pub brief: String,
    /// The seed.
    pub seed: Option<i64>,
    /// Sheets to make, `SHEET_COUNTS`.
    pub count: i64,
    /// The uploaded face to keep.
    pub face: Option<RemoteAsset>,
    /// Uploaded style references, at most `MAX_STYLES`.
    pub styles: Vec<RemoteAsset>,
    /// Fill the traits left unset at random.
    pub randomize: bool,
}

impl CharacterDesign {
    /// How many sheets one request makes.
    pub const SHEET_COUNTS: ClosedRange<i64> = ClosedRange::new(1, 4);
    /// The most style references.
    pub const MAX_STYLES: usize = 3;
    /// The longest brief.
    pub const MAX_BRIEF: usize = 4000;

    /// One sheet of `tier`, everything else left to chance.
    pub fn new(tier: Option<CharacterTier>) -> Self {
        Self {
            tier,
            traits: BTreeMap::new(),
            brief: String::new(),
            seed: None,
            count: 1,
            face: None,
            styles: Vec::new(),
            randomize: true,
        }
    }
}

/// The provider's exact request for a design and its price.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterQuote {
    /// The price of every sheet together, exact: the submit is refused above it.
    pub credits: Decimal,
    /// How many sheets.
    pub count: i64,
    /// What the provider sends to generate, unchanged; only the provider reads it.
    pub request: Vec<u8>,
}

impl CharacterQuote {
    /// The price as a cost.
    pub fn cost(&self) -> Cost {
        Cost::credits(self.credits, false)
    }
}

/// A finished character sheet of the account: an image job.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterSheet {
    /// The job ID, which a job of the same account names instead of uploading the image.
    pub id: String,
    /// The image.
    pub image_url: Url,
    /// A small picture.
    pub thumbnail_url: Option<Url>,
    /// When it was made.
    pub created_at: Option<OffsetDateTime>,
    /// The copy saved on this computer, for a sheet made in the app; the provider's links expire.
    pub file_url: Option<Url>,
}

/// One page of sheets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CharacterSheetPage {
    /// The sheets.
    pub items: Vec<CharacterSheet>,
    /// `None` on the last page.
    pub next_cursor: Option<HistoryCursor>,
}
