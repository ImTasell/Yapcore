//! Typed API response models.

mod bot;

pub use bot::BotProfile;

mod command;

pub(crate) use command::validate_command_list;
pub use command::{CommandDefinition, CommandDefinitionError};
