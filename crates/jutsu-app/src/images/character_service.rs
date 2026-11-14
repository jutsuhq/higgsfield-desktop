//! Create Character as the window sees it (`CreateCharacterSheet.swift`, `CreateCharacterForm.swift`,
//! `AppEnvironment+Characters.swift`): the form, the price on Generate, the requests, and each finished sheet saved to
//! Library › Images as it arrives.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use futures::FutureExt as _;
use jutsu_core::{
    CharacterDesign, CharacterPreset, CharacterSheetMaker, CharacterTier, CharacterTierKind, LocalImage,
    MaterialRemote, ProviderKind,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    ImageGenerateOutcome, ImageNoticeView, ImageRequestRow, ImageService, RequestStatus, local_path,
};
use crate::create::paged_feed::{Fetch, PagedFeed};
use crate::images::characters::{
    CharacterBuilder, CharacterForm, CharacterRun, CharacterRunState, CharacterSheetLog,
    CharacterSheetRecord, kind_title, quote_price, quote_price_text, tier_title,
};
use crate::images::generator::{GeneratedImage, PaidStart, UploadFn};

/// What a preset's tile, an option or a sheet title says.
const SHEET_TITLE: &str = "Character sheet";

/// One choice of a segmented control or a menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterChoice {
    pub value: String,
    pub title: String,
}

/// A ready-made character: a click fills the form with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterPresetTile {
    pub id: String,
    pub name: String,
    pub preview_url: Option<String>,
    /// Lime-rimmed: it fills the form.
    pub is_picked: bool,
    /// Off for a preset that can't be made again.
    pub enabled: bool,
    /// Why it can't be made again, else its name.
    pub hint: String,
}

/// The Presets row; shown while it has presets or more to load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterPresetsView {
    pub tiles: Vec<CharacterPresetTile>,
    pub has_more: bool,
    pub failure: Option<String>,
    /// Clear Preset next to the title.
    pub shows_clear: bool,
}

/// A trait to pick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TraitChip {
    pub id: String,
    pub title: String,
    pub is_picked: bool,
}

/// How a trait group's line reads its choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TraitChoiceKind {
    Picked,
    Random,
    Any,
}

/// A trait group on one line: its name, what's picked or "Any" ("Random" with Randomize on), and its limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TraitGroupView {
    pub id: String,
    pub title: String,
    /// The picks in the order made, "Random" or "Any".
    pub choice: String,
    pub choice_kind: TraitChoiceKind,
    /// "up to 2"; `None` for a group of one pick.
    pub limit: Option<String>,
    pub accessibility_value: String,
    pub traits: Vec<TraitChip>,
}

/// The Traits section; `None` with no groups and no failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterTraitsView {
    /// "3 picked · the rest random", "Random", "3 picked", or `None`.
    pub detail: Option<String>,
    pub failure: Option<String>,
    pub groups: Vec<TraitGroupView>,
}

/// Generate with the quote on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterGenerateView {
    pub is_busy: bool,
    pub enabled: bool,
    /// "4.5 cr"; `None` without a quote of the form as it stands.
    pub price: Option<String>,
    /// Where the price comes from, or that it's being asked for.
    pub hint: String,
}

/// Create Character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSnapshot {
    /// The account makes character sheets.
    pub connected: bool,
    pub presets: Option<CharacterPresetsView>,
    pub kind: String,
    pub kinds: Vec<CharacterChoice>,
    pub tier: String,
    /// The Look menu; empty when the kind has one tier.
    pub tiers: Vec<CharacterChoice>,
    pub brief: String,
    /// "12 / 4000"; `None` while the description is empty.
    pub brief_detail: Option<String>,
    /// The face's file.
    pub face: Option<String>,
    pub styles: Vec<String>,
    pub can_add_style: bool,
    pub randomize: bool,
    pub count: i32,
    pub counts: Vec<i32>,
    pub traits: Option<CharacterTraitsView>,
    /// The requests sent from here, oldest first.
    pub runs: Vec<ImageRequestRow>,
    pub generate: CharacterGenerateView,
    pub notice: Option<ImageNoticeView>,
    /// "Wait for Higgsfield" while busy, else "Close".
    pub close_hint: String,
}

