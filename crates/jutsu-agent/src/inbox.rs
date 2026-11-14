//! Where files from agents wait until the command that names them takes them: `inbox/` beside the socket. The helper
//! sends each file's bytes over the socket (`UploadCommand`) and the app writes them here itself.
//!
//! Nothing staged is ever replaced. Every upload gets a folder of its own, `inbox/<UUID>/`, made by this call and no
//! other, and its file is moved in only where nothing is yet. A repeated request, or one refused later for a
//! conflicting `request_id`, can't change a file a waiting job still reads.

use crate::Uuid;
use std::{
    collections::HashSet,
    fs, io,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

/// The largest file an agent can hand over: 4 GiB, a long 4K source video.
pub const MAX_FILE_SIZE: i64 = 4 << 30;
/// What an upload must leave free on the inbox's volume: about 2 GB, room for the results the app downloads.
pub const MIN_FREE_SPACE: i64 = 2_000_000_000;
/// The most the waiting uploads may take together: 8 GiB, two of the largest files.
pub const MAX_STORED_SIZE: i64 = 8 << 30;
/// The name of the hidden file an upload is written to until it is complete.
pub const PARTIAL_NAME: &str = ".partial";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentInbox {
    pub folder: PathBuf,
}

impl AgentInbox {
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    /// A new, empty folder for one upload, readable by this user only. Making a folder fails on an existing name, so
    /// a folder is never shared even if two UUIDs ever met; it then tries another.
    pub fn make_upload_folder(&self) -> io::Result<PathBuf> {
        fs::create_dir_all(&self.folder)?;
        for _ in 0..3 {
            let candidate = self.folder.join(Uuid::new_v4().to_string());
            match private_dir(&candidate) {
                Ok(()) => return Ok(candidate),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::ErrorKind::AlreadyExists.into())
    }

    /// Why an upload of `size` bytes is refused: it would leave less than `MIN_FREE_SPACE` on the volume
    /// (`free_space`, `None` when unknown, which doesn't refuse it), or take the inbox (`stored`, what it holds now)
    /// past `MAX_STORED_SIZE`. `None` when it fits.
    pub fn room_problem(size: i64, free_space: Option<i64>, stored: i64) -> Option<String> {
        if let Some(free) = free_space
            && free - size < MIN_FREE_SPACE
        {
            return Some(format!(
                "The computer has {} free, and the file needs {} with {} left free after it: free up {} or send a smaller \
                 file.",
                gigabytes(free),
                gigabytes(size),
                gigabytes(MIN_FREE_SPACE),
                gigabytes(size + MIN_FREE_SPACE - free)
            ));
        }
        if stored + size > MAX_STORED_SIZE {
            return Some(format!(
                "Files from agents waiting in Jutsu Studio take {}, and with this {} file they would pass the {} they \
                 may take: wait for their jobs to finish, or send a smaller file.",
                gibibytes(stored),
                gibibytes(size),
                gibibytes(MAX_STORED_SIZE)
            ));
        }
        None
    }

    /// The bytes the upload folders hold now, partial files included; 0 when the inbox can't be read.
    pub fn stored_size(&self) -> i64 {
        fn walk(folder: &Path) -> u64 {
            let Ok(entries) = fs::read_dir(folder) else {
                return 0;
            };
            entries
                .flatten()
                .map(|entry| match entry.file_type() {
                    Ok(kind) if kind.is_dir() => walk(&entry.path()),
                    Ok(kind) if kind.is_file() => entry.metadata().map_or(0, |metadata| metadata.len()),
                    _ => 0,
                })
                .sum()
        }
        i64::try_from(walk(&self.folder)).unwrap_or(i64::MAX)
    }

    /// The upload folder `path` is in, when it is one of this inbox's: `inbox/<UUID>/name`, nothing else.
    pub fn upload_folder(&self, path: &str) -> Option<PathBuf> {
        let file = normalized(Path::new(path));
        let folder = file.parent()?;
        let name = folder.file_name()?.to_str()?;
        Uuid::parse(name)?;
        (folder.parent()? == normalized(&self.folder)).then(|| folder.to_path_buf())
    }

    /// Removes the upload folders of `paths`, the files of a request that was refused or never queued.
    pub fn remove(&self, paths: &[String]) {
        let folders: HashSet<PathBuf> = paths.iter().filter_map(|path| self.upload_folder(path)).collect();
        for folder in folders {
            let _ = fs::remove_dir_all(folder);
        }
    }

    /// Removes the upload folders last changed before `cutoff` that hold none of `kept`, the files jobs still read.
    /// Returns how many it removed. A folder whose date can't be read stays.
    pub fn sweep(&self, kept: &HashSet<String>, cutoff: SystemTime) -> usize {
        let kept_folders: HashSet<PathBuf> = kept
            .iter()
            .filter_map(|path| self.upload_folder(path))
            .filter_map(|folder| folder.file_name().map(PathBuf::from))
            .collect();
        let Ok(entries) = fs::read_dir(&self.folder) else {
            return 0;
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(text) = name.to_str() else { continue };
            if Uuid::parse(text).is_none() || kept_folders.contains(&PathBuf::from(&name)) {
                continue;
            }
            let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
                continue;
            };
            if modified < cutoff && fs::remove_dir_all(entry.path()).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Moves the finished `partial` to `destination`, refusing with `AlreadyExists` when anything is there. A hard
    /// link fails on an existing name, so nothing is replaced; the partial file's name is removed after it.
    pub fn place(partial: &Path, destination: &Path) -> io::Result<()> {
        fs::hard_link(partial, destination)?;
        fs::remove_file(partial)
    }
}

/// Bytes as `1.25 GB`, in decimal gigabytes as file managers count free space.
fn gigabytes(bytes: i64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a size shown to two decimals")]
    let value = bytes.max(0) as f64 / 1e9;
    format!("{value:.2} GB")
}

/// Bytes as `1.25 GiB`, in binary gibibytes as the inbox's own limit is set.
fn gibibytes(bytes: i64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "a size shown to two decimals")]
    let value = bytes.max(0) as f64 / f64::from(1u32 << 30);
    format!("{value:.2} GiB")
}

/// `path` with `.` and `..` resolved by name, as a standardized file URL is.
fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other),
        }
    }
    result
}

