//! Command registration and handler contexts.

mod context;
mod registry;

pub use context::CommandContext;
pub use registry::{HandlerError, HandlerResult};
pub(crate) use registry::CommandRegistry;
