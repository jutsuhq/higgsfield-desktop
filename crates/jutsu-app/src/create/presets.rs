//! The restyle styles (`PresetCatalog.swift`, `PresetShelf.swift`): the catalog cached in `catalog.json` and
//! replaced by the provider's list on every refresh, and the gallery's sections with the favorite and recent styles
//! on top of it. Favorites and recents are stored as snapshots, so a style missing from a refreshed catalog keeps
//! its name and can still be unstarred.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};

use jutsu_core::{PortError, ProviderKind, StylePreset};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::connection::UserMessage;

/// The cached catalog's file name in `FileLayout::preset_cache_folder`.
pub const CACHE_FILE_NAME: &str = "catalog.json";
/// How many styles Recent keeps.
pub const RECENT_LIMIT: usize = 5;
/// A selected style the app knows nothing about.
pub const UNKNOWN_PRESET_NAME: &str = "Unavailable style";

/// One tile of the style gallery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetTileItem {
    pub id: String,
    pub name: String,
    /// `None` for a selected style the app knows nothing about.
    pub preview_url: Option<String>,
    /// Played on hover; `None` when the style has no example video.
    pub preview_video_url: Option<String>,
    pub is_favorite: bool,
    /// `false` when the catalog no longer has it: shown dimmed and can't be picked.
    pub is_available: bool,
}

/// Which group of the gallery a section is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PresetSectionKind {
    /// The form's style when it is in no other section, so it isn't hidden.
    Selected,
    Favorites,
    Recent,
    All,
}

impl PresetSectionKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Selected => "Selected",
            Self::Favorites => "Favorites",
            Self::Recent => "Recent",
            Self::All => "All styles",
        }
    }
}

/// A titled group of the style gallery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetGallerySection {
    pub kind: PresetSectionKind,
    pub title: String,
    pub items: Vec<PresetTileItem>,
}

fn distinct(presets: &[StylePreset]) -> Vec<&StylePreset> {
    let mut seen = HashSet::new();
    presets.iter().filter(|preset| seen.insert(preset.id)).collect()
}

/// Selected (only when hidden elsewhere), favorites, recent, then the whole catalog; empty sections are left out.
/// Every section is filtered by `query`. A style is available only if `catalog` has it. Several selections, as a
/// restyle batch has, keep their order in Selected.
pub fn sections(
    catalog: &[StylePreset],
    favorites: &[StylePreset],
    recents: &[StylePreset],
    selections: &[Uuid],
    query: &str,
) -> Vec<PresetGallerySection> {
    let mut current: HashMap<Uuid, &StylePreset> = HashMap::new();
    for preset in catalog {
        current.entry(preset.id).or_insert(preset);
    }
    let favorite_ids: HashSet<Uuid> = favorites.iter().map(|preset| preset.id).collect();
    let item = |snapshot: &StylePreset| {
        let preset = current.get(&snapshot.id).copied().unwrap_or(snapshot);
        PresetTileItem {
            id: jutsu_core::support::uuid_string(preset.id),
            name: preset.name.clone(),
            preview_url: Some(preset.preview_url.to_string()),
            preview_video_url: preset.preview_video_url.as_ref().map(ToString::to_string),
            is_favorite: favorite_ids.contains(&preset.id),
            is_available: current.contains_key(&preset.id),
        }
    };
    let favorite_items: Vec<PresetTileItem> = distinct(favorites).into_iter().map(item).collect();
    let recent_items: Vec<PresetTileItem> = distinct(recents)
        .into_iter()
        .take(RECENT_LIMIT)
        .map(item)
        .collect();
    let all_items: Vec<PresetTileItem> = distinct(catalog).into_iter().map(item).collect();

    let mut shown: HashSet<String> = favorite_items
        .iter()
        .chain(&recent_items)
        .chain(&all_items)
        .map(|item| item.id.clone())
        .collect();
    let selected_items: Vec<PresetTileItem> = selections
        .iter()
        .map(|id| jutsu_core::support::uuid_string(*id))
        .filter(|id| shown.insert(id.clone()))
        .map(|id| PresetTileItem {
            id,
            name: UNKNOWN_PRESET_NAME.to_owned(),
            preview_url: None,
            preview_video_url: None,
            is_favorite: false,
            is_available: false,
        })
        .collect();

    [
        (PresetSectionKind::Selected, selected_items),
        (PresetSectionKind::Favorites, favorite_items),
        (PresetSectionKind::Recent, recent_items),
        (PresetSectionKind::All, all_items),
    ]
    .into_iter()
    .filter_map(|(kind, items)| {
        let matching: Vec<PresetTileItem> = items
            .into_iter()
            .filter(|item| matches(&item.name, query))
            .collect();
        (!matching.is_empty()).then(|| PresetGallerySection {
            kind,
            title: kind.title().to_owned(),
            items: matching,
        })
    })
    .collect()
}