/// Makes the folder `path` (its parent must exist), readable by this user only.
pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    let builder = {
        let mut builder = fs::DirBuilder::new();
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    builder.create(path)
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::AgentInbox;
    use std::{
        collections::HashSet,
        fs,
        time::{Duration, SystemTime},
    };

    fn upload(inbox: &AgentInbox, name: &str, age: Duration) -> String {
        let folder = inbox.make_upload_folder().expect("folder");
        let file = folder.join(name);
        fs::write(&file, "x").expect("write");
        fs::File::open(&folder)
            .expect("open")
            .set_modified(SystemTime::now() - age)
            .expect("date");
        file.to_string_lossy().into_owned()
    }

    #[test]
    fn only_old_files_no_job_reads_are_swept() {
        let temporary = tempfile::tempdir().expect("temporary folder");
        let inbox = AgentInbox::new(temporary.path().join("inbox"));
        let kept = upload(&inbox, "kept.mov", Duration::from_hours(25));
        let old = upload(&inbox, "old.mov", Duration::from_hours(25));
        let fresh = upload(&inbox, "fresh.mov", Duration::from_mins(1));
        let cutoff = SystemTime::now() - Duration::from_hours(24);
        assert_eq!(inbox.sweep(&HashSet::from([kept.clone()]), cutoff), 1);
        assert!(fs::metadata(&kept).is_ok());
        assert!(fs::metadata(&old).is_err());
        assert!(fs::metadata(&fresh).is_ok());
    }

    #[test]
    fn remove_takes_only_the_inboxs_own_upload_folders() {
        let temporary = tempfile::tempdir().expect("temporary folder");
        let inbox = AgentInbox::new(temporary.path().join("inbox"));
        let staged = upload(&inbox, "a.png", Duration::ZERO);
        let outside = temporary.path().join("outside.png");
        fs::write(&outside, "x").expect("write");
        let outside = outside.to_string_lossy().into_owned();
        inbox.remove(&[staged.clone(), outside.clone(), "/etc/hosts".to_owned()]);
        assert!(fs::metadata(&staged).is_err());
        assert!(fs::metadata(&outside).is_ok());
        let escaping = inbox.folder.join("x/../../y").to_string_lossy().into_owned();
        assert!(inbox.upload_folder(&escaping).is_none());
    }
}
