//! The files that went into a result, to save back to the computer (`ResultInputs.swift`): the source video, the
//! reference images and the speech. The copy here is used while it is there; otherwise Higgsfield's is downloaded.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use url::Url;

use super::item::LibraryItem;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputRole {
    Source,
    /// 1-based, in the order the job sent them.
    Reference(usize),
    Speech,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputFile {
    pub role: InputRole,
    /// The picked file; `None` when the input only exists on Higgsfield.
    pub local: Option<PathBuf>,
    /// Where Higgsfield keeps the uploaded file.
    pub remote: Option<Url>,
}

impl InputFile {
    pub fn title(&self) -> String {
        match self.role {
            InputRole::Source => "Source video".to_owned(),
            InputRole::Reference(number) => format!("Reference {number}"),
            InputRole::Speech => "Speech".to_owned(),
        }
    }

    /// "rooftop – Source.mp4", "rooftop – Reference 2.png", "rooftop – Speech.mp3": the extension of the file, else
    /// of the link, else the kind's own.
    pub fn file_name(&self, base: &str) -> String {
        let label = match self.role {
            InputRole::Source => "Source".to_owned(),
            InputRole::Reference(number) => format!("Reference {number}"),
            InputRole::Speech => "Speech".to_owned(),
        };
        let fallback = match self.role {
            InputRole::Source => "mp4",
            InputRole::Reference(_) => "jpg",
            InputRole::Speech => "m4a",
        };
        let extension = self
            .local
            .as_ref()
            .and_then(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_owned)
            })
            .or_else(|| {
                self.remote.as_ref().and_then(|url| {
                    url.path_segments()?
                        .next_back()?
                        .rsplit_once('.')
                        .map(|(_, extension)| extension.to_owned())
                })
            })
            .filter(|extension| !extension.is_empty() && extension.len() <= 5)
            .unwrap_or_else(|| fallback.to_owned());
        format!("{} – {label}.{}", file_safe(base), extension.to_lowercase())
    }

    /// Every input of `item` that has a copy here or on Higgsfield.
    pub fn all(item: &LibraryItem) -> Vec<Self> {
        let spec = item.spec();
        let uploaded = item.job.inputs.as_ref().or(spec.remote_inputs.as_ref());
        let mut files = Vec::new();
        if let Some(source) = &spec.source {
            files.push(Self::new(
                InputRole::Source,
                &source.file_url,
                uploaded
                    .and_then(|inputs| inputs.video.as_ref())
                    .map(|asset| asset.url.clone()),
            ));
        }
        for (index, reference) in spec.references.iter().enumerate() {
            files.push(Self::new(
                InputRole::Reference(index + 1),
                &reference.file_url,
                uploaded
                    .and_then(|inputs| inputs.images.get(index))
                    .map(|asset| asset.url.clone()),
            ));
        }
        if let Some(speech) = &spec.speech {
            files.push(Self::new(
                InputRole::Speech,
                &speech.file_url,
                uploaded
                    .and_then(|inputs| inputs.audio.as_ref())
                    .map(|asset| asset.url.clone()),
            ));
        }
        files
            .into_iter()
            .filter(|file| file.local.is_some() || file.remote.is_some())
            .collect()
    }

    /// A file URL is the local copy; a web URL means the spec itself points at Higgsfield's copy.
    fn new(role: InputRole, file: &Url, uploaded: Option<Url>) -> Self {
        if file.scheme() == "file" {
            Self {
                role,
                local: file.to_file_path().ok(),
                remote: uploaded,
            }
        } else {
            Self {
                role,
                local: None,
                remote: uploaded.or_else(|| Some(file.clone())),
            }
        }
    }
}

