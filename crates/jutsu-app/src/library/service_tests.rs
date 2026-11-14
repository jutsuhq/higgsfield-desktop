//! Environment-level tests of the Library service: a temp store, and a fake account whose provider also answers
//! the history and likes.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use jutsu_core::{
    Connectivity, Credential, GenerationHistory, GenerationLikes, GenerationProvider, HistoryCursor,
    HistoryPage, PortError, ProviderError,
};
use time::{OffsetDateTime, UtcOffset};
use uuid::Uuid;

use super::LibraryService;
use crate::connection::MemoryVault;
use crate::environment::{AppDirectories, AppEnvironment, PlatformServices};
use crate::library::actions::Trash;
use crate::library::culling::CullingAction;
use crate::library::fixture::{history_image, history_item, video};
use crate::library::media::{LibraryKind, LibraryOrigin};
use crate::library::view::{CardKind, LibraryRequest, LibrarySnapshot};
use crate::provider::{BuiltProvider, PersistTokens, ProviderFactory, UnlimitedOptIn};
use crate::queue::MissingFfmpeg;
use crate::testing::{BalanceProvider, FakeServices};

struct AlwaysOnline;

impl Connectivity for AlwaysOnline {
    fn updates(&self) -> BoxStream<'static, bool> {
        futures::stream::once(async { true })
            .chain(futures::stream::pending())
            .boxed()
    }
}

/// The account's history, one page per call, and the likes it was asked for; refuses the likes in `refused`.
#[derive(Default)]
struct FakeAccount {
    pages: Mutex<Vec<HistoryPage>>,
    likes: Mutex<Vec<(String, bool)>>,
    refused: HashSet<String>,
}

#[async_trait]
impl GenerationHistory for FakeAccount {
    async fn history(&self, _after: Option<HistoryCursor>) -> Result<HistoryPage, PortError> {
        let mut pages = self.pages.lock().unwrap();
        if pages.is_empty() {
            return Err(ProviderError::Network.into());
        }
        Ok(pages.remove(0))
    }
}

#[async_trait]
impl GenerationLikes for FakeAccount {
    async fn set_liked(&self, request_id: String, is_liked: bool) -> Result<(), PortError> {
        if self.refused.contains(&request_id) {
            return Err(ProviderError::Network.into());
        }
        self.likes.lock().unwrap().push((request_id, is_liked));
        Ok(())
    }
}

struct AccountFactory(Arc<FakeAccount>);

impl ProviderFactory for AccountFactory {
    fn provider(&self, _: Credential, _: PersistTokens, _: UnlimitedOptIn) -> Arc<dyn GenerationProvider> {
        Arc::new(BalanceProvider::credits(100, None))
    }

    fn build(
        &self,
        credential: Credential,
        persist: PersistTokens,
        unlimited: UnlimitedOptIn,
    ) -> BuiltProvider {
        BuiltProvider {
            provider: self.provider(credential, persist, unlimited),
            history: Some(self.0.clone()),
            likes: Some(self.0.clone()),
            virality: None,
            voices: None,
            credits: None,
        }
    }
}

struct Fixture {
    _folder: tempfile::TempDir,
    environment: Arc<AppEnvironment>,
    account: Arc<FakeAccount>,
}

impl Fixture {
    fn library(&self) -> &LibraryService {
        &self.environment.library
    }

    async fn snapshot(&self, origin: LibraryOrigin) -> LibrarySnapshot {
        let request = LibraryRequest {
            origin,
            kind: LibraryKind::All,
            ..LibraryRequest::default()
        };
        let url_for = |path: &Path| Some(format!("media:{}", path.display()));
        self.library()
            .snapshot(&request, OffsetDateTime::now_utc(), &url_for)
            .await
            .unwrap()
    }
}

async fn fixture(account: FakeAccount, signed_in: bool) -> Fixture {
    let folder = tempfile::tempdir().unwrap();
    let account = Arc::new(account);
    let environment = AppEnvironment::open(
        AppDirectories {
            data_dir: folder.path().join("data"),
            default_output_dir: folder.path().join("Videos").join("Jutsu Studio"),
        },
        PlatformServices {
            vault: Arc::new(MemoryVault::new()),
            connection: Arc::new(FakeServices::default()),
            providers: Arc::new(AccountFactory(account.clone())),
            connectivity: Arc::new(AlwaysOnline),
            media: Some(Arc::new(MissingFfmpeg)),
            utc_offset: UtcOffset::UTC,
        },
    )
    .await
    .unwrap();
    environment.start().await;
    if signed_in {
        environment.connection.start_sign_in().await.unwrap();
    }
    Fixture {
        _folder: folder,
        environment,
        account,
    }
}

