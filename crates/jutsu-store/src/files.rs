//! Atomic file writes: written aside, then renamed over the file.

use std::path::Path;

use uuid::Uuid;

/// Writes `contents` to `path` whole or not at all, creating its folder: a crash never leaves half a file.
pub(crate) async fn write_atomically(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let folder = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(folder).await?;
    let name = path
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let aside = folder.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let written = async {
        let mut file = tokio::fs::File::create(&aside).await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, contents).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&aside, path).await
    }
    .await;
    if written.is_err() {
        let _ = tokio::fs::remove_file(&aside).await;
    }
    written
}
