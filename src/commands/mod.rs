//! Command registration and handler contexts.

mod context;
mod registry;

pub(crate) use context::SlashCommandInvocation;
pub use context::CommandContext;
pub(crate) use registry::CommandRegistry;
pub use registry::{HandlerError, HandlerResult};
