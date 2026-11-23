//! Asynchronous listeners for Yapper event envelopes.

mod registry;

pub use registry::{ErrorEvent, ErrorStage, EventRegistrationError};
pub(crate) use registry::EventRegistry;
