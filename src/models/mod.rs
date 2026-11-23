//! Typed API response models.

mod bot;

pub use bot::{
    Activity, ActivityRotation, BotProfile, BotProfileEdit, BotStatus, ImageAsset, ImageFormat,
    Presence,
};

mod component;
mod components_v2;

pub(crate) use component::{validate_action_rows, validate_component_rows};
pub use component::{
    ActionRow, Button, ButtonStyle, ComponentValidationError, MessageComponentRow,
    SelectActionRow, SelectMenu, SelectOption,
};
pub(crate) use components_v2::validate_layout;
pub use components_v2::{
    Container, FileDisplay, LayoutComponent, LayoutValidationError,
    MediaGallery, MediaGalleryItem, Section, SectionAccessory, Separator, SeparatorSpacing,
    TextDisplay, Thumbnail, COMPONENTS_V2_FLAG,
};

mod command;
mod command_options;

pub(crate) use command::validate_command_list;
pub use command::{CommandDefinition, CommandDefinitionError};
pub use command_options::{
    CommandOption, CommandOptionChoice, CommandOptionError, CommandOptionType,
    ParsedCommandOptions,
};

mod embed;

pub use embed::{Embed, EmbedAuthor, EmbedBuilder, EmbedField, EmbedFooter, EmbedValidationError};

mod event;

pub use event::{EventBatch, EventEnvelope};

mod message;

pub use message::Message;

mod interaction;

pub use interaction::{
    InteractionResponseType, Modal, ModalValidationError, TextInput, TextInputStyle,
};

mod server;

pub use server::{
    Ban, Channel, ChannelCategory, ChannelCreateOptions, ChannelEditOptions, ChannelListing,
    ChannelType, Member, PermissionTarget, Role, RoleEditOptions, Server, UserProfile,
    VoiceChannelState,
};