/// Whether `name` contains the trimmed query, ignoring case and accents; an empty query matches everything.
pub fn matches(name: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || crate::prompts::folded(name).contains(&crate::prompts::folded(query))
}

/// Unstars a favorite, or stars a catalog style as the newest favorite. Unknown IDs change nothing.
pub fn toggled_favorite(id: Uuid, favorites: &[StylePreset], catalog: &[StylePreset]) -> Vec<StylePreset> {
    if favorites.iter().any(|preset| preset.id == id) {
        return favorites
            .iter()
            .filter(|preset| preset.id != id)
            .cloned()
            .collect();
    }
    let Some(preset) = catalog.iter().find(|preset| preset.id == id) else {
        return favorites.to_vec();
    };
    std::iter::once(preset.clone())
        .chain(favorites.iter().cloned())
        .collect()
}

/// Moves the style `id` to the front of `recents`, keeping `RECENT_LIMIT` distinct styles. The snapshot comes from
/// the catalog, else from the recents or favorites; with none, nothing changes.
pub fn recording_use(
    id: Uuid,
    recents: &[StylePreset],
    catalog: &[StylePreset],
    favorites: &[StylePreset],
) -> Vec<StylePreset> {
    let Some(preset) = catalog
        .iter()
        .chain(recents)
        .chain(favorites)
        .find(|preset| preset.id == id)
    else {
        return recents.to_vec();
    };
    std::iter::once(preset.clone())
        .chain(recents.iter().filter(|preset| preset.id != id).cloned())
        .take(RECENT_LIMIT)
        .collect()
}

/// The stored list; empty for missing or unreadable data.
pub fn decode(value: Option<&Value>) -> Vec<StylePreset> {
    value
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

pub fn encode(presets: &[StylePreset]) -> Value {
    serde_json::to_value(presets).unwrap_or(Value::Array(Vec::new()))
}

/// Restyle's style gallery (`PresetSection`): one style, or several for a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PresetsView {
    /// "Style", or "Styles" in a batch.
    pub title: String,
    /// "24 presets", or "3 / 12" in a batch; `None` before the catalog is read.
    pub detail: Option<String>,
    /// Why another style can't be added to the batch.
    pub notice: Option<String>,
    /// The catalog has been read: until then a spinner stands for the gallery and Refresh is off.
    pub is_loaded: bool,
    pub is_refreshing: bool,
    pub query: String,
    /// "Can't reach Higgsfield. Showing the saved styles."
    pub failure: Option<String>,
    /// Section titles show unless the whole catalog is the only section.
    pub shows_titles: bool,
    pub sections: Vec<PresetGallerySection>,
    /// The picked styles in order; one outside a batch.
    pub selection: Vec<String>,
}

impl PresetsView {
    /// The gallery of `catalog` (`None` before it is read) for `selection`, with the batch's count and notice.
    pub fn new(
        catalog: Option<&PresetCatalog>,
        favorites: &[StylePreset],
        recents: &[StylePreset],
        selection: &[Uuid],
        batch: Option<(String, Option<String>)>,
    ) -> Self {
        let is_batch = batch.is_some();
        let (count, notice) = batch.unzip();
        let sections = catalog.map_or_else(Vec::new, |catalog| {
            sections(&catalog.presets, favorites, recents, selection, &catalog.query)
        });
        Self {
            title: if is_batch { "Styles" } else { "Style" }.to_owned(),
            detail: count.or_else(|| catalog.map(|catalog| format!("{} presets", catalog.presets.len()))),
            notice: notice.flatten(),
            is_loaded: catalog.is_some(),
            is_refreshing: catalog.is_none_or(|catalog| catalog.is_refreshing),
            query: catalog.map(|catalog| catalog.query.clone()).unwrap_or_default(),
            failure: catalog
                .and_then(|catalog| catalog.refresh_failure.as_ref())
                .map(|failure| format!("{}. Showing the saved styles.", failure.title)),
            shows_titles: sections
                .iter()
                .map(|section| section.kind)
                .ne([PresetSectionKind::All]),
            sections,
            selection: selection
                .iter()
                .map(|id| jutsu_core::support::uuid_string(*id))
                .collect(),
        }
    }
}

