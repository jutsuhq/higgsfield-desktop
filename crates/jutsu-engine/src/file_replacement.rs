//! Putting a finished file in place.

use std::fs;
use std::io;
use std::path::Path;

use jutsu_core::support::uuid_string;

/// Puts a finished file where the app or a save dialog named it, replacing a file already there.
pub struct FileReplacement;

impl FileReplacement {
    /// Renames `finished` over `target`. Across volumes, a complete copy is staged next to the target before it is
    /// renamed over the old file; a failed copy leaves both originals intact.
    pub fn move_file(finished: &Path, target: &Path) -> io::Result<()> {
        match fs::rename(finished, target) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() != io::ErrorKind::CrossesDevices => return Err(error),
            Err(_) => {}
        }
        let parent = target.parent().unwrap_or(Path::new("."));
        let folder = parent.join(format!(".replacement-{}", uuid_string(uuid::Uuid::new_v4())));
        fs::create_dir(&folder)?;
        let staged = folder.join("result");
        let result = fs::copy(finished, &staged).and_then(|_| fs::rename(&staged, target));
        let _ = fs::remove_dir_all(&folder);
        result?;
        let _ = fs::remove_file(finished);
        Ok(())
    }
}
