//! Typed API response models.

mod bot;

pub use bot::BotProfile;

mod component;

pub(crate) use component::validate_action_rows;
pub use component::{ActionRow, Button, ButtonStyle, ComponentValidationError};

mod command;

pub(crate) use command::validate_command_list;
pub use command::{CommandDefinition, CommandDefinitionError};

mod embed;

pub use embed::{Embed, EmbedAuthor, EmbedBuilder, EmbedField, EmbedFooter, EmbedValidationError};

mod event;

pub use event::{EventBatch, EventEnvelope};

mod message;

pub use message::Message;

mod interaction;

pub use interaction::InteractionResponseType;

mod server;

pub use server::{
    Channel, ChannelCategory, ChannelListing, ChannelType, Member, Role, Server, UserProfile,
};
