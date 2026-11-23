//! Asynchronous listeners for Yapper event envelopes.

mod registry;

pub(crate) use registry::EventRegistry;
pub use registry::{ErrorEvent, ErrorStage, EventRegistrationError};