/// What the window can change on the form.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CharacterEdit {
    SelectKind {
        value: String,
    },
    SelectTier {
        value: String,
    },
    SetBrief {
        text: String,
    },
    /// The face; `None` removes it.
    SetFace {
        path: Option<String>,
    },
    /// Files dropped on the Face row: the first image becomes the face.
    DropFace {
        paths: Vec<String>,
    },
    /// Style references, up to three; images past the room are left out.
    AddStyles {
        paths: Vec<String>,
    },
    RemoveStyle {
        index: u32,
    },
    SetRandomize {
        on: bool,
    },
    SetCount {
        count: i32,
    },
    ToggleTrait {
        group: String,
        id: String,
    },
    ApplyPreset {
        id: String,
    },
    ClearPreset,
}

/// What Generate did.
pub type CharacterGenerateOutcome = ImageGenerateOutcome;

/// The presets row; hidden until a page has some.
fn presets_view(feed: &PagedFeed<CharacterPreset>, form: &CharacterForm) -> Option<CharacterPresetsView> {
    let view = feed.view();
    let picked = form.preset.as_ref().map(|picked| picked.id.as_str());
    (!view.items.is_empty() || view.has_more).then(|| CharacterPresetsView {
        tiles: view
            .items
            .iter()
            .map(|preset| preset_tile(preset, picked))
            .collect(),
        has_more: view.has_more,
        failure: view.failure.map(|failure| failure.title),
        shows_clear: form.preset.is_some(),
    })
}

fn preset_tile(preset: &CharacterPreset, picked: Option<&str>) -> CharacterPresetTile {
    CharacterPresetTile {
        id: preset.id.clone(),
        name: preset.name.clone(),
        preview_url: preset.preview_url.as_ref().map(Url::to_string),
        is_picked: picked == Some(preset.id.as_str()),
        enabled: preset.settings.is_some(),
        hint: preset
            .unavailable_reason
            .clone()
            .unwrap_or_else(|| preset.name.clone()),
    }
}

/// The images among `paths`, as references.
fn images(paths: &[String]) -> Vec<LocalImage> {
    paths
        .iter()
        .map(Path::new)
        .filter(|path| jutsu_media::frames::is_image(path))
        .filter_map(|path| Url::from_file_path(path).ok())
        .map(LocalImage::new)
        .collect()
}

fn traits_detail(form: &CharacterForm) -> Option<String> {
    let picked: usize = form.traits.values().map(Vec::len).sum();
    match (picked, form.randomize) {
        (0, true) => Some("Random".to_owned()),
        (0, false) => None,
        (_, true) => Some(format!("{picked} picked · the rest random")),
        (_, false) => Some(format!("{picked} picked")),
    }
}

fn group_view(group: &jutsu_core::CharacterTraitGroup, form: &CharacterForm) -> TraitGroupView {
    let ids = form.traits.get(&group.id).cloned().unwrap_or_default();
    let picked: Vec<String> = ids
        .iter()
        .filter_map(|id| group.traits.iter().find(|item| &item.id == id))
        .map(|item| item.title.clone())
        .collect();
    let (choice, choice_kind) = if !picked.is_empty() {
        (picked.join(", "), TraitChoiceKind::Picked)
    } else if form.randomize {
        ("Random".to_owned(), TraitChoiceKind::Random)
    } else {
        ("Any".to_owned(), TraitChoiceKind::Any)
    };
    let accessibility_value = if group.max_picks > 1 {
        format!("{choice}, up to {}", group.max_picks)
    } else {
        choice.clone()
    };
    TraitGroupView {
        id: group.id.clone(),
        title: group.title.clone(),
        choice,
        choice_kind,
        limit: (group.max_picks > 1).then(|| format!("up to {}", group.max_picks)),
        accessibility_value,
        traits: group
            .traits
            .iter()
            .map(|item| TraitChip {
                id: item.id.clone(),
                title: item.title.clone(),
                is_picked: ids.contains(&item.id),
            })
            .collect(),
    }
}

