//! The JSON file of a log of work outside the job engine (`LogFile.swift`): `ImageSpendLog` and `ImageJobLog`, next
//! to the store.

use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// A log couldn't be written (or, for a financial log, read): what it holds is known to this launch only, until a
/// later write goes through.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
#[error("Jutsu Studio can't save {file}: {reason}")]
pub struct LogWriteError {
    /// The log's file name, e.g. "image-spend.json".
    pub file: String,
    /// Why, as the file system said it: a full disk, a folder that can't be written.
    pub reason: String,
}

/// A list of records in one JSON file. `None` keeps everything in memory, as tests do. Execution logs read
/// leniently: a file with unreadable records gives the ones that can be read and is moved aside, so the next write
/// doesn't erase the others. Financial callers use `read_strict`.
#[derive(Debug, Clone)]
pub struct LogFile<Record> {
    path: Option<PathBuf>,
    records: PhantomData<fn() -> Record>,
}

impl<Record: Serialize + DeserializeOwned> LogFile<Record> {
    pub fn new(folder: Option<&Path>, name: &str) -> Self {
        Self {
            path: folder.map(|folder| folder.join(name)),
            records: PhantomData,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn name(path: &Path) -> String {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The records that can be read; the file is moved aside when any can't.
    pub fn read(&self) -> Vec<Record> {
        let Some(path) = &self.path else {
            return Vec::new();
        };
        if !path.exists() {
            return Vec::new();
        }
        let values = std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice::<Vec<serde_json::Value>>(&data).ok());
        let Some(values) = values else {
            move_aside(path);
            return Vec::new();
        };
        let total = values.len();
        let records: Vec<Record> = values
            .into_iter()
            .filter_map(|value| serde_json::from_value(value).ok())
            .collect();
        if records.len() != total {
            move_aside(path);
        }
        records
    }

    /// Every record, or why the file can't be read; a missing file is empty. Financial records must be complete, so
    /// an unreadable file stays where it is.
    pub fn read_strict(&self) -> Result<Vec<Record>, LogWriteError> {
        let Some(path) = &self.path else {
            return Ok(Vec::new());
        };
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(LogWriteError {
                    file: Self::name(path),
                    reason: error.to_string(),
                });
            }
        };
        serde_json::from_slice(&data).map_err(|error| LogWriteError {
            file: Self::name(path),
            reason: format!("The data couldn't be read because it isn't in the correct format. ({error})"),
        })
    }

    /// Replaces the file in one step, keys sorted.
    pub fn write(&self, records: &[Record]) -> Result<(), LogWriteError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let failed = |error: &dyn std::fmt::Display| LogWriteError {
            file: Self::name(path),
            reason: error.to_string(),
        };
        // A `serde_json::Value` keeps its keys sorted.
        let value = serde_json::to_value(records).map_err(|error| failed(&error))?;
        let data = serde_json::to_vec(&value).map_err(|error| failed(&error))?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|error| failed(&error))?;
        }
        let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        std::fs::write(&temporary, data).map_err(|error| failed(&error))?;
        std::fs::rename(&temporary, path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            failed(&error)
        })
    }
}

/// `<name>.unreadable-<uuid>` next to the file (`RecordList.asideURL`).
fn move_aside(path: &Path) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let aside = path.with_file_name(format!(
        "{name}.unreadable-{}",
        Uuid::new_v4().to_string().to_uppercase()
    ));
    if let Err(error) = std::fs::rename(path, aside) {
        tracing::error!(%error, "can't move an unreadable log aside");
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Record {
        name: String,
        count: i64,
    }

    fn record(name: &str) -> Record {
        Record {
            name: name.to_owned(),
            count: 1,
        }
    }

    #[test]
    fn records_round_trip_with_sorted_keys() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let file = LogFile::<Record>::new(Some(folder.path()), "log.json");
        file.write(&[record("a")])
            .unwrap_or_else(|error| panic!("{error}"));
        let text = std::fs::read_to_string(folder.path().join("log.json")).unwrap_or_default();
        assert_eq!(text, r#"[{"count":1,"name":"a"}]"#);
        assert_eq!(file.read(), vec![record("a")]);
        assert_eq!(file.read_strict(), Ok(vec![record("a")]));
    }

    /// `LogFileTests`: a file with a record that can't be read gives the others and is kept aside.
    #[test]
    fn unreadable_records_are_skipped_and_the_file_kept_aside() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("log.json");
        std::fs::write(&path, r#"[{"name":"a","count":1},{"name":"b"}]"#)
            .unwrap_or_else(|error| panic!("{error}"));
        let file = LogFile::<Record>::new(Some(folder.path()), "log.json");
        assert_eq!(file.read(), vec![record("a")]);
        assert!(!path.exists());
        let aside: Vec<String> = std::fs::read_dir(folder.path())
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert!(aside.iter().any(|name| name.starts_with("log.json.unreadable-")));
    }

    #[test]
    fn a_strict_read_keeps_a_damaged_file_in_place() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let path = folder.path().join("log.json");
        std::fs::write(&path, "[{").unwrap_or_else(|error| panic!("{error}"));
        let file = LogFile::<Record>::new(Some(folder.path()), "log.json");
        let error = file.read_strict().err();
        assert_eq!(error.map(|error| error.file), Some("log.json".to_owned()));
        assert!(path.exists());
        assert_eq!(
            LogFile::<Record>::new(Some(&folder.path().join("missing")), "log.json").read_strict(),
            Ok(Vec::new())
        );
    }

    #[test]
    fn a_write_under_a_file_fails_with_its_name() {
        let folder = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let blocker = folder.path().join("file");
        std::fs::write(&blocker, "").unwrap_or_else(|error| panic!("{error}"));
        let file = LogFile::<Record>::new(Some(&blocker), "log.json");
        let error = file.write(&[record("a")]).err();
        assert_eq!(error.map(|error| error.file), Some("log.json".to_owned()));
    }
}
