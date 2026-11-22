//! Asynchronous listeners for Yapper event envelopes.

mod registry;

pub use registry::EventRegistrationError;
pub(crate) use registry::EventRegistry;
