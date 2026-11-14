//! Content hashes: SHA-256 streamed over a file (Swift's `ContentHasher`) and a cache of them by path and fingerprint
//! (`FileHashCache`).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
#[cfg(test)]
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bytes read at once: large videos never sit in memory whole.
pub(crate) const CHUNK_SIZE: usize = 1 << 20;

/// Hex SHA-256 of the file's contents, read a chunk at a time.
pub fn sha256_hex_of_file(path: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    read_chunks(path, |chunk| hasher.update(chunk))?;
    Ok(hex(&hasher.finalize()))
}

/// Hex SHA-256 of `text`'s UTF-8 bytes.
pub fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// Feeds the file to `use_chunk` a chunk at a time.
pub(crate) fn read_chunks(path: &Path, mut use_chunk: impl FnMut(&[u8])) -> std::io::Result<()> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        use_chunk(&buffer[..read]);
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Seconds between 1970 and 2001, Foundation's reference date: dates are stored as Swift's `JSONEncoder` writes them.
const REFERENCE_DATE: f64 = 978_307_200.0;

/// What tells one version of a file from another without reading it. A file replaced by another one gets a new
/// `resource_id` (device and inode) even when its size and modification date match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFingerprint {
    /// Bytes.
    pub size: i64,
    /// Seconds since 2001-01-01, as Foundation's `Date`.
    pub modification_date: f64,
    /// "device:inode" on Unix; on Windows the creation time, which a replacing file has its own of.
    #[serde(rename = "resourceID")]
    pub resource_id: String,
}

impl FileFingerprint {
    /// The fingerprint of the file at `path` as it is now.
    pub fn of_file(path: &Path) -> std::io::Result<Self> {
        let metadata = std::fs::metadata(path)?;
        let modified = metadata.modified()?;
        Ok(Self {
            size: metadata.len() as i64,
            modification_date: seconds_since_reference(modified),
            resource_id: resource_id(&metadata),
        })
    }
}

fn seconds_since_reference(time: SystemTime) -> f64 {
    let unix = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_secs_f64(),
        Err(before) => -before.duration().as_secs_f64(),
    };
    unix - REFERENCE_DATE
}

#[cfg(unix)]
fn resource_id(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt;
    format!("{}:{}", metadata.dev(), metadata.ino())
}

#[cfg(windows)]
fn resource_id(metadata: &std::fs::Metadata) -> String {
    use std::os::windows::fs::MetadataExt;
    format!("created:{}", metadata.creation_time())
}

#[cfg(not(any(unix, windows)))]
fn resource_id(metadata: &std::fs::Metadata) -> String {
    metadata
        .created()
        .map(|time| format!("created:{}", seconds_since_reference(time)))
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    fingerprint: FileFingerprint,
    hash: String,
    last_use: u64,
}

#[derive(Debug, Default)]
struct State {
    entries: Option<HashMap<String, Entry>>,
    /// Orders uses for forgetting; the wall clock can repeat and go back.
    last_use: u64,
}

/// Content hashes by path and `FileFingerprint`, so an unchanged file is read once: a changed size, modification date
/// or resource ID hashes it again. With a `store` the hashes outlive the app; the least recently used ones beyond
/// `capacity` are forgotten.
#[derive(Debug)]
pub struct FileHashCache {
    state: Mutex<State>,
    store: Option<PathBuf>,
    capacity: usize,
}

impl Default for FileHashCache {
    fn default() -> Self {
        Self::new(None)
    }
}

impl FileHashCache {
    /// How many hashes are kept by default.
    pub const DEFAULT_CAPACITY: usize = 512;

    /// A cache saved at `store` when given, keeping `DEFAULT_CAPACITY` hashes.
    pub fn new(store: Option<PathBuf>) -> Self {
        Self::with_capacity(store, Self::DEFAULT_CAPACITY)
    }

    /// A cache keeping at most `capacity` hashes.
    pub fn with_capacity(store: Option<PathBuf>, capacity: usize) -> Self {
        Self {
            state: Mutex::default(),
            store,
            capacity,
        }
    }

