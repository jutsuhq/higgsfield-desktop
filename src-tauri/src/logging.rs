//! The log file in the app's log folder, with secrets masked by `jutsu_app::logging` before anything is written.

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Mutex;

use jutsu_app::logging::RedactingWriter;

const FILE_NAME: &str = "jutsu-studio.log";

/// Sends `tracing` to `<folder>/jutsu-studio.log`. Without a log file the app still runs; it logs to stderr.
pub(crate) fn start(folder: &Path) {
    let file = std::fs::create_dir_all(folder).and_then(|()| {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(folder.join(FILE_NAME))
    });
    let subscriber = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO);
    let installed = match file {
        Ok(file) => subscriber
            .with_writer(Mutex::new(RedactingWriter::new(file)))
            .try_init(),
        Err(_) => subscriber
            .with_writer(|| RedactingWriter::new(std::io::stderr()))
            .try_init(),
    };
    drop(installed);
}