fn page() -> HistoryPage {
    HistoryPage {
        items: vec![history_item("gen_1", false)],
        images: vec![history_image("img_1", 5.0)],
        next_cursor: None,
    }
}

#[tokio::test]
async fn the_accounts_history_is_cached_and_listed_from_higgsfield() {
    let fixture = fixture(
        FakeAccount {
            pages: Mutex::new(vec![page()]),
            ..FakeAccount::default()
        },
        true,
    )
    .await;
    fixture.library().load_history(true).await;

    let remote = fixture.snapshot(LibraryOrigin::Higgsfield).await;
    assert_eq!(remote.cards.len(), 2);
    assert!(remote.cards.iter().all(|card| card.needs_import));
    assert_eq!(remote.counts.videos, 1);
    assert_eq!(remote.counts.images, 1);
    assert!(remote.placeholder.is_none());
    let footer = remote.history.unwrap();
    assert!(!footer.has_more && !footer.is_loading && footer.can_refresh);

    let local = fixture.snapshot(LibraryOrigin::Local).await;
    assert_eq!(local.cards.len(), 0);
    assert_eq!(local.empty.unwrap().title, "No results yet");
}

#[tokio::test]
async fn an_account_image_goes_by_its_model_without_a_prompt_and_tags_its_aspect() {
    let mut image = history_image("img_1", 5.0);
    image.prompt = String::new();
    let fixture = fixture(
        FakeAccount {
            pages: Mutex::new(vec![HistoryPage {
                items: vec![history_item("gen_1", false)],
                images: vec![image],
                next_cursor: None,
            }]),
            ..FakeAccount::default()
        },
        true,
    )
    .await;
    fixture.library().load_history(true).await;

    let snapshot = fixture.snapshot(LibraryOrigin::Higgsfield).await;
    let image = snapshot
        .cards
        .iter()
        .find(|card| card.kind == CardKind::Image)
        .unwrap();
    assert_eq!(image.title, "soul-2.0");
    assert!(image.subtitle.starts_with("soul-2.0 · "), "{}", image.subtitle);
    assert_eq!(image.aspect_text.as_deref(), Some("1:1"));
    assert_eq!(image.duration_text, None);
    let video = snapshot
        .cards
        .iter()
        .find(|card| card.kind == CardKind::Video)
        .unwrap();
    assert_eq!(video.aspect_text, None);
}

#[tokio::test]
async fn a_failed_history_load_is_shown_in_the_footer() {
    let fixture = fixture(FakeAccount::default(), true).await;
    fixture.library().load_history(true).await;
    let snapshot = fixture.snapshot(LibraryOrigin::Higgsfield).await;
    assert_eq!(snapshot.cards.len(), 0);
    assert!(snapshot.history.unwrap().failure.is_some());
}

