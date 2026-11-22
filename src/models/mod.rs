//! Typed API response models.

mod bot;

pub use bot::BotProfile;

mod command;

pub use command::{CommandDefinition, CommandDefinitionError};
pub(crate) use command::validate_command_list;
