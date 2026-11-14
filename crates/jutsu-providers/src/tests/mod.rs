//! The ports of the Swift `HiggsfieldProvidersTests`, one module per Swift file, against stub hosts. No test
//! reaches a real host.

#![allow(
    dead_code,
    reason = "the support is shared by every test module, and not each uses all of it"
)]

mod account_characters;
mod account_charges;
mod account_elements;
mod account_sign_in;
mod api_provider;
mod gemini_omni_flash;
mod genjutsu_lite;
mod history;
mod history_mapping;
mod image_provider;
mod intone;
mod kling4;
mod kling4_flash;
mod kling_o3_provider;
mod linked_input;
mod mcp_provider;
mod mcp_provider_constraints;
mod mcp_provider_genjutsu;
mod mcp_provider_quote;
mod mcp_provider_status_batch;
mod mcp_provider_unlimited;
mod mcp_provider_variants;
mod more_video_models;
mod native_draft_history;
mod native_draft_provider;
mod post_operation_provider;
mod support;
mod virality_provider;
