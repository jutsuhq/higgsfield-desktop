//! The Queue (`Presentation`'s queue files and `JutsuStudio/Queue`): what each row shows, the live jobs fed from the
//! engine, the "Didn't finish" section, the commands, and the engine's system ports on Windows and Linux.

pub mod batch_cancel;
pub mod format;
pub mod live;
pub mod ports;
pub mod row;
pub mod service;

pub use batch_cancel::BatchCancel;
pub use live::{JobBatch, LiveJobs, QueueEntry, QueueProblems};
pub use ports::{MissingFfmpeg, RouteConnectivity};
pub use row::{
    ColorRole, JobCommand, JobRowModel, QueueJobCommands, RowAction, RowNote, Stage, TimeEstimate,
};
pub use service::{
    DuplicateOutcome, JobPreview, JobProgress, QueueBatch, QueueConfirmation, QueueEntryModel, QueueRow,
    QueueService, QueueSnapshot,
};

#[cfg(test)]
pub(crate) mod fixture;
