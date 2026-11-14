//! The Create draft saved between launches (`CreateDraftStore`), and the draft a replacement lost, kept for Undo
//! (`DraftReplacement`).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use jutsu_core::TemplateKind;

use serde::{Deserialize, Serialize};
use url::Url;

use super::form::CreateForm;
use crate::images::ImageDraft;

/// The video and image services write the same file: one read-modify-write at a time.
static WRITES: Mutex<()> = Mutex::new(());

/// `create-draft.json`: the video form, the image form (`ImageDraft`), and the side Create opens on. Each side is
/// written without touching the other, one write at a time.
pub struct CreateDraftStore {
    path: PathBuf,
}

/// Bumped when the saved drafts change shape; a file of another version is ignored rather than half-read.
const VERSION: i64 = 2;

#[derive(Serialize, Deserialize)]
struct SavedDrafts {
    version: i64,
    #[serde(default)]
    video: Option<CreateForm>,
    #[serde(default)]
    image: Option<serde_json::Value>,
    #[serde(default)]
    kind: Option<String>,
}

impl CreateDraftStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Option<SavedDrafts> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str::<SavedDrafts>(&text)
            .ok()
            .filter(|saved| saved.version == VERSION)
    }

    /// The saved video draft; `None` when there is none, it can't be read or another version wrote it.
    pub fn load(&self) -> Option<CreateForm> {
        self.read()?.video
    }

    /// The saved draft with files that can't be read any more dropped; `None` when nothing worth keeping is left.
    pub fn restore(&self) -> Option<CreateForm> {
        let form = self.load()?.reopening_files(&is_readable);
        form.has_inputs().then_some(form)
    }

    /// The saved image draft; `None` when there is none or it can't be read.
    pub fn load_image(&self) -> Option<ImageDraft> {
        serde_json::from_value(self.read()?.image?).ok()
    }

    /// The saved image draft with a file that can't be read any more dropped; `None` when nothing worth keeping is
    /// left.
    pub fn restore_image(&self) -> Option<ImageDraft> {
        let draft = self
            .load_image()?
            .reopening_files(|image| is_readable(&image.file_url).then(|| image.clone()));
        draft.has_inputs().then_some(draft)
    }

    /// The side Create was on; video when none was saved.
    pub fn load_kind(&self) -> TemplateKind {
        self.read()
            .and_then(|saved| saved.kind)
            .and_then(|kind| TemplateKind::from_raw(&kind))
            .unwrap_or_default()
    }

    /// Writes `form`, replacing the file in one step; `None` removes the video draft.
    pub fn save(&self, form: Option<&CreateForm>) -> std::io::Result<()> {
        self.write(|saved| saved.video = form.cloned())
    }

    /// Writes the image draft and the side, keeping the video draft; `None` removes the image draft.
    pub fn save_image(&self, draft: Option<&ImageDraft>, kind: TemplateKind) -> std::io::Result<()> {
        let image = draft
            .map(serde_json::to_value)
            .transpose()
            .map_err(std::io::Error::other)?;
        self.write(|saved| {
            saved.image = image;
            saved.kind = Some(kind.raw_value().to_owned());
        })
    }

    fn write(&self, change: impl FnOnce(&mut SavedDrafts)) -> std::io::Result<()> {
        let _turn = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let mut saved = self.read().unwrap_or(SavedDrafts {
            version: VERSION,
            video: None,
            image: None,
            kind: None,
        });
        change(&mut saved);
        if saved.kind.is_none() {
            saved.kind = Some(TemplateKind::Video.raw_value().to_owned());
        }
        let data = serde_json::to_vec(&saved).map_err(std::io::Error::other)?;
        if let Some(folder) = self.path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, &self.path)
    }
}

/// A local file that is still there.
pub fn is_readable(url: &Url) -> bool {
    url.to_file_path().is_ok_and(|path| path.is_file())
}

/// A draft replaced by Edit inputs or Generate again, kept so the toast can undo it. `None` when nothing worth
/// undoing was lost: `replaced` drops none of `previous`'s inputs, or `previous` is still exactly what the last
/// replacement left (`untouched`).
pub fn replacement(
    previous: &CreateForm,
    replaced: &CreateForm,
    untouched: Option<&CreateForm>,
) -> Option<CreateForm> {
    (Some(previous) != untouched && replaced.drops_inputs(previous)).then(|| previous.clone())
}