#[tokio::test]
async fn without_an_account_the_history_says_why() {
    let fixture = fixture(FakeAccount::default(), false).await;
    fixture.library().load_history(true).await;
    let snapshot = fixture.snapshot(LibraryOrigin::Higgsfield).await;
    assert!(snapshot.placeholder.is_some());
    assert_eq!(fixture.account.pages.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn starring_a_history_result_likes_it_on_the_account() {
    let fixture = fixture(
        FakeAccount {
            pages: Mutex::new(vec![page()]),
            ..FakeAccount::default()
        },
        true,
    )
    .await;
    fixture.library().load_history(true).await;
    let ids: Vec<Uuid> = fixture
        .snapshot(LibraryOrigin::Higgsfield)
        .await
        .cards
        .iter()
        .map(|card| card.id.parse().unwrap())
        .collect();

    fixture.library().set_starred(&ids, true).await;

    let mut likes = fixture.account.likes.lock().unwrap().clone();
    likes.sort();
    assert_eq!(likes, [("gen_1".to_owned(), true), ("img_1".to_owned(), true)]);
    let snapshot = fixture.snapshot(LibraryOrigin::Higgsfield).await;
    assert!(snapshot.cards.iter().all(|card| card.is_starred));
}

#[tokio::test]
async fn a_refused_like_leaves_the_star_off_and_alerts() {
    let fixture = fixture(
        FakeAccount {
            pages: Mutex::new(vec![page()]),
            refused: HashSet::from(["gen_1".to_owned()]),
            ..FakeAccount::default()
        },
        true,
    )
    .await;
    fixture.library().load_history(true).await;
    let snapshot = fixture.snapshot(LibraryOrigin::Higgsfield).await;
    let video = snapshot
        .cards
        .iter()
        .find(|card| card.duration_text.is_some())
        .unwrap();

    fixture
        .library()
        .set_starred(&[video.id.parse().unwrap()], true)
        .await;

    let snapshot = fixture.snapshot(LibraryOrigin::Higgsfield).await;
    assert!(!snapshot.cards.iter().any(|card| card.is_starred));
    assert!(fixture.environment.connection.alert().is_some());
}

#[tokio::test]
async fn a_refused_video_like_fails_an_agents_star_and_leaves_the_images_unstarred() {
    let fixture = fixture(
        FakeAccount {
            pages: Mutex::new(vec![page()]),
            refused: HashSet::from(["gen_1".to_owned()]),
            ..FakeAccount::default()
        },
        true,
    )
    .await;
    fixture.library().load_history(true).await;
    let ids: Vec<Uuid> = fixture
        .snapshot(LibraryOrigin::Higgsfield)
        .await
        .cards
        .iter()
        .map(|card| card.id.parse().unwrap())
        .collect();

    let result = fixture.library().star(&ids, true).await;

    assert!(matches!(result, Err(crate::library::actions::StarError::Like(_))));
    let snapshot = fixture.snapshot(LibraryOrigin::Higgsfield).await;
    assert!(!snapshot.cards.iter().any(|card| card.is_starred));
    assert!(
        fixture
            .account
            .likes
            .lock()
            .unwrap()
            .iter()
            .all(|(id, _)| id != "img_1")
    );
    assert!(fixture.environment.connection.alert().is_none());
}

#[tokio::test]
async fn a_rejected_result_is_hidden_until_undone() {
    let fixture = fixture(FakeAccount::default(), false).await;
    let material = video(1.0);
    fixture
        .environment
        .store
        .save_material(material.clone())
        .await
        .unwrap();
    let local = fixture.snapshot(LibraryOrigin::Local).await;
    assert_eq!(local.cards.len(), 1);
    assert!(
        local.cards[0].is_unavailable,
        "its file isn't there, so it stays listed as unavailable"
    );

    let done = fixture
        .library()
        .cull(CullingAction::Reject, &[material.id])
        .await;
    assert_eq!(done, Some(CullingAction::Reject));
    let culled = fixture.snapshot(LibraryOrigin::Local).await;
    assert_eq!(culled.cards.len(), 0);
    assert!(culled.can_undo);
    assert_eq!(fixture.library().counts().await.all, 0);

    assert_eq!(
        fixture.library().undo_culling().await,
        Some(material.id),
        "the restored result becomes the current card"
    );
    assert_eq!(fixture.snapshot(LibraryOrigin::Local).await.cards.len(), 1);
    assert_eq!(fixture.library().undo_culling().await, None);
}

/// Every file is there and moves.
#[derive(Default)]
struct RecordingTrash(Mutex<Vec<PathBuf>>);

impl Trash for RecordingTrash {
    fn file_exists(&self, _path: &Path) -> bool {
        true
    }

    fn move_to_trash(&self, path: &Path) -> Result<(), String> {
        self.0.lock().unwrap().push(path.to_owned());
        Ok(())
    }
}

#[tokio::test]
async fn deleting_trashes_the_file_and_takes_the_result_off_the_library() {
    let fixture = fixture(FakeAccount::default(), false).await;
    let material = video(1.0);
    fixture
        .environment
        .store
        .save_material(material.clone())
        .await
        .unwrap();
    let trash = RecordingTrash::default();

    let message = fixture.library().delete_with(&[material.id], &trash).await;

    assert_eq!(message, None);
    assert_eq!(trash.0.lock().unwrap().len(), 1);
    assert_eq!(fixture.snapshot(LibraryOrigin::Local).await.cards.len(), 0);
}

/// Answers every score call with `outcome`, or refuses it with `error`.
struct FakeScorer(Result<jutsu_core::ViralityOutcome, ProviderError>);

#[async_trait]
impl jutsu_core::ViralityScoring for FakeScorer {
    async fn virality_estimate(&self, _job_id: &str) -> Result<Option<jutsu_core::Cost>, PortError> {
        Ok(None)
    }

    async fn score_virality(&self, _job_id: &str) -> Result<jutsu_core::ViralityOutcome, PortError> {
        self.0.clone().map_err(PortError::from)
    }
}

fn credits(amount: i64) -> jutsu_core::Cost {
    jutsu_core::Cost::credits(rust_decimal::Decimal::from(amount), false)
}

fn entry() -> crate::library::virality::ViralityEntry {
    crate::library::virality::ViralityEntry {
        item_id: Uuid::new_v4(),
        job_id: "req-1".to_owned(),
    }
}

/// `VariantScorer.score`: a score's price is logged when it is sent and settled at what the answer says it charged.
#[tokio::test]
async fn a_score_is_charged_when_sent_and_settled_on_the_answer() {
    let fixture = fixture(FakeAccount::default(), true).await;
    let scorer = FakeScorer(Ok(jutsu_core::ViralityOutcome {
        score: None,
        charged: Some(credits(2)),
    }));
    let failure = fixture.library().score_one(&scorer, &entry(), &credits(3)).await;
    assert_eq!(failure.as_deref(), Some(crate::library::virality::UNREADABLE));
    let charges = fixture.library().image_spend.charges();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].cost, credits(2));
    assert!(!charges[0].is_pending);
    assert_eq!(
        charges[0].paid_for(),
        jutsu_core::ImageChargePurpose::ViralityScore
    );
}

