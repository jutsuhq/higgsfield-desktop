//! The folders inside the output folder.

use std::path::{Path, PathBuf};

use crate::support::string_enum;

string_enum! {
    /// The folders inside the output folder, one per kind of result, so each kind stays apart from the others and a
    /// new kind gets a folder of its own next to them.
    pub enum OutputSubfolder {
        /// Finished jobs: generations, finals and operations.
        Videos = "Videos",
        /// Images made with Generate… and character sheets (`SavedImage`).
        Images = "Images",
        /// Takes exported for editing and delivery sets an agent exported (`export`, `delivery_set`).
        Exports = "Exports",
    }
}

impl OutputSubfolder {
    /// This folder inside `output_folder`.
    pub fn path(self, output_folder: &Path) -> PathBuf {
        output_folder.join(self.raw_value())
    }
}
