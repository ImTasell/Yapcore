#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

//! A native asynchronous Rust framework for building bots on Yapper.
//!
//! The initial release focuses on the documented HTTPS API and long-poll
//! event transport. Undocumented gateway and voice-media protocols are not
//! assumed to be supported.

pub mod api;
pub mod bot;
pub mod commands;
pub mod config;
pub mod error;
pub mod ids;
pub mod models;

pub use api::ApiClient;
pub use bot::{Bot, BotBuilder, ShutdownHandle};
pub use commands::{CommandContext, HandlerError, HandlerResult};
pub use config::{Config, ConfigBuilder, ConfigError, DEFAULT_API_BASE_URL};
pub use error::{ApiError, Error, Result};
pub use ids::{
    CategoryId, ChannelId, EmojiId, IdError, InteractionId, MessageId, RoleId, ServerId, ThreadId,
    UserId, WebhookId,
};
pub use models::{
    BotProfile, CommandDefinition, CommandDefinitionError, EventBatch, EventEnvelope,
    InteractionResponseType, Message,
};