/// Where each input goes in `folder`: its own name, or " 2", " 3"… when a file there or an earlier input has it
/// (`InputExport.choose`).
pub fn folder_targets(
    files: Vec<InputFile>,
    base: &str,
    folder: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Vec<(InputFile, PathBuf)> {
    let mut taken: HashSet<PathBuf> = HashSet::new();
    files
        .into_iter()
        .map(|file| {
            let target = jutsu_core::unused_file_path(&folder.join(file.file_name(base)), |path| {
                taken.contains(path) || exists(path)
            });
            taken.insert(target.clone());
            (file, target)
        })
        .collect()
}

/// "Can't save source video and reference 2": the inputs that couldn't be saved, as one alert's title.
pub fn failure_title(titles: &[String]) -> String {
    let lowered: Vec<String> = titles.iter().map(|title| title.to_lowercase()).collect();
    let list = match lowered.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    };
    format!("Can't save {list}")
}

/// The alert's message after an input couldn't be saved.
pub const FAILURE_MESSAGE: &str =
    "It is no longer on this computer, and the download from Higgsfield failed. Try again in a moment.";

/// A name without path separators or leading dots; "Result" when nothing is left.
pub fn file_safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if matches!(character, '/' | ':' | '\\') {
                '-'
            } else {
                character
            }
        })
        .collect();
    let visible = cleaned.trim_start_matches(|character: char| character == '.' || character.is_whitespace());
    if visible.is_empty() {
        "Result".to_owned()
    } else {
        visible.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{InputFile, InputRole, file_safe};
    use crate::library::fixture::{ItemFixture, file, history_url};

    #[test]
    fn names_say_what_each_input_was() {
        let item = ItemFixture {
            references: 2,
            ..ItemFixture::default()
        }
        .item();
        let inputs = InputFile::all(&item);
        assert_eq!(
            inputs.iter().map(|input| input.role).collect::<Vec<_>>(),
            [
                InputRole::Source,
                InputRole::Reference(1),
                InputRole::Reference(2)
            ]
        );
        assert_eq!(inputs[0].file_name("rooftop"), "rooftop – Source.mov");
        assert_eq!(inputs[2].file_name("rooftop"), "rooftop – Reference 2.png");
        assert_eq!(inputs[2].title(), "Reference 2");
    }

    #[test]
    fn a_remote_input_takes_the_links_extension() {
        let input = InputFile {
            role: InputRole::Reference(1),
            local: None,
            remote: Some(history_url("a.webp")),
        };
        assert_eq!(input.file_name("x"), "x – Reference 1.webp");
        let bare = InputFile {
            role: InputRole::Source,
            local: file("/tmp/clip").to_file_path().ok(),
            remote: None,
        };
        assert_eq!(bare.file_name("x"), "x – Source.mp4");
    }

    /// `InputExport`: several inputs never overwrite a file, nor each other.
    #[test]
    fn a_folder_save_takes_unused_names() {
        let input = |number| InputFile {
            role: InputRole::Reference(number),
            local: None,
            remote: Some(history_url("a.png")),
        };
        let folder = std::path::Path::new("/tmp/inputs");
        let taken = folder.join("x – Reference 1.png");
        let targets = super::folder_targets(vec![input(1), input(1), input(2)], "x", folder, |path| {
            path == taken
        });
        let names: Vec<_> = targets
            .iter()
            .map(|(_, target)| target.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "x – Reference 1 2.png",
                "x – Reference 1 3.png",
                "x – Reference 2.png"
            ]
        );
    }

    #[test]
    fn failures_share_one_title() {
        let titles = |list: &[&str]| list.iter().map(|title| (*title).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            super::failure_title(&titles(&["Source video"])),
            "Can't save source video"
        );
        assert_eq!(
            super::failure_title(&titles(&["Source video", "Reference 2"])),
            "Can't save source video and reference 2"
        );
        assert_eq!(
            super::failure_title(&titles(&["Source video", "Reference 1", "Reference 2"])),
            "Can't save source video, reference 1, and reference 2"
        );
    }

    #[test]
    fn file_safe_names_drop_separators_and_leading_dots() {
        assert_eq!(file_safe("a/b:c"), "a-b-c");
        assert_eq!(file_safe(" .hidden"), "hidden");
        assert_eq!(file_safe(".."), "Result");
    }
}
