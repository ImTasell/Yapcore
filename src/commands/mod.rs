//! Command registration and handler contexts.

mod context;
mod registry;

pub use context::CommandContext;
pub(crate) use context::SlashCommandInvocation;
pub(crate) use registry::CommandRegistry;
pub use registry::{HandlerError, HandlerResult};