    /// The hash of the file at `path`, from the cache or else from `compute`, which reads the file. A file that
    /// changes while `compute` runs isn't cached.
    pub fn hash<E: From<std::io::Error>>(
        &self,
        path: &Path,
        compute: impl FnOnce(&Path) -> Result<String, E>,
    ) -> Result<String, E> {
        let key = std::path::absolute(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        let before = FileFingerprint::of_file(path)?;
        if let Some(cached) = self.cached(&key, &before) {
            return Ok(cached);
        }
        let hash = compute(path)?;
        if FileFingerprint::of_file(path).ok().as_ref() == Some(&before) {
            self.store(key, before, hash.clone());
        }
        Ok(hash)
    }

    fn cached(&self, key: &str, fingerprint: &FileFingerprint) -> Option<String> {
        let mut state = self.state.lock().ok()?;
        self.load(&mut state);
        let next_use = state.last_use + 1;
        let entry = state.entries.as_mut()?.get_mut(key)?;
        if entry.fingerprint != *fingerprint {
            return None;
        }
        entry.last_use = next_use;
        let hash = entry.hash.clone();
        state.last_use = next_use;
        Some(hash)
    }

    fn store(&self, key: String, fingerprint: FileFingerprint, hash: String) {
        let Ok(mut state) = self.state.lock() else { return };
        self.load(&mut state);
        state.last_use += 1;
        let last_use = state.last_use;
        let capacity = self.capacity;
        let Some(entries) = state.entries.as_mut() else {
            return;
        };
        entries.insert(
            key,
            Entry {
                fingerprint,
                hash,
                last_use,
            },
        );
        if entries.len() > capacity {
            let mut uses: Vec<(String, u64)> = entries
                .iter()
                .map(|(key, entry)| (key.clone(), entry.last_use))
                .collect();
            uses.sort_by_key(|(_, last_use)| *last_use);
            let excess = entries.len() - capacity;
            for (key, _) in uses.into_iter().take(excess) {
                entries.remove(&key);
            }
        }
        self.save(entries);
    }

    /// Reads the saved hashes the first time they are needed.
    fn load(&self, state: &mut State) {
        if state.entries.is_some() {
            return;
        }
        let loaded: HashMap<String, Entry> = self
            .store
            .as_ref()
            .and_then(|store| std::fs::read(store).ok())
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default();
        state.last_use = loaded.values().map(|entry| entry.last_use).max().unwrap_or(0);
        state.entries = Some(loaded);
    }

    /// Best effort: a hash that isn't saved is computed again next launch.
    fn save(&self, entries: &HashMap<String, Entry>) {
        let Some(store) = &self.store else { return };
        let Ok(data) = serde_json::to_vec(entries) else {
            return;
        };
        if let Some(folder) = store.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let partial = store.with_extension(format!("{}.partial", uuid::Uuid::new_v4()));
        if std::fs::write(&partial, data).is_ok() && std::fs::rename(&partial, store).is_err() {
            let _ = std::fs::remove_file(&partial);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::test_support::dir;

    /// `seconds` after 1970, for setting modification dates.
    fn unix_time(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn matches_known_digest() {
        let folder = dir();
        let url = folder.path().join("abc.txt");
        for (contents, expected) in [
            (
                b"abc".to_vec(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                vec![97_u8; 3 * 1024 * 1024 + 17],
                "8cb91458835f2fefeb12cf01e67a7ea18523a0567d4835a974dc04c2ddbb955b",
            ),
        ] {
            std::fs::write(&url, contents).unwrap();
            assert_eq!(sha256_hex_of_file(&url).unwrap(), expected);
        }
    }

    /// Hashes with a counter, so a test sees which calls read the file.
    #[derive(Default)]
    struct Hasher {
        reads: Cell<usize>,
    }

    impl Hasher {
        fn hash(&self, cache: &FileHashCache, path: &Path) -> String {
            cache
                .hash(path, |path| {
                    self.reads.set(self.reads.get() + 1);
                    std::fs::read_to_string(path)
                })
                .unwrap()
        }
    }

    fn file(folder: &Path, name: &str, text: &str) -> PathBuf {
        let path = folder.join(name);
        std::fs::write(&path, text).unwrap();
        set_modified(&path, unix_time(1_000_000));
        path
    }

    fn set_modified(path: &Path, time: SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    #[test]
    fn an_unchanged_file_is_read_once() {
        let folder = dir();
        let url = file(folder.path(), "a", "one");
        let cache = FileHashCache::default();
        let hasher = Hasher::default();

        assert_eq!(hasher.hash(&cache, &url), "one");
        assert_eq!(hasher.hash(&cache, &url), "one");
        assert_eq!(hasher.reads.get(), 1);
    }

    #[test]
    fn a_new_size_is_read_again() {
        let folder = dir();
        let url = file(folder.path(), "a", "one");
        let cache = FileHashCache::default();
        let hasher = Hasher::default();
        hasher.hash(&cache, &url);

        file(folder.path(), "a", "three");

        assert_eq!(hasher.hash(&cache, &url), "three");
        assert_eq!(hasher.reads.get(), 2);
    }

    #[test]
    fn a_new_modification_date_is_read_again() {
        let folder = dir();
        let url = file(folder.path(), "a", "one");
        let cache = FileHashCache::default();
        let hasher = Hasher::default();
        hasher.hash(&cache, &url);

        std::fs::write(&url, "two").unwrap();
        set_modified(&url, unix_time(2_000_000));

        assert_eq!(hasher.hash(&cache, &url), "two");
        assert_eq!(hasher.reads.get(), 2);
    }

    #[test]
    fn a_replaced_file_is_read_again_when_size_and_date_match() {
        let folder = dir();
        let url = file(folder.path(), "a", "one");
        let cache = FileHashCache::default();
        let hasher = Hasher::default();
        hasher.hash(&cache, &url);
        let original = FileFingerprint::of_file(&url).unwrap();

        let replacement = file(folder.path(), "b", "two");
        std::fs::remove_file(&url).unwrap();
        std::fs::rename(&replacement, &url).unwrap();
        let replaced = FileFingerprint::of_file(&url).unwrap();

        assert!(
            replaced.size == original.size
                && replaced
                    .modification_date
                    .total_cmp(&original.modification_date)
                    .is_eq()
        );
        assert_ne!(replaced.resource_id, original.resource_id);
        assert_eq!(hasher.hash(&cache, &url), "two");
        assert_eq!(hasher.reads.get(), 2);
    }

    #[test]
    fn a_file_changed_while_hashing_is_not_cached() {
        let folder = dir();
        let url = file(folder.path(), "a", "one");
        let cache = FileHashCache::default();
        let reads = Cell::new(0_u64);

        for _ in 0..2 {
            cache
                .hash(&url, |path| {
                    reads.set(reads.get() + 1);
                    set_modified(path, unix_time(1_000_000 + reads.get()));
                    Ok::<_, std::io::Error>("stale".to_owned())
                })
                .unwrap();
        }

        assert_eq!(reads.get(), 2);
    }

    #[test]
    fn saved_hashes_outlive_the_cache() {
        let folder = dir();
        let url = file(folder.path(), "a", "one");
        let store = folder.path().join("store/hashes.json");
        let hasher = Hasher::default();
        hasher.hash(&FileHashCache::new(Some(store.clone())), &url);

        assert_eq!(hasher.hash(&FileHashCache::new(Some(store.clone())), &url), "one");
        assert_eq!(hasher.reads.get(), 1);
        file(folder.path(), "a", "three");
        assert_eq!(hasher.hash(&FileHashCache::new(Some(store)), &url), "three");
        assert_eq!(hasher.reads.get(), 2);
    }

    #[test]
    fn the_least_recently_used_hash_is_forgotten_beyond_capacity() {
        let folder = dir();
        let first = file(folder.path(), "a", "one");
        let second = file(folder.path(), "b", "two");
        let third = file(folder.path(), "c", "three");
        let cache = FileHashCache::with_capacity(None, 2);
        let hasher = Hasher::default();
        hasher.hash(&cache, &first);
        hasher.hash(&cache, &second);
        hasher.hash(&cache, &first);

        hasher.hash(&cache, &third);
        hasher.hash(&cache, &first);
        assert_eq!(hasher.reads.get(), 3);
        hasher.hash(&cache, &second);
        assert_eq!(hasher.reads.get(), 4);
    }

    #[test]
    fn a_missing_file_fails() {
        let folder = dir();
        let cache = FileHashCache::default();
        let result = cache.hash(&folder.path().join("missing"), |_| {
            Ok::<_, std::io::Error>("never".into())
        });
        assert!(result.is_err());
    }
}
