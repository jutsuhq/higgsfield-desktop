//! File rules with no file access: a list read record by record, where an unreadable file is kept aside, and the
//! Finder-style unused name.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

use crate::support::url_ext::{deleting_path_extension, file_name, path_extension, sibling};

/// A JSON array of records decoded one by one: a record that can't be read, such as one written by a newer version,
/// is skipped instead of failing the whole list, and counted so the file holding it can be kept aside before it is
/// written over.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordList<Record> {
    /// The records that could be read, in order.
    pub records: Vec<Record>,
    /// How many records couldn't be read.
    pub skipped: usize,
}

impl<'de, Record: DeserializeOwned> Deserialize<'de> for RecordList<Record> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
        let mut list = Self {
            records: Vec::with_capacity(values.len()),
            skipped: 0,
        };
        for value in values {
            match serde_json::from_value(value) {
                Ok(record) => list.records.push(record),
                Err(_) => list.skipped += 1,
            }
        }
        Ok(list)
    }
}

impl<Record> RecordList<Record> {
    /// Where a file that couldn't be read in full is kept: next to it, under a name of its own, so a later write
    /// never replaces records it couldn't read.
    pub fn aside_path(path: &Path) -> PathBuf {
        let name = format!(
            "{}.unreadable-{}",
            file_name(path),
            crate::support::uuid_string(uuid::Uuid::new_v4())
        );
        sibling(path, &name)
    }
}

/// `path` if nothing is there, else the same name with " 2", " 3"… before the extension, as Finder does. `exists`
/// says whether a path is taken.
pub fn unused_file_path(path: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    if !exists(path) {
        return path.to_path_buf();
    }
    let name = file_name(path);
    let stem = deleting_path_extension(&name);
    let ext = path_extension(&name);
    (2_u64..u64::MAX)
        .map(|number| {
            let candidate = if ext.is_empty() {
                format!("{stem} {number}")
            } else {
                format!("{stem} {number}.{ext}")
            };
            sibling(path, &candidate)
        })
        .find(|candidate| !exists(candidate))
        .unwrap_or_else(|| path.to_path_buf())
}
