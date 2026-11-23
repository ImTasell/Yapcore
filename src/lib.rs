#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

//! A native asynchronous Rust framework for building bots on Yapper.
//!
//! The initial release focuses on the documented HTTPS API and long-poll
//! event transport. Undocumented gateway and voice-media protocols are not
//! assumed to be supported.

pub mod api;
pub mod bot;
pub mod cogs;
pub mod commands;
pub mod config;
pub mod error;
pub mod events;
pub mod ids;
pub mod interactions;
pub mod models;
pub mod webhooks;

pub use api::ApiClient;
pub use bot::{
    Bot, BotBuilder, PresenceRotationHandle, ServerPresenceMonitorHandle, ShutdownHandle,
};
pub use cogs::{Cog, CogRegistrationError};
pub use commands::{CommandContext, HandlerError, HandlerResult};
pub use config::{Config, ConfigBuilder, ConfigError, DEFAULT_API_BASE_URL};
pub use error::{ApiError, Error, Result};
pub use events::{ErrorEvent, ErrorStage, EventRegistrationError};
pub use ids::{
    CategoryId, ChannelId, EmojiId, IdError, InteractionId, MessageId, RoleId, ServerId, ThreadId,
    UserId, WebhookId,
};
pub use interactions::{InteractionContext, InteractionRegistrationError};

pub use models::{
    ActionRow, Activity, ActivityRotation, Ban, BotProfile, BotProfileEdit, BotStatus, Button,
    ButtonStyle, Channel, ChannelCategory, ChannelCreateOptions, ChannelEditOptions,
    ChannelListing, ChannelType, CommandDefinition, CommandDefinitionError,
    CommandOption, CommandOptionChoice, CommandOptionError, CommandOptionType,
    ComponentValidationError, ComponentV2ValidationError, Container, Embed, EmbedAuthor,
    EmbedBuilder, EmbedField, EmbedFooter, FileDisplay, LayoutComponent, LayoutValidationError,
    EmbedValidationError, EventBatch, EventEnvelope, ImageAsset, ImageFormat,
    InteractionResponseType, Member, Message, Modal, ModalValidationError, PermissionTarget,
    MediaGallery, MediaGalleryItem, ParsedCommandOptions, Presence, Role, RoleEditOptions,
    Section, SectionAccessory, Separator, SeparatorSpacing, Server, TextDisplay, TextInput,
    TextInputStyle, Thumbnail, COMPONENTS_V2_FLAG,
    UserProfile,
    VoiceChannelState,
};

pub use webhooks::{EventWebhook, WebhookVerificationError, verify_event_webhook};
