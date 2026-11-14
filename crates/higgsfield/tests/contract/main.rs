//! Contract tests of the Higgsfield clients against stub hosts. No test reaches a real host.

// Test support outside `#[test]` functions: a failed setup is a failed test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod account;
mod api;
mod authorization;
mod catalog;
mod elements;
mod genjutsu;
mod images;
mod influencer;
mod library;
mod live_paid_run;
mod mcp_client;
mod models;
mod oauth_discovery;
mod support;
mod token_provider;
mod unlimited;
mod video_edits;
mod video_variants;
mod virality;