/// The restyle styles: the cached catalog first, replaced by the provider's list on every refresh.
#[derive(Debug, Clone)]
pub struct PresetCatalog {
    pub presets: Vec<StylePreset>,
    pub is_refreshing: bool,
    /// The last refresh failed; the cached list is still shown.
    pub refresh_failure: Option<UserMessage>,
    pub query: String,
    cache_folder: PathBuf,
}

impl PresetCatalog {
    pub fn new(cache_folder: PathBuf) -> Self {
        Self {
            presets: Vec::new(),
            is_refreshing: false,
            refresh_failure: None,
            query: String::new(),
            cache_folder,
        }
    }

    /// Styles whose name contains the query, ignoring case and accents.
    pub fn filtered(&self) -> Vec<&StylePreset> {
        self.presets
            .iter()
            .filter(|preset| matches(&preset.name, &self.query))
            .collect()
    }

    pub fn preset(&self, id: Option<Uuid>) -> Option<&StylePreset> {
        let id = id?;
        self.presets.iter().find(|preset| preset.id == id)
    }

    /// Shows the cached catalog until a refresh replaces it. A missing or unreadable cache is ignored, and so is the
    /// cache once styles are shown.
    pub fn load_cache(&mut self) {
        if !self.presets.is_empty() {
            return;
        }
        if let Some(cached) = cached_presets(&self.cache_folder) {
            self.presets = cached;
        }
    }

    /// Starts a refresh; `false` while one is running, which is then left alone.
    pub fn begin_refresh(&mut self) -> bool {
        if self.is_refreshing {
            return false;
        }
        self.is_refreshing = true;
        true
    }

    /// Ends a refresh: a fresh list is shown and cached; a failure keeps the list and says why.
    pub fn finish_refresh(
        &mut self,
        fetched: Result<Vec<StylePreset>, PortError>,
        kind: Option<ProviderKind>,
    ) {
        self.is_refreshing = false;
        match fetched {
            Ok(fresh) => {
                self.refresh_failure = None;
                let _ = std::fs::create_dir_all(&self.cache_folder);
                if let Ok(data) = serde_json::to_vec(&fresh) {
                    let file = self.cache_folder.join(CACHE_FILE_NAME);
                    let partial = self.cache_folder.join(format!("{CACHE_FILE_NAME}.partial"));
                    if std::fs::write(&partial, data).is_ok() {
                        let _ = std::fs::rename(&partial, &file);
                    }
                }
                self.presets = fresh;
            }
            Err(error) => {
                self.refresh_failure = UserMessage::from_error(&error, kind)
                    .or_else(|| Some(UserMessage::failure(error.to_string())));
            }
        }
    }

    /// Fetches the catalog and caches it. Ignored while a refresh is running.
    pub async fn refresh<F>(&mut self, fetch: F, kind: Option<ProviderKind>)
    where
        F: Future<Output = Result<Vec<StylePreset>, PortError>>,
    {
        if !self.begin_refresh() {
            return;
        }
        let fetched = fetch.await;
        self.finish_refresh(fetched, kind);
    }
}

/// Every style of the cached catalog; `None` when nothing is cached or the file can't be read.
pub fn cached_presets(cache_folder: &Path) -> Option<Vec<StylePreset>> {
    let data = std::fs::read(cache_folder.join(CACHE_FILE_NAME)).ok()?;
    serde_json::from_slice(&data).ok()
}

/// A style of the cached catalog; `None` when it isn't cached.
pub fn cached_preset(id: Uuid, cache_folder: &Path) -> Option<StylePreset> {
    cached_presets(cache_folder)?
        .into_iter()
        .find(|preset| preset.id == id)
}