/// A call refused before it ran is taken off again; one that may have run stays at its price.
#[tokio::test]
async fn a_refused_score_takes_its_charge_off() {
    let fixture = fixture(FakeAccount::default(), true).await;
    let refused = FakeScorer(Err(ProviderError::Rejected));
    assert!(
        fixture
            .library()
            .score_one(&refused, &entry(), &credits(3))
            .await
            .is_some()
    );
    assert_eq!(
        fixture.library().image_spend.charges(),
        Vec::<jutsu_core::ImageCharge>::new()
    );
    let lost = FakeScorer(Err(ProviderError::Network));
    assert!(
        fixture
            .library()
            .score_one(&lost, &entry(), &credits(3))
            .await
            .is_some()
    );
    let charges = fixture.library().image_spend.charges();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].cost, credits(3));
    assert!(!charges[0].is_pending);
}

/// Today's spend under the cap counts the image charges too, as Create's does.
#[tokio::test]
async fn the_cap_counts_the_image_charges() {
    let fixture = fixture(FakeAccount::default(), true).await;
    fixture
        .environment
        .preferences
        .update(crate::preferences::Preferences {
            spend_cap_credits_enabled: Some(true),
            spend_cap_credits: Some(1000),
            ..Default::default()
        })
        .unwrap();
    fixture
        .library()
        .image_spend
        .add(jutsu_core::ImageCharge::new(
            Uuid::new_v4(),
            OffsetDateTime::now_utc(),
            credits(150),
            jutsu_core::ProviderKind::Account,
            "Nano Banana",
            1,
            Some(jutsu_core::ImageChargePurpose::ReferenceImages),
        ))
        .unwrap();
    let (committed, cap) = fixture.library().cap_and_spend().await.unwrap();
    assert_eq!(committed, rust_decimal::Decimal::from(150));
    assert!(cap.is_some());
}

/// A 1×1 PNG.
const PIXEL_PNG: [u8; 70] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00,
    0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00,
    0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xfc, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05,
    0x05, 0x02, 0x00, 0x5f, 0xc8, 0xf1, 0xd2, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42,
    0x60, 0x82,
];

