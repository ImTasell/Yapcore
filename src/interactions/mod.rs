//! Button and other interaction event contexts.

mod context;
mod registry;
mod session;

pub use context::InteractionContext;
pub(crate) use context::InteractionInvocation;
pub use registry::InteractionRegistrationError;
pub(crate) use registry::{ButtonHandler, ButtonRegistry};
pub(crate) use session::InteractionSession;