/// A style's name from the cached catalog; `None` when it isn't cached.
pub fn cached_name(id: Uuid, cache_folder: &Path) -> Option<String> {
    cached_preset(id, cache_folder).map(|preset| preset.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use PresetSectionKind::{All, Favorites, Recent};
    use jutsu_core::ProviderError;
    use url::Url;

    fn preset(name: &str, index: u8) -> StylePreset {
        let mut bytes = [0_u8; 16];
        bytes[0] = index;
        StylePreset {
            id: Uuid::from_bytes(bytes),
            name: name.to_owned(),
            preview_url: Url::parse(&format!("https://example.com/{index}.webp")).unwrap(),
            preview_video_url: None,
        }
    }

    struct Styles {
        clay: StylePreset,
        anime: StylePreset,
        noir: StylePreset,
        pixel: StylePreset,
    }

    fn styles() -> Styles {
        Styles {
            clay: preset("Claymation", 1),
            anime: preset("Cel-Shaded CG Anime", 2),
            noir: preset("Film Noir", 3),
            pixel: preset("Pixel Art", 4),
        }
    }

    fn of(
        catalog: &[StylePreset],
        favorites: &[StylePreset],
        recents: &[StylePreset],
        selection: Option<Uuid>,
        query: &str,
    ) -> Vec<PresetGallerySection> {
        sections(
            catalog,
            favorites,
            recents,
            &selection.into_iter().collect::<Vec<_>>(),
            query,
        )
    }

    fn kinds(sections: &[PresetGallerySection]) -> Vec<PresetSectionKind> {
        sections.iter().map(|section| section.kind).collect()
    }

    fn names(sections: &[PresetGallerySection], kind: PresetSectionKind) -> Vec<String> {
        sections
            .iter()
            .find(|section| section.kind == kind)
            .map(|section| section.items.iter().map(|item| item.name.clone()).collect())
            .unwrap_or_default()
    }

    fn more() -> Vec<StylePreset> {
        (10..20)
            .map(|index| preset(&format!("Style {index}"), index))
            .collect()
    }

    #[test]
    fn favorites_then_recent_then_whole_catalog() {
        let s = styles();
        let result = of(
            &[s.clay.clone(), s.anime.clone(), s.noir.clone()],
            std::slice::from_ref(&s.noir),
            &[s.anime.clone(), s.noir.clone()],
            None,
            "",
        );
        assert_eq!(kinds(&result), [Favorites, Recent, All]);
        assert_eq!(names(&result, Favorites), ["Film Noir"]);
        assert_eq!(names(&result, Recent), ["Cel-Shaded CG Anime", "Film Noir"]);
        assert_eq!(
            names(&result, All),
            ["Claymation", "Cel-Shaded CG Anime", "Film Noir"]
        );
        let stars: Vec<bool> = result[2].items.iter().map(|item| item.is_favorite).collect();
        assert_eq!(stars, [false, false, true]);
        assert!(
            result
                .iter()
                .all(|section| section.items.iter().all(|item| item.is_available))
        );
    }

    #[test]
    fn empty_sections_are_left_out() {
        let s = styles();
        assert_eq!(
            kinds(&of(&[s.clay], &[], &[], None, "")),
            [PresetSectionKind::All]
        );
        assert_eq!(of(&[], &[], &[], None, "").len(), 0);
    }

    #[test]
    fn sections_are_deduplicated_and_recent_is_capped() {
        let s = styles();
        let catalog: Vec<StylePreset> = [s.clay.clone(), s.clay.clone()]
            .into_iter()
            .chain(more())
            .collect();
        let recents: Vec<StylePreset> = std::iter::once(s.clay.clone()).chain(more()).collect();
        let result = of(&catalog, &[s.clay.clone(), s.clay.clone()], &recents, None, "");
        assert_eq!(names(&result, PresetSectionKind::Favorites), ["Claymation"]);
        assert_eq!(names(&result, PresetSectionKind::Recent).len(), RECENT_LIMIT);
        assert_eq!(names(&result, PresetSectionKind::Recent)[0], "Claymation");
        assert_eq!(
            names(&result, PresetSectionKind::All)
                .iter()
                .filter(|name| *name == "Claymation")
                .count(),
            1
        );
    }

    #[test]
    fn catalog_version_wins_over_the_stored_snapshot() {
        let s = styles();
        let mut renamed = s.clay.clone();
        renamed.name = "Claymation v2".to_owned();
        let result = of(
            &[renamed],
            std::slice::from_ref(&s.clay),
            std::slice::from_ref(&s.clay),
            None,
            "",
        );
        assert_eq!(names(&result, PresetSectionKind::Favorites), ["Claymation v2"]);
        assert_eq!(names(&result, PresetSectionKind::Recent), ["Claymation v2"]);
    }

    #[test]
    fn presets_missing_from_the_catalog_are_unavailable() {
        let s = styles();
        let result = of(&[s.clay], &[s.noir], &[s.anime], None, "");
        assert_eq!(names(&result, PresetSectionKind::Favorites), ["Film Noir"]);
        assert_eq!(names(&result, PresetSectionKind::Recent), ["Cel-Shaded CG Anime"]);
        assert!(!result[0].items[0].is_available);
        assert!(!result[1].items[0].is_available);
        assert!(result[2].items[0].is_available);
    }

    #[test]
    fn a_missing_selection_shown_nowhere_else_gets_its_own_section() {
        let s = styles();
        let unknown = Uuid::new_v4();
        let result = of(&[s.clay], &[], &[], Some(unknown), "");
        assert_eq!(
            kinds(&result),
            [PresetSectionKind::Selected, PresetSectionKind::All]
        );
        assert_eq!(
            result[0].items,
            [PresetTileItem {
                id: jutsu_core::support::uuid_string(unknown),
                name: UNKNOWN_PRESET_NAME.to_owned(),
                preview_url: None,
                preview_video_url: None,
                is_favorite: false,
                is_available: false,
            }]
        );
    }

    #[test]
    fn a_selection_shown_elsewhere_gets_no_extra_section() {
        let s = styles();
        assert_eq!(
            kinds(&of(std::slice::from_ref(&s.clay), &[], &[], Some(s.clay.id), "")),
            [All]
        );
        assert_eq!(
            kinds(&of(
                &[s.clay],
                std::slice::from_ref(&s.noir),
                &[],
                Some(s.noir.id),
                ""
            )),
            [Favorites, All]
        );
        assert_eq!(
            kinds(&of(
                &[],
                &[],
                std::slice::from_ref(&s.anime),
                Some(s.anime.id),
                ""
            )),
            [Recent]
        );
    }

    #[test]
    fn search_filters_every_section_ignoring_case_and_accents() {
        let s = styles();
        let result = of(
            &[s.clay.clone(), s.anime.clone(), s.noir.clone()],
            &[s.noir, s.clay.clone()],
            &[s.anime, s.clay.clone()],
            None,
            "  CLÁY ",
        );
        assert_eq!(kinds(&result), [Favorites, Recent, All]);
        assert!(result.iter().all(|section| {
            section
                .items
                .iter()
                .map(|item| item.name.as_str())
                .eq(["Claymation"])
        }));
        assert_eq!(
            of(
                std::slice::from_ref(&s.clay),
                std::slice::from_ref(&s.clay),
                &[],
                None,
                "pixel"
            )
            .len(),
            0
        );
    }

    #[test]
    fn toggling_stars_newest_first_and_unstars_missing_presets() {
        let s = styles();
        let catalog = [s.clay.clone(), s.anime.clone()];
        let mut favorites = toggled_favorite(s.clay.id, &[], &catalog);
        favorites = toggled_favorite(s.anime.id, &favorites, &catalog);
        assert_eq!(favorites, [s.anime.clone(), s.clay.clone()]);
        assert_eq!(toggled_favorite(s.clay.id, &favorites, &[]), [s.anime]);
        assert_eq!(toggled_favorite(s.noir.id, &favorites, &[s.clay]), favorites);
    }

    #[test]
    fn recording_use_moves_the_preset_to_the_front_and_caps() {
        let s = styles();
        let catalog = [s.clay.clone(), s.anime.clone(), s.noir.clone()];
        let mut recents: Vec<StylePreset> = Vec::new();
        for preset in [&s.clay, &s.anime, &s.noir, &s.clay] {
            recents = recording_use(preset.id, &recents, &catalog, &[]);
        }
        assert_eq!(recents, [s.clay, s.noir, s.anime]);

        let more = more();
        for preset in &more {
            recents = recording_use(preset.id, &recents, &more, &[]);
        }
        let expected: Vec<StylePreset> = more.iter().rev().take(RECENT_LIMIT).cloned().collect();
        assert_eq!(recents, expected);
    }

    #[test]
    fn recording_use_falls_back_to_stored_snapshots() {
        let s = styles();
        assert_eq!(
            recording_use(
                s.noir.id,
                std::slice::from_ref(&s.clay),
                &[],
                std::slice::from_ref(&s.noir)
            ),
            [s.noir, s.clay.clone()]
        );
        assert_eq!(
            recording_use(s.clay.id, &[s.anime.clone(), s.clay.clone()], &[], &[]),
            [s.clay.clone(), s.anime]
        );
        assert_eq!(
            recording_use(s.pixel.id, std::slice::from_ref(&s.clay), &[], &[]),
            [s.clay]
        );
    }

    #[test]
    fn stored_lists_round_trip_and_bad_data_is_empty() {
        let s = styles();
        let list = [s.clay, s.anime];
        assert_eq!(decode(Some(&encode(&list))), list);
        assert_eq!(decode(None).len(), 0);
        assert_eq!(decode(Some(&Value::String(String::new()))).len(), 0);
        assert_eq!(decode(Some(&Value::String("not json".to_owned()))).len(), 0);
    }

    fn catalog_presets() -> Vec<StylePreset> {
        vec![
            StylePreset {
                id: Uuid::new_v4(),
                name: "Cel-Shaded CG Anime".to_owned(),
                preview_url: Url::parse("https://example.com/a.png").unwrap(),
                preview_video_url: None,
            },
            StylePreset {
                id: Uuid::new_v4(),
                name: "Claymation".to_owned(),
                preview_url: Url::parse("https://example.com/c.png").unwrap(),
                preview_video_url: None,
            },
        ]
    }

    #[tokio::test]
    async fn refresh_caches_and_search_filters() {
        let folder = tempfile::tempdir().unwrap();
        let presets = catalog_presets();
        let mut catalog = PresetCatalog::new(folder.path().to_path_buf());
        catalog.load_cache();
        assert_eq!(catalog.presets.len(), 0);
        catalog.refresh(async { Ok(presets.clone()) }, None).await;
        assert_eq!(catalog.presets, presets);
        catalog.query = "  clay ".to_owned();
        let filtered: Vec<&str> = catalog
            .filtered()
            .iter()
            .map(|preset| preset.name.as_str())
            .collect();
        assert_eq!(filtered, ["Claymation"]);
        assert_eq!(
            catalog
                .preset(Some(presets[0].id))
                .map(|preset| preset.name.as_str()),
            Some("Cel-Shaded CG Anime")
        );

        let mut reopened = PresetCatalog::new(folder.path().to_path_buf());
        reopened.load_cache();
        assert_eq!(reopened.presets, presets);
        reopened
            .refresh(async { Err(PortError::Provider(ProviderError::Network)) }, None)
            .await;
        assert_eq!(reopened.presets, presets);
        assert_eq!(
            reopened.refresh_failure.map(|failure| failure.title).as_deref(),
            Some("Can't reach Higgsfield")
        );
        assert!(!reopened.is_refreshing);
        assert_eq!(
            cached_name(presets[0].id, folder.path()).as_deref(),
            Some("Cel-Shaded CG Anime")
        );
        assert_eq!(cached_name(Uuid::new_v4(), folder.path()), None);
        assert_eq!(cached_name(presets[0].id, &folder.path().join("missing")), None);
    }

    #[tokio::test]
    async fn cache_never_replaces_presets_already_shown() {
        let folder = tempfile::tempdir().unwrap();
        let cached = catalog_presets();
        PresetCatalog::new(folder.path().to_path_buf())
            .refresh(async { Ok(cached.clone()) }, None)
            .await;
        let mut catalog = PresetCatalog::new(folder.path().to_path_buf());
        let fresh = vec![cached[1].clone()];
        catalog.refresh(async { Ok(fresh.clone()) }, None).await;
        catalog.load_cache();
        assert_eq!(catalog.presets, fresh);
    }

    #[test]
    fn unreadable_cache_is_ignored() {
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join(CACHE_FILE_NAME), "not json").unwrap();
        let mut catalog = PresetCatalog::new(folder.path().to_path_buf());
        catalog.load_cache();
        assert_eq!(catalog.presets.len(), 0);
    }
}
