//! The app side of agents (`JutsuStudio/Agents`, `Presentation/Agent*.swift`): the rules an agent's request goes
//! through, the settings that allow it, and the handler that answers the `jutsu` helper's requests.

pub mod activity;
pub mod budget;
pub mod element_lookup;
pub mod element_making;
pub mod enhance_order;
pub mod image_queue;
pub mod image_record;
pub mod job;
pub mod library_detail;
pub mod library_search;
pub mod preferences;
pub mod raising;
pub mod request_gate;
pub mod request_log;
pub mod runtime;
pub mod service;
pub mod settings;
pub mod video_order;

/// An app ID as the wire carries it: upper-case and hyphenated.
pub fn wire_id(id: uuid::Uuid) -> jutsu_agent::Uuid {
    id.into()
}

/// A wire ID as the app keeps it.
pub fn app_id(id: jutsu_agent::Uuid) -> uuid::Uuid {
    id.into()
}

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
pub(crate) mod test_fakes;