/// Three images of one variant group whose files are here, and a fourth whose file is gone; the files live as long
/// as the returned folder.
async fn image_group(
    fixture: &Fixture,
) -> (
    tempfile::TempDir,
    Uuid,
    Vec<jutsu_core::Material>,
    jutsu_core::Material,
) {
    use crate::library::fixture::catalog_image;
    use jutsu_core::Culling;

    let folder = tempfile::tempdir().unwrap();
    let group = Uuid::new_v4();
    let mut here = Vec::new();
    for (index, prompt) in ["A knight", "", "A queen"].into_iter().enumerate() {
        let minutes = f64::from(u8::try_from(index).unwrap()) + 1.0;
        let mut material = catalog_image(prompt, Some(group), minutes, Culling::Unreviewed, false);
        let path = folder.path().join(format!("variant {index}.png"));
        std::fs::write(&path, PIXEL_PNG).unwrap();
        material.current.file_url = Some(crate::testing::file_url(&path));
        here.push(material);
    }
    let missing = catalog_image("Gone", Some(group), 4.0, Culling::Unreviewed, false);
    for material in here.iter().chain([&missing]) {
        fixture
            .environment
            .store
            .save_material(material.clone())
            .await
            .unwrap();
    }
    (folder, group, here, missing)
}

#[allow(clippy::unnecessary_wraps, reason = "a `UrlFor`, which may find no URL")]
fn media_url(path: &Path) -> Option<String> {
    Some(format!("media:{}", path.display()))
}

/// `LibraryViewCulling.compareVariants` and `LibraryImageMenu`: a group's images step two at a time without the one
/// whose file is gone, which offers neither Compare… nor Copy Image.
#[tokio::test]
async fn an_image_group_compares_its_variants_here() {
    let fixture = fixture(FakeAccount::default(), false).await;
    let (_folder, group, here, missing) = image_group(&fixture).await;
    assert_eq!(
        fixture.snapshot(LibraryOrigin::Local).await.cards.len(),
        1,
        "the group folds into its face"
    );
    let request = LibraryRequest {
        kind: LibraryKind::Image,
        ..LibraryRequest::default()
    };
    let expanded = LibraryRequest {
        expanded_groups: vec![group.to_string()],
        ..request.clone()
    };
    let snapshot = fixture
        .library()
        .snapshot(&expanded, OffsetDateTime::now_utc(), &media_url)
        .await
        .unwrap();
    let menu = |id: Uuid| {
        let card = snapshot
            .cards
            .iter()
            .find(|card| card.id == id.to_string())
            .unwrap();
        (card.menu.can_compare, card.menu.can_copy_image)
    };
    assert_eq!(menu(here[0].id), (true, true));
    assert_eq!(
        menu(missing.id),
        (false, false),
        "a missing file can't be compared or copied"
    );

    let variants = fixture
        .library()
        .image_variants(&request, group, &media_url)
        .await
        .unwrap();
    let titles: Vec<&str> = variants.images.iter().map(|image| image.title.as_str()).collect();
    assert_eq!(titles, ["A knight", "variant 1.png", "A queen"]);
    assert_eq!(variants.sides.position.as_deref(), Some("Variant 2 of 3"));
    assert!(variants.images[0].url.as_deref().unwrap().starts_with("media:"));
    assert!(
        fixture
            .library()
            .image_variants(&request, Uuid::new_v4(), &media_url)
            .await
            .is_err()
    );
}

/// Two picked images compare side by side in the order they were picked, Copy Image reads the file that is here, and
/// the selection bar's Compare leaves a missing file out.
#[tokio::test]
async fn two_images_compare_and_copy_their_pixels() {
    let fixture = fixture(FakeAccount::default(), false).await;
    let (_folder, _group, here, missing) = image_group(&fixture).await;
    let pair = fixture
        .library()
        .image_compare(&[here[2].id, here[0].id], &media_url)
        .await
        .unwrap();
    let ids: Vec<String> = pair.images.iter().map(|image| image.id.clone()).collect();
    assert_eq!(ids, [here[2].id.to_string(), here[0].id.to_string()]);
    assert_eq!(pair.sides.position, None);
    for twice_or_missing in [[here[0].id, here[0].id], [here[0].id, missing.id]] {
        assert!(
            fixture
                .library()
                .image_compare(&twice_or_missing, &media_url)
                .await
                .is_err()
        );
    }

    let pixels = fixture.library().image_pixels(here[0].id).await.unwrap();
    assert_eq!((pixels.width, pixels.height, pixels.rgba.len()), (1, 1, 4));
    assert!(fixture.library().image_pixels(missing.id).await.is_none());

    let bulk = fixture.library().bulk(&[here[0].id, missing.id]).await.unwrap();
    assert_eq!(bulk.compare_ids, [here[0].id.to_string()]);
}

