//! Subtitles made on this computer from a video's speech: the runs, the speech model's download, what agents ask of
//! them, and what the sheet, editor and settings show.

pub mod agent;
pub mod models;
pub mod runs;
pub mod save;
pub mod service;
pub mod text;

#[cfg(test)]
mod tests;