fn run_row(run: &CharacterRun) -> ImageRequestRow {
    let (status, detail) = match &run.state {
        CharacterRunState::Sending => (RequestStatus::Working, "Sending…".to_owned()),
        CharacterRunState::Running => (RequestStatus::Working, "Being made".to_owned()),
        CharacterRunState::Finished => (
            RequestStatus::Finished,
            "Done · in Characters and Library › Images".to_owned(),
        ),
        CharacterRunState::Failed(message) | CharacterRunState::Interrupted(message) => {
            (RequestStatus::Failed, message.clone())
        }
    };
    ImageRequestRow {
        id: run.id.to_string(),
        title: format!("{} character sheets", run.count),
        status,
        detail: Some(detail),
        can_cancel: false,
        subtitle: None,
        review: None,
    }
}

fn ipc(count: i64) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

impl ImageService {
    pub fn characters(&self) -> &Arc<CharacterBuilder> {
        &self.characters
    }

    pub fn character_log(&self) -> &Arc<CharacterSheetLog> {
        self.characters.log()
    }

    /// The connection's character sheet service; `None` without an account.
    pub fn character_maker(&self) -> Option<Arc<dyn CharacterSheetMaker>> {
        self.locked().characters.maker.clone()
    }

    /// Saves each finished sheet into Library › Images as it arrives, and remembers where.
    pub(super) fn wire_characters(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        self.characters
            .set_on_sheets(Arc::new(move |records: Vec<CharacterSheetRecord>| {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                let mut jobs = HashMap::new();
                let sheets: Vec<GeneratedImage> = records
                    .iter()
                    .filter_map(|record| {
                        let url = record.image_url.clone()?;
                        let prompt = if record.brief.is_empty() {
                            SHEET_TITLE
                        } else {
                            record.brief.as_str()
                        };
                        let mut image = GeneratedImage::new(url, prompt, SHEET_TITLE, None);
                        image.remote = Some(MaterialRemote {
                            provider: ProviderKind::Account,
                            owner: record.owner.clone(),
                            id: record.id.clone(),
                            output: 0,
                        });
                        image.variant_group_id = Some(record.request_id);
                        jobs.insert(image.id, record.id.clone());
                        Some(image)
                    })
                    .collect();
                let log = Arc::downgrade(service.characters.log());
                service.saver.save(
                    sheets,
                    HashMap::new(),
                    Arc::new(move |image: &GeneratedImage, file| {
                        if let (Some(log), Some(job), Ok(url)) =
                            (log.upgrade(), jobs.get(&image.id), Url::from_file_path(&file))
                        {
                            log.set_file(url, job);
                        }
                    }),
                );
            }));
        let connection = Arc::downgrade(&self.deps.connection);
        self.characters.set_on_sign_in_expired(Arc::new(move || {
            if let Some(connection) = connection.upgrade() {
                connection.mark_needs_attention();
            }
        }));
        let mut changes = self.characters.subscribe();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                service.notify();
            }
        });
    }

    /// A new connection: the old one's sheets stop being polled, and the new account's sheets still running are
    /// followed again.
    pub fn set_character_maker(self: &Arc<Self>, maker: Option<Arc<dyn CharacterSheetMaker>>) {
        self.characters.reset();
        {
            let mut state = self.locked();
            let characters = &mut state.characters;
            characters.generation += 1;
            characters.maker.clone_from(&maker);
            characters.presets = maker.clone().map(|maker| {
                let fetch: Fetch<CharacterPreset> = Arc::new(move |cursor| {
                    let maker = maker.clone();
                    async move {
                        let page = maker.character_presets(cursor).await?;
                        Ok((page.items, page.next_cursor))
                    }
                    .boxed()
                });
                PagedFeed::new("Can't load the presets", fetch)
            });
            characters.traits_for = None;
            characters.quoted = None;
        }
        if let Some(maker) = maker {
            self.characters.resume(&maker);
        }
        self.notify();
    }

    /// Prepares a face or style image like a reference of a job, then uploads it to `maker`.
    fn character_uploader(&self, maker: &Arc<dyn CharacterSheetMaker>) -> UploadFn {
        let (media, maker) = (self.deps.media.clone(), maker.clone());
        Arc::new(move |image: LocalImage| {
            let (media, maker) = (media.clone(), maker.clone());
            Box::pin(async move { maker.upload(media.prepare_image(&image).await?).await })
        })
    }

    /// Create Character as it stands; reads the traits and the presets when they're due, and the price a moment
    /// after the form last changed.
    pub fn character_snapshot(self: &Arc<Self>) -> CharacterSnapshot {
        self.ensure_character_loads();
        let builder = &self.characters;
        let form = builder.form();
        let maker = self.character_maker();
        let (presets, is_starting) = {
            let state = self.locked();
            (state.characters.presets.clone(), state.characters.is_starting)
        };
        let is_busy = is_starting || builder.is_submitting();
        let quote = builder.current_quote();
        let blocker = builder.blocker();
        let quote_failure = builder.quote_failure();
        let groups = builder.trait_groups();
        let traits_failure = builder.traits_failure();
        let notice = if maker.is_none() {
            Some(ImageNoticeView {
                text: "Connect your Higgsfield account to create characters.".to_owned(),
                is_quiet: false,
            })
        } else if let Some(blocker) = &blocker {
            Some(ImageNoticeView {
                text: blocker.clone(),
                is_quiet: false,
            })
        } else {
            quote_failure.map(|text| ImageNoticeView { text, is_quiet: true })
        };
        let hint = match &quote {
            Some(quote) => quote_price_text(quote),
            None if builder.is_quoting() && maker.is_some() => {
                "Getting the price from Higgsfield…".to_owned()
            }
            None => String::new(),
        };
        let kind = form.tier.kind();
        let tiers = kind.tiers();
        CharacterSnapshot {
            connected: maker.is_some(),
            presets: presets.and_then(|feed| presets_view(&feed, &form)),
            kind: kind.raw_value().to_owned(),
            kinds: CharacterTierKind::ALL
                .iter()
                .map(|kind| CharacterChoice {
                    value: kind.raw_value().to_owned(),
                    title: kind_title(*kind).to_owned(),
                })
                .collect(),
            tier: form.tier.raw_value().to_owned(),
            tiers: if tiers.len() > 1 {
                tiers
                    .iter()
                    .map(|tier| CharacterChoice {
                        value: tier.raw_value().to_owned(),
                        title: tier_title(*tier).to_owned(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            brief_detail: (!form.brief.is_empty()).then(|| format!("{} / 4000", form.brief.chars().count())),
            brief: form.brief.clone(),
            face: form.face.as_ref().and_then(|face| local_path(&face.file_url)),
            styles: form
                .styles
                .iter()
                .filter_map(|style| local_path(&style.file_url))
                .collect(),
            can_add_style: form.styles.len() < CharacterDesign::MAX_STYLES,
            randomize: form.randomize,
            count: ipc(form.count),
            counts: (CharacterDesign::SHEET_COUNTS.lower()..=CharacterDesign::SHEET_COUNTS.upper())
                .map(ipc)
                .collect(),
            traits: (traits_failure.is_some() || !groups.is_empty()).then(|| CharacterTraitsView {
                detail: traits_detail(&form),
                failure: traits_failure,
                groups: groups.iter().map(|group| group_view(group, &form)).collect(),
            }),
            runs: builder.runs().iter().rev().map(run_row).collect(),
            generate: CharacterGenerateView {
                is_busy,
                enabled: !is_busy && quote.is_some() && maker.is_some(),
                price: quote.as_ref().map(quote_price),
                hint,
            },
            notice,
            close_hint: if is_busy { "Wait for Higgsfield" } else { "Close" }.to_owned(),
        }
    }

    /// The traits of the tier once per tier and connection, the first page of presets, and the price of the form a
    /// moment after it last changed.
    fn ensure_character_loads(self: &Arc<Self>) {
        let Some(maker) = self.character_maker() else {
            return;
        };
        let tier = self.characters.form().tier;
        let (load_traits, presets, generation) = {
            let mut state = self.locked();
            let characters = &mut state.characters;
            let due = characters.traits_for != Some((characters.generation, tier));
            if due {
                characters.traits_for = Some((characters.generation, tier));
            }
            (due, characters.presets.clone(), characters.generation)
        };
        if load_traits {
            let (builder, maker) = (self.characters.clone(), maker.clone());
            tokio::spawn(async move { builder.load_traits(maker.as_ref()).await });
        }
        if let Some(feed) = presets
            && !feed.view().has_loaded
            && !feed.view().is_loading
        {
            let weak = Arc::downgrade(self);
            tokio::spawn(async move {
                feed.load_if_needed().await;
                if let Some(service) = weak.upgrade() {
                    service.notify();
                }
            });
        }
        self.schedule_character_quote(&maker, generation);
    }

    fn schedule_character_quote(self: &Arc<Self>, maker: &Arc<dyn CharacterSheetMaker>, generation: u64) {
        if self.characters.blocker().is_some() || self.characters.current_quote().is_some() {
            return;
        }
        let key = self.characters.quote_key();
        {
            let mut state = self.locked();
            let quoted = (generation, key.clone());
            if state.characters.quoted.as_ref() == Some(&quoted) {
                return;
            }
            state.characters.quoted = Some(quoted);
        }
        let weak = Arc::downgrade(self);
        let maker = maker.clone();
        tokio::spawn(async move {
            tokio::time::sleep(super::QUOTE_DEBOUNCE).await;
            let Some(service) = weak.upgrade() else {
                return;
            };
            // Changed meanwhile: the newer form is quoted instead.
            if service.characters.quote_key() != key {
                return;
            }
            let upload = service.character_uploader(&maker);
            service.characters.refresh_quote(maker.as_ref(), &upload).await;
        });
    }

    /// One change to the form; answers with the sheet after it.
    pub fn edit_character(self: &Arc<Self>, edit: CharacterEdit) -> CharacterSnapshot {
        let builder = &self.characters;
        match edit {
            CharacterEdit::SelectKind { value } => {
                if let Some(kind) = CharacterTierKind::from_raw(&value) {
                    builder.select_kind(kind);
                }
            }
            CharacterEdit::SelectTier { value } => {
                if let Some(tier) = CharacterTier::from_raw(&value) {
                    builder.select_tier(tier);
                }
            }
            CharacterEdit::SetBrief { text } => builder.edit_form(|form| form.brief = text),
            CharacterEdit::SetFace { path } => builder.edit_form(|form| {
                form.face = path
                    .and_then(|path| Url::from_file_path(Path::new(&path)).ok())
                    .map(LocalImage::new);
            }),
            CharacterEdit::DropFace { paths } => {
                if let Some(face) = images(&paths).into_iter().next() {
                    builder.edit_form(|form| form.face = Some(face));
                }
            }
            CharacterEdit::AddStyles { paths } => {
                builder.add_styles(images(&paths));
            }
            CharacterEdit::RemoveStyle { index } => builder.remove_style(index as usize),
            CharacterEdit::SetRandomize { on } => builder.edit_form(|form| form.randomize = on),
            CharacterEdit::SetCount { count } => {
                let count = i64::from(count);
                if CharacterDesign::SHEET_COUNTS.contains(count) {
                    builder.edit_form(|form| form.count = count);
                }
            }
            CharacterEdit::ToggleTrait { group, id } => builder.toggle(&group, &id),
            CharacterEdit::ApplyPreset { id } => {
                let preset = self
                    .locked()
                    .characters
                    .presets
                    .as_ref()
                    .and_then(|feed| feed.view().items.into_iter().find(|preset| preset.id == id));
                if let Some(preset) = preset {
                    builder.apply_preset(&preset);
                }
            }
            CharacterEdit::ClearPreset => builder.clear_preset(),
        }
        self.character_snapshot()
    }

    /// The next page of presets.
    pub async fn load_more_character_presets(self: &Arc<Self>) {
        let feed = self.locked().characters.presets.clone();
        if let Some(feed) = feed {
            feed.load_more().await;
        }
        self.notify();
    }

    /// Generate: asks first when the price takes today past the cap; call again `approved`.
    pub async fn generate_characters(self: &Arc<Self>, approved: bool) -> CharacterGenerateOutcome {
        let Some(maker) = self.character_maker() else {
            return ImageGenerateOutcome::NotStarted;
        };
        {
            let mut state = self.locked();
            if state.characters.is_starting || self.characters.is_submitting() {
                return ImageGenerateOutcome::NotStarted;
            }
            state.characters.is_starting = true;
        }
        self.notify();
        let started = self.characters.generate(maker, approved).await;
        self.locked().characters.is_starting = false;
        self.notify();
        match started {
            PaidStart::Started => ImageGenerateOutcome::Started,
            PaidStart::NeedsConfirmation(confirmation) => ImageGenerateOutcome::Confirm { confirmation },
            PaidStart::NotStarted => ImageGenerateOutcome::NotStarted,
        }
    }
}

/// What Create Character holds between calls.
#[derive(Default)]
pub(super) struct CharacterState {
    pub(super) maker: Option<Arc<dyn CharacterSheetMaker>>,
    pub(super) presets: Option<PagedFeed<CharacterPreset>>,
    /// Goes up with each connection.
    pub(super) generation: u64,
    /// The connection and tier whose traits were asked for.
    pub(super) traits_for: Option<(u64, CharacterTier)>,
    /// The connection and form last quoted.
    pub(super) quoted: Option<(u64, String)>,
    pub(super) is_starting: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trait_group_reads_its_picks_or_random_or_any() {
        let group = jutsu_core::CharacterTraitGroup::new(
            "hair",
            "Hair",
            2,
            vec![
                jutsu_core::CharacterTrait {
                    id: "curly".to_owned(),
                    title: "Curly".to_owned(),
                },
                jutsu_core::CharacterTrait {
                    id: "long".to_owned(),
                    title: "Long".to_owned(),
                },
            ],
        );
        let mut form = CharacterForm::default();
        let random = group_view(&group, &form);
        assert_eq!(random.choice, "Random");
        assert_eq!(random.limit.as_deref(), Some("up to 2"));
        assert_eq!(random.accessibility_value, "Random, up to 2");
        assert_eq!(traits_detail(&form).as_deref(), Some("Random"));
        form.randomize = false;
        assert_eq!(group_view(&group, &form).choice, "Any");
        assert_eq!(traits_detail(&form), None);
        form.traits
            .insert("hair".to_owned(), vec!["long".to_owned(), "curly".to_owned()]);
        let picked = group_view(&group, &form);
        assert_eq!(picked.choice, "Long, Curly");
        assert_eq!(picked.choice_kind, TraitChoiceKind::Picked);
        assert_eq!(traits_detail(&form).as_deref(), Some("2 picked"));
        form.randomize = true;
        assert_eq!(
            traits_detail(&form).as_deref(),
            Some("2 picked · the rest random")
        );
    }
}