/// `material` with a second version made from its first, each with a file in `folder` named after `names`; the first
/// version has the material's own ID, as a material's first version does.
async fn with_edited_version(
    fixture: &Fixture,
    mut material: jutsu_core::Material,
    folder: &Path,
    names: [&str; 2],
) -> (jutsu_core::Material, [PathBuf; 2]) {
    let [first, second] = names.map(|name| folder.join(name));
    for path in [&first, &second] {
        std::fs::write(path, PIXEL_PNG).unwrap();
    }
    material.current.file_url = Some(crate::testing::file_url(&first));
    if let Some(video) = material.current.video.as_mut() {
        video.output_file.clone_from(&material.current.file_url);
    }
    let original = material.reference();
    assert_eq!(original.version_id, material.id);
    let mut edited = material.current.clone();
    edited.id = Uuid::new_v4();
    edited.file_url = Some(crate::testing::file_url(&second));
    edited.inputs = vec![jutsu_core::MaterialInput {
        source: original,
        role: jutsu_core::MaterialInputRole::Parent,
        position: 0,
    }];
    material.add_version(edited).unwrap();
    let saved = fixture.environment.store.save_material(material).await.unwrap();
    (saved, [first, second])
}

/// `LibraryViewImages.originalImageCompare` and `MaterialVersions.imagePair`: a saved image made from an earlier
/// version offers Compare with Original, the original on the left with an ID of its own, while that file is here.
#[tokio::test]
async fn an_image_made_from_an_earlier_version_compares_with_the_original() {
    use crate::library::fixture::catalog_image;

    let fixture = fixture(FakeAccount::default(), false).await;
    let folder = tempfile::tempdir().unwrap();
    let image = catalog_image("A knight", None, 1.0, jutsu_core::Culling::Unreviewed, false);
    let (material, [first, _]) = with_edited_version(
        &fixture,
        image,
        folder.path(),
        ["knight.png", "knight upscaled.png"],
    )
    .await;
    let has_original = || async {
        let request = LibraryRequest {
            kind: LibraryKind::Image,
            ..LibraryRequest::default()
        };
        let snapshot = fixture
            .library()
            .snapshot(&request, OffsetDateTime::now_utc(), &media_url)
            .await
            .unwrap();
        snapshot.cards[0].menu.has_original
    };
    assert!(has_original().await);

    let compare = fixture
        .library()
        .image_compare_original(material.id, &media_url)
        .await
        .unwrap();
    let titles: Vec<&str> = compare.images.iter().map(|image| image.title.as_str()).collect();
    assert_eq!(titles, ["Original", "New version"]);
    assert_ne!(compare.images[0].id, material.id.to_string());
    assert_eq!(compare.images[1].id, material.id.to_string());
    assert!(compare.images[0].url.as_deref().unwrap().ends_with("knight.png"));
    assert!(
        compare.images[1]
            .url
            .as_deref()
            .unwrap()
            .ends_with("knight upscaled.png")
    );

    std::fs::remove_file(first).unwrap();
    assert!(
        !has_original().await,
        "an original whose file is gone isn't offered"
    );
    assert!(
        fixture
            .library()
            .image_compare_original(material.id, &media_url)
            .await
            .is_err()
    );
}

/// `MaterialVersions.videoPair`: the original's track has an ID of its own, though its version has the material's.
#[tokio::test]
async fn the_original_track_never_shares_the_results_id() {
    let fixture = fixture(FakeAccount::default(), false).await;
    let folder = tempfile::tempdir().unwrap();
    let (material, _files) = with_edited_version(
        &fixture,
        video(1.0),
        folder.path(),
        ["rooftop.mp4", "rooftop smooth.mp4"],
    )
    .await;
    let compare = fixture
        .library()
        .compare_original(material.id, &media_url)
        .await
        .unwrap();
    let ids: Vec<&str> = compare.tracks.iter().map(|track| track.id.as_str()).collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert_eq!(ids[1], material.id.to_string());
}

#[test]
fn the_source_line_names_the_file_or_says_where_the_source_is() {
    use super::source_file_name;
    assert_eq!(
        source_file_name(&url::Url::parse("file:///tmp/clips/rooftop%20take.mov").unwrap()),
        "rooftop take.mov"
    );
    assert_eq!(
        source_file_name(&url::Url::parse("https://cdn.example.com/a.mp4").unwrap()),
        ""
    );
}
