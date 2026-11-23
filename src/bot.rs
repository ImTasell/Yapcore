use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::task::JoinHandle;

use secrecy::SecretString;
use serde_json::Value;
use tokio::sync::watch;

macro_rules! log_info {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled { tracing::info!($($args)*); }
    };
}

macro_rules! log_warn {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled { tracing::warn!($($args)*); }
    };
}

macro_rules! log_error {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled { tracing::error!($($args)*); }
    };
}
use crate::api::ApiClient;
use crate::commands::{CommandContext, CommandRegistry, HandlerResult, SlashCommandInvocation};
use crate::config::{Config, ConfigBuilder, ConfigError};
use crate::error::{Error, Result};
use crate::events::{ErrorEvent, ErrorStage, EventRegistrationError, EventRegistry};
use crate::ids::{ChannelId, InteractionId, MessageId, UserId};
use crate::interactions::{
    ButtonRegistry, InteractionContext, InteractionInvocation, InteractionRegistrationError,
};
use crate::models::{
    Activity, ActivityRotation, BotProfile, BotProfileEdit, BotStatus, CommandDefinition,
    ImageAsset, ImageFormat, InteractionResponseType, Message, PermissionTarget, Presence,
    VoiceChannelState,
};

/// Handle used to request a graceful stop of a running bot.
#[derive(Debug, Clone)]
pub struct ShutdownHandle {
    sender: watch::Sender<bool>,
}

impl ShutdownHandle {
    /// Request that the bot stop after its current event handler completes.
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }
}

/// Handle for a running automatic activity rotation.
///
/// Dropping this handle requests cancellation. Call [`stop`](Self::stop) to
/// cancel the rotation and await its task result.
#[derive(Debug)]
pub struct PresenceRotationHandle {
    sender: watch::Sender<bool>,
    task: Option<JoinHandle<Result<()>>>,
}

impl PresenceRotationHandle {
    /// Request that the activity rotation stop promptly.
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }

    /// Return whether the rotation task has finished.
    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Cancel the rotation and wait for its task to finish.
    pub async fn stop(mut self) -> Result<()> {
        self.cancel();
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        task.await
            .map_err(|error| Error::PresenceRotationTaskFailed(error.to_string()))?
    }
}

impl Drop for PresenceRotationHandle {
    fn drop(&mut self) {
        self.sender.send_replace(true);
    }
}

/// Handle for a task that keeps the activity's server count up to date.
///
/// Dropping the handle requests cancellation. Use [`stop`](Self::stop) to
/// cancel the monitor and wait for its task to finish.
#[derive(Debug)]
pub struct ServerPresenceMonitorHandle {
    sender: watch::Sender<bool>,
    task: Option<JoinHandle<Result<()>>>,
}

impl ServerPresenceMonitorHandle {
    /// Request that the server-presence monitor stop promptly.
    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }

    /// Return whether the monitor task has finished.
    pub fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Cancel the monitor and wait for its task to finish.
    pub async fn stop(mut self) -> Result<()> {
        self.cancel();
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        task.await
            .map_err(|error| Error::ServerPresenceMonitorTaskFailed(error.to_string()))?
    }
}

impl Drop for ServerPresenceMonitorHandle {
    fn drop(&mut self) {
        self.sender.send_replace(true);
    }
}
/// Builder for a configured Yapper bot.
#[derive(Debug)]
pub struct BotBuilder {
    config: ConfigBuilder,
    prefixes: Vec<String>,
    ignore_bots: bool,
    logging: bool,
}

impl Default for BotBuilder {
    fn default() -> Self {
        Self {
            config: ConfigBuilder::default(),
            prefixes: vec!["/".to_owned(), "!".to_owned()],
            ignore_bots: true,
            logging: true,
        }
    }
}

impl BotBuilder {
    /// Create a bot builder with Yapcore's safe defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the bot token directly.
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.config = self.config.token(token);
        self
    }

    /// Read the bot token from an environment variable.
    pub fn token_from_env(mut self, variable: impl AsRef<str>) -> Result<Self, ConfigError> {
        self.config = self.config.token_from_env(variable)?;
        Ok(self)
    }

    /// Override the API base URL.
    pub fn api_base_url(mut self, url: impl Into<String>) -> Self {
        self.config = self.config.api_base_url(url);
        self
    }

    /// Set the timeout for each HTTP request.
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.config = self.config.request_timeout(timeout);
        self
    }

    /// Set the maximum response body size.
    pub fn max_response_bytes(mut self, limit: usize) -> Self {
        self.config = self.config.max_response_bytes(limit);
        self
    }

    /// Permit HTTP only for explicitly enabled loopback tests.
    pub fn allow_insecure_http_for_tests(mut self, allow: bool) -> Self {
        self.config = self.config.allow_insecure_http_for_tests(allow);
        self
    }

    /// Set message prefixes used to invoke local commands.
    ///
    /// Prefixes are checked longest-first. Empty values are ignored.
    pub fn prefixes<I, S>(mut self, prefixes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.prefixes = prefixes
            .into_iter()
            .map(Into::into)
            .filter(|prefix: &String| !prefix.is_empty())
            .collect();
        self.prefixes
            .sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        self.prefixes.dedup();
        self
    }

    /// Configure whether messages authored by bots are ignored.
    ///
    /// Defaults to true to prevent bot-to-bot command loops.
    pub fn ignore_bots(mut self, ignore: bool) -> Self {
        self.ignore_bots = ignore;
        self
    }

    /// Enable or disable Yapcore runtime logs (enabled by default).
    ///
    /// This controls lifecycle and handler logs emitted through tracing; it
    /// does not install or configure a global tracing subscriber.
    pub fn logging(mut self, enabled: bool) -> Self {
        self.logging = enabled;
        self
    }

    /// Validate configuration and create the bot's shared HTTP client.
    pub fn build(self) -> Result<Bot> {
        let config: Config = self.config.build()?;
        let api = ApiClient::new(config)?;
        let (shutdown, _) = watch::channel(false);

        Ok(Bot {
            api,
            commands: CommandRegistry::default(),
            buttons: ButtonRegistry::default(),
            events: EventRegistry::default(),
            prefixes: self.prefixes,
            ignore_bots: self.ignore_bots,
            logging: self.logging,
            cog_names: Vec::new(),
            registering_cog: None,
            last_event_id: Arc::new(AtomicU64::new(0)),
            shutdown,
        })
    }
}

/// Configured client and prefix-command runtime for a Yapper bot account.
#[derive(Debug)]
pub struct Bot {
    api: ApiClient,
    commands: CommandRegistry,
    buttons: ButtonRegistry,
    events: EventRegistry,
    prefixes: Vec<String>,
    ignore_bots: bool,
    logging: bool,
    cog_names: Vec<String>,
    registering_cog: Option<String>,
    last_event_id: Arc<AtomicU64>,
    shutdown: watch::Sender<bool>,
}

impl Bot {
    /// Create a bot builder.
    pub fn builder() -> BotBuilder {
        BotBuilder::new()
    }

    /// Access the authenticated REST API client.
    pub fn api_client(&self) -> &ApiClient {
        &self.api
    }

    /// Perform an authenticated request to a relative bot API route.
    pub async fn api(
        &self,
        method: reqwest::Method,
        route: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        self.api.api(method, route, body).await
    }

    /// Register a statically compiled cog as one atomic registration unit.
    ///
    /// A cog groups commands, event listeners and button handlers. If its setup
    /// fails, Yapcore restores the registries to their pre-registration state.
    /// Rust cogs are compiled code; dynamic source-file hot reload is intentionally
    /// not provided by this API.
    pub fn add_cog<C: crate::cogs::Cog>(
        &mut self,
        cog: C,
    ) -> std::result::Result<&mut Self, crate::cogs::CogRegistrationError> {
        let name = cog.name().to_owned();
        if name.is_empty() || name.len() > 64 {
            return Err(crate::cogs::CogRegistrationError::InvalidName);
        }
        if !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        }) {
            return Err(crate::cogs::CogRegistrationError::InvalidName);
        }
        if self.cog_names.contains(&name) {
            return Err(crate::cogs::CogRegistrationError::DuplicateName(name));
        }

        let commands = self.commands.clone();
        let events = self.events.clone();
        let buttons = self.buttons.clone();
        let cog_names = self.cog_names.clone();
        let previous_owner = self.registering_cog.replace(name.clone());
        let setup_result = cog.register(self);
        self.registering_cog = previous_owner;
        if let Err(reason) = setup_result {
            self.commands = commands;
            self.events = events;
            self.buttons = buttons;
            self.cog_names = cog_names;
            return Err(crate::cogs::CogRegistrationError::SetupFailed { name, reason });
        }

        self.cog_names.push(name);
        Ok(self)
    }

    /// Remove a cog and all of the handlers it owns.
    ///
    /// This is intended for setup-time/module management before `run` starts.
    /// It removes registrations, not compiled code or any other independently
    /// registered cog.
    pub fn remove_cog(&mut self, name: &str) -> bool {
        if !self.cog_names.iter().any(|registered| registered == name) {
            return false;
        }
        self.commands.remove_owner(name);
        self.events.remove_owner(name);
        self.buttons.remove_owner(name);
        self.cog_names.retain(|registered| registered != name);
        true
    }

    /// Replace one cog's registrations with those from another compiled instance.
    ///
    /// If registration fails, the previous cog and all its handlers are restored.
    /// This replaces registrations only; it does not dynamically reload Rust code.
    pub fn replace_cog<C: crate::cogs::Cog>(
        &mut self,
        cog: C,
    ) -> std::result::Result<&mut Self, crate::cogs::CogRegistrationError> {
        let name = cog.name().to_owned();
        if !self.cog_names.iter().any(|registered| registered == &name) {
            return self.add_cog(cog);
        }

        let commands = self.commands.clone();
        let events = self.events.clone();
        let buttons = self.buttons.clone();
        let cog_names = self.cog_names.clone();
        self.remove_cog(&name);

        if let Err(error) = self.add_cog(cog) {
            self.commands = commands;
            self.events = events;
            self.buttons = buttons;
            self.cog_names = cog_names;
            self.registering_cog = None;
            return Err(error);
        }

        Ok(self)
    }

    /// Names of cogs registered on this bot in registration order.
    pub fn cogs(&self) -> Vec<&str> {
        self.cog_names.iter().map(String::as_str).collect()
    }

    /// Return the last event cursor observed by the runtime.
    ///
    /// Persist this value externally and pass it to run_from on restart to
    /// resume from the event after the saved cursor.
    pub fn last_event_id(&self) -> u64 {
        self.last_event_id.load(Ordering::Acquire)
    }

    /// Create a handle that can cancel the bot's run loop.
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            sender: self.shutdown.clone(),
        }
    }

    /// Register a local prefix command.
    ///
    /// Handlers are asynchronous and receive an owned context. The future
    /// must return a boxed error so the framework can report failures without
    /// terminating the event loop.
    pub fn command<F, Fut>(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: F,
    ) -> std::result::Result<&mut Self, crate::CommandDefinitionError>
    where
        F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        if let Some(owner) = self.registering_cog.clone() {
            self.commands
                .register_owned(name, description, Some(owner), handler)?;
        } else {
            self.commands.register(name, description, handler)?;
        }
        Ok(self)
    }

    /// Register a command with a typed positional/named argument schema.
    ///
    /// Invalid arguments are rejected before the handler runs. Inside the
    /// handler, use `context.options()` to read parsed values.
    pub fn command_with_options<F, Fut>(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        options: Vec<crate::models::CommandOption>,
        handler: F,
    ) -> std::result::Result<&mut Self, crate::CommandDefinitionError>
    where
        F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        let schema = Arc::new(options);
        let handler = Arc::new(handler);
        self.command(name, description, move |context| {
            let schema = schema.clone();
            let handler = handler.clone();
            let original_context = context.clone();
            let is_interaction = context.is_interaction();
            let context = context.with_option_schema(&schema);
            async move {
                let context = match context {
                    Ok(context) => context,
                    Err(error) => {
                        let message = format!("Invalid arguments: {error}");
                        let response = if is_interaction {
                            original_context.ephemeral(message).await
                        } else {
                            original_context.reply(message).await
                        };
                        response.map_err(|error| -> crate::HandlerError {
                            Box::new(error)
                        })?;
                        return Ok(());
                    }
                };
                handler(context).await
            }
        })
    }

    /// Register a button handler for one exact component custom ID.
    /// Register a command that runs only when its asynchronous check returns true.
    ///
    /// A rejected check silently skips the handler. The check runs before every
    /// prefix or slash invocation.
    pub fn command_with_check<C, CFut, F, Fut>(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        check: C,
        handler: F,
    ) -> std::result::Result<&mut Self, crate::CommandDefinitionError>
    where
        C: Fn(CommandContext) -> CFut + Send + Sync + 'static,
        CFut: Future<Output = bool> + Send + 'static,
        F: Fn(CommandContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        if let Some(owner) = self.registering_cog.clone() {
            self.commands.register_with_check_owned(
                name,
                description,
                check,
                Some(owner),
                handler,
            )?;
        } else {
            self.commands
                .register_with_check(name, description, check, handler)?;
        }
        Ok(self)
    }

    /// Register a lifecycle error listener. Errors from error listeners are logged only.
    pub fn on_error<F, Fut>(&mut self, handler: F) -> &mut Self
    where
        F: Fn(ErrorEvent) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        self.events.register_error(handler);
        self
    }

    /// Register a button handler for one exact component custom ID.
    pub fn on_button<F, Fut>(
        &mut self,
        custom_id: impl Into<String>,
        handler: F,
    ) -> std::result::Result<&mut Self, InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        if let Some(owner) = self.registering_cog.clone() {
            self.buttons
                .register_exact_owned(custom_id, Some(owner), handler)?;
        } else {
            self.buttons.register_exact(custom_id, handler)?;
        }
        Ok(self)
    }

    /// Register a handler for a select-menu submission with an exact custom ID.
    ///
    /// Submitted selections are available through InteractionContext::values.
    pub fn on_select<F, Fut>(
        &mut self,
        custom_id: impl Into<String>,
        handler: F,
    ) -> std::result::Result<&mut Self, InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        let owner = self.registering_cog.clone();
        self.buttons
            .register_kind_owned("select", custom_id, owner, handler)?;
        Ok(self)
    }

    /// Register a handler for a modal submission with an exact custom ID.
    ///
    /// Submitted text/select fields are available through InteractionContext::values
    /// as a forward-compatible JSON value.
    pub fn on_modal<F, Fut>(
        &mut self,
        custom_id: impl Into<String>,
        handler: F,
    ) -> std::result::Result<&mut Self, InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        let owner = self.registering_cog.clone();
        self.buttons
            .register_kind_owned("modal_submit", custom_id, owner, handler)?;
        Ok(self)
    }

    /// Register a button handler for every custom ID beginning with prefix.
    pub fn button<F, Fut>(
        &mut self,
        prefix: impl Into<String>,
        handler: F,
    ) -> std::result::Result<&mut Self, InteractionRegistrationError>
    where
        F: Fn(InteractionContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        if let Some(owner) = self.registering_cog.clone() {
            self.buttons
                .register_prefix_owned(prefix, Some(owner), handler)?;
        } else {
            self.buttons.register_prefix(prefix, handler)?;
        }
        Ok(self)
    }

    /// Register an asynchronous listener for a Yapper event type.
    ///
    /// The handler receives the full event envelope, including unknown JSON
    /// fields. Multiple listeners for one event type run in registration order.
    pub fn on_event<F, Fut>(
        &mut self,
        event_type: impl Into<String>,
        handler: F,
    ) -> std::result::Result<&mut Self, EventRegistrationError>
    where
        F: Fn(crate::models::EventEnvelope) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        if let Some(owner) = self.registering_cog.clone() {
            self.events
                .register_owned(event_type, Some(owner), handler)?;
        } else {
            self.events.register(event_type, handler)?;
        }
        Ok(self)
    }

    /// Fetch recent channel messages in oldest-first order.
    pub async fn get_recent_messages(
        &self,
        channel_id: &ChannelId,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<Vec<Message>> {
        self.api
            .get_recent_messages(channel_id, limit, before)
            .await
    }

    /// Fetch messages newer than an ISO-8601 timestamp, oldest first.
    pub async fn get_messages_after(
        &self,
        channel_id: &ChannelId,
        after: &str,
    ) -> Result<Vec<Message>> {
        self.api.get_messages_after(channel_id, after).await
    }

    /// List servers the bot has been added to.
    pub async fn list_servers(&self) -> Result<Vec<crate::models::Server>> {
        self.api.list_servers().await
    }

    /// Fetch details for one server.
    pub async fn get_server(
        &self,
        server_id: &crate::ids::ServerId,
    ) -> Result<crate::models::Server> {
        self.api.get_server(server_id).await
    }

    /// List visible channels globally or within one server.
    pub async fn list_channels(
        &self,
        server_id: Option<&crate::ids::ServerId>,
        channel_type: Option<crate::models::ChannelType>,
    ) -> Result<crate::models::ChannelListing> {
        self.api.list_channels(server_id, channel_type).await
    }

    /// Fetch one visible channel.
    pub async fn get_channel(&self, channel_id: &ChannelId) -> Result<crate::models::Channel> {
        self.api.get_channel(channel_id).await
    }

    /// Create a text or voice channel in a server.
    pub async fn create_channel(
        &self,
        server_id: &crate::ids::ServerId,
        options: &crate::models::ChannelCreateOptions,
    ) -> Result<crate::models::Channel> {
        self.api.create_channel(server_id, options).await
    }

    /// Edit selected fields of a channel.
    pub async fn edit_channel(
        &self,
        channel_id: &ChannelId,
        changes: &crate::models::ChannelEditOptions,
    ) -> Result<crate::models::Channel> {
        self.api.edit_channel(channel_id, changes).await
    }

    /// Delete a channel.
    pub async fn delete_channel(&self, channel_id: &ChannelId) -> Result<()> {
        self.api.delete_channel(channel_id).await
    }

    /// Create a category in a server.
    pub async fn create_category(
        &self,
        server_id: &crate::ids::ServerId,
        name: &str,
    ) -> Result<crate::models::ChannelCategory> {
        self.api.create_category(server_id, name).await
    }

    /// Rename a category in a server.
    pub async fn rename_category(
        &self,
        server_id: &crate::ids::ServerId,
        category_id: &crate::ids::CategoryId,
        name: &str,
    ) -> Result<crate::models::ChannelCategory> {
        self.api.rename_category(server_id, category_id, name).await
    }

    /// Delete a category in a server.
    pub async fn delete_category(
        &self,
        server_id: &crate::ids::ServerId,
        category_id: &crate::ids::CategoryId,
    ) -> Result<()> {
        self.api.delete_category(server_id, category_id).await
    }

    /// List server members in user-ID order.
    pub async fn list_members(
        &self,
        server_id: &crate::ids::ServerId,
        limit: Option<usize>,
        after: Option<&UserId>,
    ) -> Result<Vec<crate::models::Member>> {
        self.api.list_members(server_id, limit, after).await
    }

    /// Fetch one server member.
    pub async fn get_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
    ) -> Result<crate::models::Member> {
        self.api.get_member(server_id, user_id).await
    }

    /// Add a role to a server member. Requires Manage Roles permission.
    pub async fn add_role(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
        role_id: &crate::ids::RoleId,
    ) -> Result<()> {
        self.api.add_role(server_id, user_id, role_id).await
    }

    /// Remove a role from a server member. Requires Manage Roles permission.
    pub async fn remove_role(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
        role_id: &crate::ids::RoleId,
    ) -> Result<()> {
        self.api.remove_role(server_id, user_id, role_id).await
    }

    /// Set a server member's nickname; an empty string clears it.
    pub async fn set_nickname(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
        nickname: &str,
    ) -> Result<serde_json::Value> {
        self.api.set_nickname(server_id, user_id, nickname).await
    }

    /// Kick a member from a server. Requires Kick Members permission.
    pub async fn kick_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
    ) -> Result<serde_json::Value> {
        self.api.kick_member(server_id, user_id).await
    }

    /// Ban a member from a server. Requires Ban Members permission.
    pub async fn ban_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
    ) -> Result<serde_json::Value> {
        self.api.ban_member(server_id, user_id).await
    }

    /// List bans in a server. Requires Ban Members permission.
    pub async fn list_bans(
        &self,
        server_id: &crate::ids::ServerId,
    ) -> Result<Vec<crate::models::Ban>> {
        self.api.list_bans(server_id).await
    }

    /// Remove a server ban. Requires Ban Members permission.
    pub async fn unban_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
    ) -> Result<()> {
        self.api.unban_member(server_id, user_id).await
    }

    /// Set a member timeout. Use zero minutes to clear the timeout.
    pub async fn timeout_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
        minutes: Option<u32>,
        reason: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.api
            .timeout_member(server_id, user_id, minutes, reason)
            .await
    }

    /// List roles in a server.
    pub async fn list_roles(
        &self,
        server_id: &crate::ids::ServerId,
    ) -> Result<Vec<crate::models::Role>> {
        self.api.list_roles(server_id).await
    }

    /// Fetch one role in a server.
    ///
    /// The source-only endpoint is retained for feature parity but should be
    /// verified against the current public API before depending on it.
    pub async fn get_role(
        &self,
        server_id: &crate::ids::ServerId,
        role_id: &crate::ids::RoleId,
    ) -> Result<crate::models::Role> {
        self.api.get_role(server_id, role_id).await
    }

    /// Create a role below the bot's highest role.
    pub async fn create_role(
        &self,
        server_id: &crate::ids::ServerId,
        name: &str,
        color: Option<&str>,
        mentionable: bool,
        permissions: Option<&serde_json::Value>,
    ) -> Result<crate::models::Role> {
        self.api
            .create_role(server_id, name, color, mentionable, permissions)
            .await
    }

    /// Edit selected fields of a role; None leaves a field unchanged.
    pub async fn edit_role(
        &self,
        server_id: &crate::ids::ServerId,
        role_id: &crate::ids::RoleId,
        changes: &crate::models::RoleEditOptions,
    ) -> Result<crate::models::Role> {
        self.api.edit_role(server_id, role_id, changes).await
    }

    /// Delete a role in a server.
    pub async fn delete_role(
        &self,
        server_id: &crate::ids::ServerId,
        role_id: &crate::ids::RoleId,
    ) -> Result<()> {
        self.api.delete_role(server_id, role_id).await
    }

    /// Reorder roles from highest to lowest using their IDs.
    pub async fn reorder_roles(
        &self,
        server_id: &crate::ids::ServerId,
        role_ids: &[crate::ids::RoleId],
    ) -> Result<Vec<crate::models::Role>> {
        self.api.reorder_roles(server_id, role_ids).await
    }

    /// Set the bot's online state and optional activity line.
    pub async fn set_status(&self, status: BotStatus, activity: Option<&str>) -> Result<Value> {
        self.api.set_status(status, activity).await
    }

    /// Set the bot's status and activity using the typed presence builder.
    ///
    /// This can be called while the bot is running; presence changes use the
    /// HTTP API and do not interrupt event polling.
    pub async fn set_presence(&self, presence: &Presence) -> Result<Value> {
        self.api.set_presence(presence).await
    }

    /// Rotate through a sequence of activities until cancelled or the bot shuts down.
    ///
    /// The first activity is applied before this method returns. Subsequent
    /// activities are applied at the configured interval. The rotation is
    /// cancelled when its handle is dropped, when `stop()` is called, or when
    /// the bot's [`ShutdownHandle`] is cancelled. API failures stop the
    /// rotation and are returned by `PresenceRotationHandle::stop`.
    pub async fn rotate_presence(
        &self,
        rotation: ActivityRotation,
    ) -> Result<PresenceRotationHandle> {
        let activities = rotation.activities().to_vec();
        let interval = rotation.interval();
        let status = rotation.bot_status();

        let initial_presence = Presence::new(status).activity(activities[0].clone());
        self.api.set_presence(&initial_presence).await?;

        let api = self.api.clone();
        let logging = self.logging;
        let (sender, mut stop_receiver) = watch::channel(false);
        let mut bot_shutdown = self.shutdown.subscribe();

        let task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Tokio intervals tick immediately once; consume that tick because
            // the first activity was already applied by the caller above.
            ticker.tick().await;
            let mut index = 0;

            loop {
                if *stop_receiver.borrow() || *bot_shutdown.borrow() {
                    return Ok(());
                }

                tokio::select! {
                    biased;
                    changed = stop_receiver.changed() => {
                        if changed.is_err() || *stop_receiver.borrow() {
                            return Ok(());
                        }
                    }
                    changed = bot_shutdown.changed() => {
                        if changed.is_err() || *bot_shutdown.borrow() {
                            return Ok(());
                        }
                    }
                    _ = ticker.tick() => {
                        index = (index + 1) % activities.len();
                        let presence = Presence::new(status).activity(activities[index].clone());
                        let result = tokio::select! {
                            biased;
                            changed = stop_receiver.changed() => {
                                if changed.is_err() || *stop_receiver.borrow() {
                                    return Ok(());
                                }
                                continue;
                            }
                            changed = bot_shutdown.changed() => {
                                if changed.is_err() || *bot_shutdown.borrow() {
                                    return Ok(());
                                }
                                continue;
                            }
                            result = api.set_presence(&presence) => result,
                        };

                        if let Err(error) = result {
                            log_error!(logging, error = %error, "activity rotation stopped after a presence update failed");
                            return Err(error);
                        }
                    }
                }
            }
        });

        Ok(PresenceRotationHandle {
            sender,
            task: Some(task),
        })
    }
    /// Fetch the bot's current server count and publish it as its activity.
    ///
    /// Returns the number of servers reported by Yapper. The activity is
    /// formatted as `Overviewing N servers` (singular when N is one).
    pub async fn refresh_server_count_presence(&self, status: BotStatus) -> Result<usize> {
        let servers = self.api.list_servers().await?;
        let count = servers.len();
        let activity = Activity::overviewing_servers(count)?;
        self.api
            .set_presence(&Presence::new(status).activity(activity))
            .await?;
        Ok(count)
    }

    /// Keep `Overviewing N servers` synchronized with the bot's server list.
    ///
    /// The current count is published immediately. The server list is then
    /// refreshed at the supplied interval, and presence is updated only when
    /// the count changes. Transient API failures are logged and retried on the
    /// next interval. The returned handle stops the monitor when dropped or
    /// cancelled; bot shutdown also stops it.
    pub async fn monitor_server_count_presence(
        &self,
        interval: Duration,
        status: BotStatus,
    ) -> Result<ServerPresenceMonitorHandle> {
        if interval.is_zero() {
            return Err(Error::InvalidServerPresenceMonitorInterval);
        }

        let mut last_count = self.refresh_server_count_presence(status).await?;
        let api = self.api.clone();
        let logging = self.logging;
        let (sender, mut stop_receiver) = watch::channel(false);
        let mut bot_shutdown = self.shutdown.subscribe();

        let task = tokio::spawn(async move {
            let first_tick = tokio::time::Instant::now() + interval;
            let mut ticker = tokio::time::interval_at(first_tick, interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                if *stop_receiver.borrow() || *bot_shutdown.borrow() {
                    return Ok(());
                }

                tokio::select! {
                    biased;
                    changed = stop_receiver.changed() => {
                        if changed.is_err() || *stop_receiver.borrow() {
                            return Ok(());
                        }
                    }
                    changed = bot_shutdown.changed() => {
                        if changed.is_err() || *bot_shutdown.borrow() {
                            return Ok(());
                        }
                    }
                    _ = ticker.tick() => {
                        let servers = tokio::select! {
                            biased;
                            changed = stop_receiver.changed() => {
                                if changed.is_err() || *stop_receiver.borrow() {
                                    return Ok(());
                                }
                                continue;
                            }
                            changed = bot_shutdown.changed() => {
                                if changed.is_err() || *bot_shutdown.borrow() {
                                    return Ok(());
                                }
                                continue;
                            }
                            result = api.list_servers() => match result {
                                Ok(servers) => servers,
                                Err(error) => {
                                    log_warn!(logging, error = %error, "could not refresh server count for bot presence");
                                    continue;
                                }
                            },
                        };

                        let count = servers.len();
                        if count == last_count {
                            continue;
                        }

                        let activity = Activity::overviewing_servers(count)?;
                        let presence = Presence::new(status).activity(activity);
                        let result = tokio::select! {
                            biased;
                            changed = stop_receiver.changed() => {
                                if changed.is_err() || *stop_receiver.borrow() {
                                    return Ok(());
                                }
                                continue;
                            }
                            changed = bot_shutdown.changed() => {
                                if changed.is_err() || *bot_shutdown.borrow() {
                                    return Ok(());
                                }
                                continue;
                            }
                            result = api.set_presence(&presence) => result,
                        };

                        match result {
                            Ok(_) => last_count = count,
                            Err(error) => log_warn!(logging, error = %error, "could not publish updated server count presence; will retry"),
                        }
                    }
                }
            }
        });

        Ok(ServerPresenceMonitorHandle {
            sender,
            task: Some(task),
        })
    }
    /// Read voice occupancy for each visible voice channel in a server.
    pub async fn get_voice_states(
        &self,
        server_id: &crate::ids::ServerId,
    ) -> Result<Vec<VoiceChannelState>> {
        self.api.get_voice_states(server_id).await
    }

    /// Move a member from their current voice channel to another channel.
    pub async fn move_member_to_voice(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
        channel_id: &ChannelId,
    ) -> Result<Value> {
        self.api
            .move_member_to_voice(server_id, user_id, channel_id)
            .await
    }

    /// Disconnect a member from voice.
    pub async fn disconnect_member_from_voice(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
    ) -> Result<Value> {
        self.api
            .disconnect_member_from_voice(server_id, user_id)
            .await
    }
    /// Fetch a member's effective server permissions (experimental API route).
    pub async fn get_member_permissions(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &UserId,
    ) -> Result<Value> {
        self.api.get_member_permissions(server_id, user_id).await
    }

    /// Fetch a member's effective permissions in one channel (experimental API route).
    pub async fn get_channel_member_permissions(
        &self,
        channel_id: &ChannelId,
        user_id: &UserId,
    ) -> Result<Value> {
        self.api
            .get_channel_member_permissions(channel_id, user_id)
            .await
    }

    /// List channel permission overrides (experimental API route).
    pub async fn list_channel_permission_overrides(&self, channel_id: &ChannelId) -> Result<Value> {
        self.api.list_channel_permission_overrides(channel_id).await
    }

    /// Set a channel permission override (experimental API route).
    pub async fn set_channel_permissions(
        &self,
        channel_id: &ChannelId,
        target: &PermissionTarget,
        allow: &[String],
        deny: &[String],
    ) -> Result<Value> {
        self.api
            .set_channel_permissions(channel_id, target, allow, deny)
            .await
    }

    /// Clear a channel permission override (experimental API route).
    pub async fn clear_channel_permissions(
        &self,
        channel_id: &ChannelId,
        target: &PermissionTarget,
    ) -> Result<()> {
        self.api.clear_channel_permissions(channel_id, target).await
    }

    /// Set server-wide @everyone permissions (experimental API route).
    pub async fn set_everyone_permissions(
        &self,
        server_id: &crate::ids::ServerId,
        permissions: &Value,
    ) -> Result<Value> {
        self.api
            .set_everyone_permissions(server_id, permissions)
            .await
    }

    /// Set a category permission override (experimental API route).
    pub async fn set_category_permissions(
        &self,
        server_id: &crate::ids::ServerId,
        category_id: &crate::ids::CategoryId,
        target: &PermissionTarget,
        allow: &[String],
        deny: &[String],
    ) -> Result<Value> {
        self.api
            .set_category_permissions(server_id, category_id, target, allow, deny)
            .await
    }

    /// Clear a category permission override (experimental API route).
    pub async fn clear_category_permissions(
        &self,
        server_id: &crate::ids::ServerId,
        category_id: &crate::ids::CategoryId,
        target: &PermissionTarget,
    ) -> Result<()> {
        self.api
            .clear_category_permissions(server_id, category_id, target)
            .await
    }

    /// Open or find a direct-message channel with a user.
    pub async fn open_dm(&self, user_id: &UserId) -> Result<ChannelId> {
        self.api.open_dm(user_id).await
    }

    /// Open a DM and send a plain-text message to it.
    pub async fn send_dm(&self, user_id: &UserId, content: impl AsRef<str>) -> Result<Message> {
        self.api.send_dm(user_id, content.as_ref()).await
    }
    /// Fetch one public user profile.
    pub async fn get_user(&self, user_id: &UserId) -> Result<crate::models::UserProfile> {
        self.api.get_user(user_id).await
    }

    /// Fetch one message from a channel.
    pub async fn get_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
    ) -> Result<Message> {
        self.api.get_message(channel_id, message_id).await
    }

    /// Edit the text content of a message.
    pub async fn edit_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        content: impl AsRef<str>,
    ) -> Result<Message> {
        self.api
            .edit_message(channel_id, message_id, content.as_ref())
            .await
    }

    /// Delete a message.
    pub async fn delete_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
    ) -> Result<()> {
        self.api.delete_message(channel_id, message_id).await
    }

    /// List reactions attached to a message.
    pub async fn get_reactions(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
    ) -> Result<serde_json::Value> {
        self.api.get_reactions(channel_id, message_id).await
    }

    /// Pin a message. Requires Manage Messages permission.
    pub async fn pin_message(&self, channel_id: &ChannelId, message_id: &MessageId) -> Result<()> {
        self.api.pin_message(channel_id, message_id).await
    }

    /// Unpin a message. Requires Manage Messages permission.
    pub async fn unpin_message(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
    ) -> Result<()> {
        self.api.unpin_message(channel_id, message_id).await
    }

    /// Bulk-delete between 1 and 100 messages in a channel.
    pub async fn bulk_delete_messages(
        &self,
        channel_id: &ChannelId,
        message_ids: &[MessageId],
    ) -> Result<serde_json::Value> {
        self.api.bulk_delete_messages(channel_id, message_ids).await
    }

    /// Add the bot's reaction to a message.
    pub async fn add_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: impl AsRef<str>,
    ) -> Result<()> {
        self.api
            .add_reaction(channel_id, message_id, emoji.as_ref())
            .await
    }

    /// Remove the bot's reaction from a message.
    pub async fn remove_reaction(
        &self,
        channel_id: &ChannelId,
        message_id: &MessageId,
        emoji: impl AsRef<str>,
    ) -> Result<()> {
        self.api
            .remove_reaction(channel_id, message_id, emoji.as_ref())
            .await
    }

    /// Fetch the commands currently registered with Yapper.
    pub async fn list_commands(&self) -> Result<Vec<CommandDefinition>> {
        self.api.list_commands().await
    }

    /// Return the currently registered command definitions in name order.
    ///
    /// This is useful for inspection and for synchronizing the registry once
    /// slash-command interaction handling is enabled.
    pub fn command_definitions(&self) -> Vec<CommandDefinition> {
        self.commands.definitions()
    }

    /// Fetch the current bot profile.
    pub async fn get_me(&self) -> Result<BotProfile> {
        self.api.get_me().await
    }

    /// Edit the bot's display name and/or bio.
    pub async fn edit_me(&self, changes: &BotProfileEdit) -> Result<BotProfile> {
        self.api.edit_me(changes).await
    }

    /// Upload a PNG, JPEG, GIF, or WebP avatar (maximum 8 MiB).
    pub async fn set_avatar(&self, bytes: &[u8], format: ImageFormat) -> Result<ImageAsset> {
        self.api.set_avatar(bytes, format).await
    }

    /// Upload a bot avatar using the multipart file field.
    pub async fn set_avatar_multipart(
        &self,
        bytes: &[u8],
        format: ImageFormat,
    ) -> Result<ImageAsset> {
        self.api.set_avatar_multipart(bytes, format).await
    }

    /// Remove the bot's avatar.
    pub async fn delete_avatar(&self) -> Result<()> {
        self.api.delete_avatar().await
    }

    /// Upload a PNG, JPEG, GIF, or WebP banner (maximum 10 MiB).
    pub async fn set_banner(&self, bytes: &[u8], format: ImageFormat) -> Result<ImageAsset> {
        self.api.set_banner(bytes, format).await
    }

    /// Upload a bot banner using the multipart file field.
    pub async fn set_banner_multipart(
        &self,
        bytes: &[u8],
        format: ImageFormat,
    ) -> Result<ImageAsset> {
        self.api.set_banner_multipart(bytes, format).await
    }

    /// Remove the bot's banner.
    pub async fn delete_banner(&self) -> Result<()> {
        self.api.delete_banner().await
    }
    /// Send a message with button action rows.
    pub async fn send_message_with_components(
        &self,
        channel_id: &ChannelId,
        content: Option<&str>,
        components: &[crate::models::ActionRow],
    ) -> Result<Message> {
        self.api
            .send_message_with_components(channel_id, content, components)
            .await
    }

    /// Send a message containing any supported combination of text, embeds and buttons.
    pub async fn send_rich_message(
        &self,
        channel_id: &ChannelId,
        content: Option<&str>,
        embeds: &[crate::models::Embed],
        components: &[crate::models::ActionRow],
    ) -> Result<Message> {
        self.api
            .send_rich_message(channel_id, content, embeds, components)
            .await
    }

    /// Send a message with optional text content and up to five embeds.
    pub async fn send_message_with_embeds(
        &self,
        channel_id: &ChannelId,
        content: Option<&str>,
        embeds: &[crate::models::Embed],
    ) -> Result<Message> {
        self.api
            .send_message_with_embeds(channel_id, content, embeds)
            .await
    }

    /// Send a plain-text message to a channel.
    pub async fn send_message(
        &self,
        channel_id: &ChannelId,
        content: impl AsRef<str>,
    ) -> Result<Message> {
        self.api.send_message(channel_id, content.as_ref()).await
    }

    /// Run the bot until the shutdown handle is cancelled.
    ///
    /// The first runtime milestone handles prefix commands in message events.
    /// Slash-command interactions, components and general event listeners are
    /// separate follow-up features.
    pub async fn run(&self) -> Result<()> {
        let result = self.run_inner(None).await;
        if let Err(error) = &result {
            self.report_error(ErrorEvent::new(ErrorStage::Runtime, error.to_string()))
                .await;
        }
        result.map(|_| ())
    }

    /// Resume event polling after an externally persisted event ID.
    ///
    /// Returns the last cursor observed when the bot shuts down. Use
    /// last_event_id() while running to checkpoint progress. Yapper keeps
    /// events for one day; anything older may already have been discarded.
    pub async fn run_from(&self, last_event_id: u64) -> Result<u64> {
        let result = self.run_inner(Some(last_event_id)).await;
        if let Err(error) = &result {
            self.report_error(ErrorEvent::new(ErrorStage::Runtime, error.to_string()))
                .await;
        }
        result
    }

    async fn run_inner(&self, saved_cursor: Option<u64>) -> Result<u64> {
        let profile = self.api.get_me().await?;
        let bot_id = profile.id.clone();

        // Yapper's PUT /commands replaces the full registry, so always sync
        // the complete set once before beginning event polling.
        self.api.sync_commands(&self.commands.definitions()).await?;

        log_info!(
            self.logging,
            username = profile.username.as_deref().unwrap_or("<unknown>"),
            "Yapcore bot connected"
        );

        let ready_event = crate::models::EventEnvelope {
            id: 0,
            event_type: "ready".to_owned(),
            data: serde_json::json!({ "bot": profile }),
            created_at: None,
        };
        self.dispatch_event(&ready_event, bot_id.as_ref()).await?;

        let mut cursor = match saved_cursor {
            Some(cursor) => cursor,
            None => self.api.poll_events(0, 0, None).await?.last(),
        };
        self.last_event_id.store(cursor, Ordering::Release);
        let mut shutdown = self.shutdown.subscribe();
        let mut consecutive_failures = 0_u32;

        loop {
            if *shutdown.borrow() {
                break;
            }
            let batch = tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                    continue;
                }
                result = self.api.poll_events(cursor, 25, None) => {
                    match result {
                        Ok(batch) => { consecutive_failures = 0; batch }
                        Err(error) => {
                            let Some(delay) = poll_retry_delay(&error, consecutive_failures) else {
                                return Err(error);
                            };
                            consecutive_failures = consecutive_failures.saturating_add(1);
                            tracing::warn!(
                                error = %error,
                                retry_seconds = delay.as_secs_f64(),
                                "event polling failed; retrying"
                            );
                            tokio::select! {
                                _ = tokio::time::sleep(delay) => {}
                                changed = shutdown.changed() => {
                                    if changed.is_err() || *shutdown.borrow() { break; }
                                }
                            }
                            continue;
                        }
                    }
                }
            };

            for event in batch.events() {
                if event.id <= cursor {
                    continue;
                }
                if let Err(error) = self.dispatch_event(event, bot_id.as_ref()).await {
                    let mut report = ErrorEvent::new(ErrorStage::EventDispatch, error.to_string());
                    report.event_id = Some(event.id);
                    report.event_type = Some(event.event_type.clone());
                    log_error!(
                        self.logging,
                        event_type = %event.event_type,
                        event_id = event.id,
                        error = %error,
                        "event dispatch failed"
                    );
                    self.report_error(report).await;
                }
                cursor = event.id;
                self.last_event_id.store(cursor, Ordering::Release);
                if *shutdown.borrow() {
                    break;
                }
            }
            cursor = cursor.max(batch.last());
            self.last_event_id.store(cursor, Ordering::Release);
        }
        Ok(cursor)
    }
    /// Alias for run.
    pub async fn start(&self) -> Result<()> {
        self.run().await
    }

    async fn report_error(&self, event: ErrorEvent) {
        for handler in self.events.error_handlers() {
            if let Err(error) = handler(event.clone()).await {
                log_error!(
                    self.logging,
                    error = %error,
                    "on_error handler returned an error"
                );
            }
        }
    }

    async fn dispatch_event(
        &self,
        event: &crate::models::EventEnvelope,
        bot_id: Option<&UserId>,
    ) -> Result<()> {
        if event.event_type == "message"
            && self.ignore_bots
            && event.data.get("author_bot").and_then(Value::as_bool) == Some(true)
        {
            return Ok(());
        }

        let built_in_result = match event.event_type.as_str() {
            "message" => self.dispatch_message_event(event, bot_id).await,
            "command" => self.dispatch_command_event(event).await,
            "interaction" => self.dispatch_interaction_event(event).await,
            _ => Ok(()),
        };

        for registered in self.events.handlers_for(&event.event_type) {
            if let Err(error) = (registered.handler)(event.clone()).await {
                let mut report = ErrorEvent::new(ErrorStage::EventListener, error.to_string());
                report.event_id = Some(event.id);
                report.event_type = Some(event.event_type.clone());
                report.cog = registered.owner.clone();
                log_error!(
                    self.logging,
                    event_type = %event.event_type,
                    event_id = event.id,
                    error = %error,
                    cog = registered.owner.as_deref().unwrap_or("<none>"),
                    "event listener failed"
                );
                self.report_error(report).await;
            }
        }

        built_in_result
    }

    async fn dispatch_message_event(
        &self,
        event: &crate::models::EventEnvelope,
        bot_id: Option<&UserId>,
    ) -> Result<()> {
        let data = &event.data;

        if self.ignore_bots && data.get("author_bot").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }

        if let (Some(bot_id), Some(targets)) =
            (bot_id, data.get("command_for").and_then(Value::as_array))
        {
            let addressed_to_this_bot = targets.iter().any(|target| {
                target
                    .as_str()
                    .is_some_and(|target| target == bot_id.as_str())
            });
            if !addressed_to_this_bot {
                return Ok(());
            }
        }

        let content = data
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let parsed = data
            .get("command")
            .and_then(Value::as_str)
            .map(|name| {
                (
                    name.to_ascii_lowercase(),
                    data.get("args")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                )
            })
            .or_else(|| self.parse_prefix_command(content));

        let Some((command, args)) = parsed else {
            return Ok(());
        };

        if !self.commands.contains(&command) {
            return Ok(());
        }

        let Some(message_id) = data.get("id").and_then(Value::as_str) else {
            tracing::warn!(event_id = event.id, "message event has no message ID");
            return Ok(());
        };
        let Some(channel_id) = data.get("channel_id").and_then(Value::as_str) else {
            tracing::warn!(event_id = event.id, "message event has no channel ID");
            return Ok(());
        };

        let message_id = match MessageId::new(message_id) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(event_id = event.id, %error, "message event has an invalid message ID");
                return Ok(());
            }
        };
        let channel_id = match ChannelId::new(channel_id) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(event_id = event.id, %error, "message event has an invalid channel ID");
                return Ok(());
            }
        };

        let author_id = data
            .get("author_id")
            .and_then(Value::as_str)
            .and_then(|id| UserId::new(id).ok());

        let context = CommandContext::new(
            self.api.clone(),
            command.clone(),
            content.to_owned(),
            args,
            message_id,
            channel_id,
            author_id,
        );

        if let Some(Err(error)) = self.commands.dispatch(&command, context).await {
            log_error!(self.logging, command = %command, error = %error, "command handler returned an error");
            let mut report = ErrorEvent::new(ErrorStage::CommandHandler, error.to_string());
            report.event_id = Some(event.id);
            report.event_type = Some(event.event_type.clone());
            report.cog = self.commands.owner(&command).map(str::to_owned);
            report.command = Some(command);
            self.report_error(report).await;
        }

        Ok(())
    }

    async fn dispatch_command_event(&self, event: &crate::models::EventEnvelope) -> Result<()> {
        let data = &event.data;

        let Some(interaction_id) = data.get("id").and_then(Value::as_str) else {
            tracing::warn!(event_id = event.id, "command event has no interaction ID");
            return Ok(());
        };
        let interaction_id = match InteractionId::new(interaction_id) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(event_id = event.id, %error, "command event has an invalid interaction ID");
                return Ok(());
            }
        };

        let Some(token) = data.get("token").and_then(Value::as_str) else {
            tracing::warn!(
                event_id = event.id,
                "command event has no interaction token"
            );
            return Ok(());
        };
        let token = SecretString::from(token.to_owned());

        let command = data
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if command.is_empty() {
            self.api
                .respond_interaction(
                    &interaction_id,
                    &token,
                    InteractionResponseType::Ephemeral,
                    Some("This command could not be recognized."),
                )
                .await?;
            return Ok(());
        }

        if !self.commands.contains(&command) {
            self.api
                .respond_interaction(
                    &interaction_id,
                    &token,
                    InteractionResponseType::Ephemeral,
                    Some("This command is not registered by this bot."),
                )
                .await?;
            return Ok(());
        }

        let Some(channel_id) = data.get("channel_id").and_then(Value::as_str) else {
            self.api
                .respond_interaction(
                    &interaction_id,
                    &token,
                    InteractionResponseType::Ephemeral,
                    Some("This command has no channel context."),
                )
                .await?;
            return Ok(());
        };
        let channel_id = match ChannelId::new(channel_id) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(event_id = event.id, %error, "command event has an invalid channel ID");
                self.api
                    .respond_interaction(
                        &interaction_id,
                        &token,
                        InteractionResponseType::Ephemeral,
                        Some("This command has an invalid channel context."),
                    )
                    .await?;
                return Ok(());
            }
        };

        let args = data
            .get("args")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let content = if args.is_empty() {
            format!("/{command}")
        } else {
            format!("/{command} {args}")
        };
        let author_id = data
            .get("user_id")
            .and_then(Value::as_str)
            .and_then(|id| UserId::new(id).ok());

        let context = CommandContext::new_interaction(
            self.api.clone(),
            SlashCommandInvocation {
                command: command.clone(),
                content,
                args,
                interaction_id,
                interaction_token: token,
                channel_id,
                author_id,
            },
        );
        let response_count = context
            .interaction_response_count()
            .expect("slash command context always has interaction state");
        let fallback_context = context.clone();

        let handler = self.commands.dispatch(&command, context);
        tokio::pin!(handler);

        // Leave a one-second safety margin before the platform's 3-second
        // interaction deadline. If the handler hasn't answered, acknowledge
        // now and let the handler continue; subsequent replies remain valid.
        let outcome = tokio::select! {
            result = &mut handler => Some(result),
            _ = tokio::time::sleep(Duration::from_secs(2)) => None,
        };

        match outcome {
            Some(Some(Ok(()))) => {
                if response_count.load(Ordering::Acquire) == 0 {
                    fallback_context.ack().await?;
                }
            }
            Some(Some(Err(error))) => {
                log_error!(self.logging, command = %command, error = %error, "slash command handler returned an error");
                let mut report =
                    ErrorEvent::new(ErrorStage::SlashCommandHandler, error.to_string());
                report.event_id = Some(event.id);
                report.event_type = Some(event.event_type.clone());
                report.cog = self.commands.owner(&command).map(str::to_owned);
                report.command = Some(command.clone());
                self.report_error(report).await;
                if response_count.load(Ordering::Acquire) == 0 {
                    fallback_context
                        .ephemeral(format!("Something went wrong running /{command}"))
                        .await?;
                }
            }
            Some(None) => {
                if response_count.load(Ordering::Acquire) == 0 {
                    fallback_context
                        .ephemeral("This command is no longer registered.")
                        .await?;
                }
            }
            None => {
                if response_count.load(Ordering::Acquire) == 0 {
                    fallback_context.ack().await?;
                }

                if let Some(Err(error)) = handler.await {
                    log_error!(self.logging, command = %command, error = %error, "slash command handler failed after automatic acknowledgement");
                    let mut report =
                        ErrorEvent::new(ErrorStage::SlashCommandAfterAck, error.to_string());
                    report.event_id = Some(event.id);
                    report.event_type = Some(event.event_type.clone());
                    report.cog = self.commands.owner(&command).map(str::to_owned);
                    report.command = Some(command.clone());
                    self.report_error(report).await;
                }
            }
        }

        Ok(())
    }

    async fn dispatch_interaction_event(&self, event: &crate::models::EventEnvelope) -> Result<()> {
        let data = &event.data;

        let Some(interaction_id) = data.get("id").and_then(Value::as_str) else {
            tracing::warn!(
                event_id = event.id,
                "interaction event has no interaction ID"
            );
            return Ok(());
        };
        let interaction_id = match InteractionId::new(interaction_id) {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!(event_id = event.id, %error, "interaction event has an invalid ID");
                return Ok(());
            }
        };

        let Some(token) = data.get("token").and_then(Value::as_str) else {
            tracing::warn!(
                event_id = event.id,
                "interaction event has no callback token"
            );
            return Ok(());
        };
        let Some(custom_id) = data.get("custom_id").and_then(Value::as_str) else {
            tracing::warn!(event_id = event.id, "interaction event has no custom ID");
            return Ok(());
        };

        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("button")
            .to_owned();

        if !matches!(kind.as_str(), "button" | "select" | "modal_submit") {
            return Ok(());
        }

        let handlers = self.buttons.handlers_for(&kind, custom_id);
        if handlers.is_empty() {
            return Ok(());
        }

        let error_stage = if kind == "button" {
            ErrorStage::ButtonHandler
        } else {
            ErrorStage::InteractionHandler
        };

        let invocation = InteractionInvocation {
            id: interaction_id,
            token: SecretString::from(token.to_owned()),
            kind,
            custom_id: custom_id.to_owned(),
            message_id: data
                .get("message_id")
                .and_then(Value::as_str)
                .and_then(|value| MessageId::new(value).ok()),
            channel_id: data
                .get("channel_id")
                .and_then(Value::as_str)
                .and_then(|value| ChannelId::new(value).ok()),
            user_id: data
                .get("user_id")
                .and_then(Value::as_str)
                .and_then(|value| UserId::new(value).ok()),
            username: data
                .get("username")
                .and_then(Value::as_str)
                .map(str::to_owned),
            values: data.get("values").cloned().unwrap_or(Value::Null),
        };
        let context = InteractionContext::new(self.api.clone(), invocation);
        let response_count = context.response_count();
        let fallback_context = context.clone();
        let custom_id_for_log = custom_id.to_owned();

        let dispatch = async {
            for registered in handlers {
                if let Err(error) = (registered.handler)(context.clone()).await {
                    log_error!(
                        self.logging,
                        custom_id = %custom_id_for_log,
                        error = %error,
                        "component interaction handler failed"
                    );
                    let mut report = ErrorEvent::new(error_stage, error.to_string());
                    report.event_id = Some(event.id);
                    report.event_type = Some(event.event_type.clone());
                    report.custom_id = Some(custom_id_for_log.clone());
                    report.cog = registered.owner.clone();
                    self.report_error(report).await;
                }
            }
        };
        tokio::pin!(dispatch);

        // Yapper requires the initial interaction response within three
        // seconds. Acknowledge after two seconds if handlers are still busy.
        let completed = tokio::select! {
            _ = &mut dispatch => true,
            _ = tokio::time::sleep(Duration::from_secs(2)) => false,
        };

        if !completed {
            if response_count.load(Ordering::Acquire) == 0 {
                fallback_context.ack().await?;
            }
            dispatch.await;
        } else if response_count.load(Ordering::Acquire) == 0 {
            fallback_context.ack().await?;
        }

        Ok(())
    }

    fn parse_prefix_command(&self, content: &str) -> Option<(String, String)> {
        for prefix in &self.prefixes {
            let Some(rest) = content.strip_prefix(prefix) else {
                continue;
            };

            let rest = rest.trim_start();
            let mut parts = rest.splitn(2, char::is_whitespace);
            let name = parts.next()?.to_ascii_lowercase();

            if name.is_empty()
                || name.len() > 32
                || !name.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                })
            {
                continue;
            }

            if self.commands.contains(&name) {
                let args = parts.next().unwrap_or_default().trim().to_owned();
                return Some((name, args));
            }
        }

        None
    }
}

fn poll_retry_delay(error: &Error, failures: u32) -> Option<Duration> {
    let backoff_seconds = 1_u64.checked_shl(failures.min(5)).unwrap_or(30).min(30);
    let backoff = Duration::from_secs(backoff_seconds);

    match error {
        Error::Transport(_) => Some(backoff),
        Error::Api(api_error) => match api_error.status().as_u16() {
            429 | 503 => api_error.retry_after(),
            502 | 504 => Some(backoff),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_requires_a_bot_token() {
        let error = Bot::builder().build().unwrap_err();
        assert!(matches!(
            error,
            crate::Error::Configuration(ConfigError::MissingToken)
        ));
    }

    #[test]
    fn matches_longest_registered_prefix() {
        let mut bot = Bot::builder()
            .token("yb_test-token")
            .prefixes(["!", "!!"])
            .build()
            .unwrap();

        bot.command("ping", "Check whether the bot is alive", |_context| async {
            Ok(())
        })
        .unwrap();

        assert_eq!(
            bot.parse_prefix_command("!!ping hello there"),
            Some(("ping".to_owned(), "hello there".to_owned()))
        );
        assert_eq!(bot.parse_prefix_command("!unknown"), None);
    }

    #[test]
    fn ignores_bot_messages_by_default() {
        let bot = Bot::builder().token("yb_test-token").build().unwrap();
        assert!(bot.ignore_bots);
    }

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Receiver};
    use std::thread::{self, JoinHandle};

    struct CallbackServer {
        uri: String,
        request: Receiver<String>,
        thread: Option<JoinHandle<()>>,
    }

    impl CallbackServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let uri = format!("http://{address}");
            let (sender, request) = mpsc::channel();

            let thread = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();

                let mut request_bytes = Vec::new();
                let mut buffer = [0_u8; 1024];
                let mut expected_length = None;

                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    request_bytes.extend_from_slice(&buffer[..read]);

                    if expected_length.is_none() {
                        if let Some(header_end) = request_bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                        {
                            let body_start = header_end + 4;
                            let headers = String::from_utf8_lossy(&request_bytes[..header_end]);
                            let content_length = headers
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            expected_length = Some(body_start + content_length);
                        }
                    }

                    if expected_length.is_some_and(|length| request_bytes.len() >= length) {
                        break;
                    }
                }

                let request_text = String::from_utf8_lossy(&request_bytes).into_owned();
                let _ = sender.send(request_text);

                let response_body = "{}";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                    response_body.len(),
                    response_body
                );
                stream.write_all(response.as_bytes()).unwrap();
            });

            Self {
                uri,
                request,
                thread: Some(thread),
            }
        }

        fn uri(&self) -> &str {
            &self.uri
        }

        fn finish(mut self) -> String {
            self.thread.take().unwrap().join().unwrap();
            self.request.recv().unwrap()
        }
    }

    fn test_bot(server: &CallbackServer) -> Bot {
        Bot::builder()
            .token("yb_test-token")
            .api_base_url(format!("{}/api/bot/v1", server.uri()))
            .allow_insecure_http_for_tests(true)
            .build()
            .unwrap()
    }

    fn slash_command_event() -> crate::models::EventEnvelope {
        crate::models::EventEnvelope {
            id: 7,
            event_type: "command".to_owned(),
            data: serde_json::json!({
                "id": "123e4567-e89b-12d3-a456-426614174010",
                "token": "slash/token",
                "command": "ping",
                "args": "hello there",
                "user_id": "123e4567-e89b-12d3-a456-426614174003",
                "username": "test-user",
                "channel_id": "123e4567-e89b-12d3-a456-426614174002"
            }),
            created_at: None,
        }
    }

    fn button_interaction_event(custom_id: &str) -> crate::models::EventEnvelope {
        crate::models::EventEnvelope {
            id: 8,
            event_type: "interaction".to_owned(),
            data: serde_json::json!({
                "id": "123e4567-e89b-12d3-a456-426614174011",
                "token": "button/token",
                "type": "button",
                "custom_id": custom_id,
                "message_id": "123e4567-e89b-12d3-a456-426614174001",
                "channel_id": "123e4567-e89b-12d3-a456-426614174002",
                "user_id": "123e4567-e89b-12d3-a456-426614174003",
                "username": "test-user"
            }),
            created_at: None,
        }
    }

    #[tokio::test]
    async fn prefix_button_handler_updates_the_source_message() {
        let server = CallbackServer::start();
        let mut bot = test_bot(&server);

        bot.button("vote:", |interaction| async move {
            assert_eq!(interaction.custom_id(), "vote:yes");
            assert_eq!(interaction.kind(), "button");
            assert_eq!(interaction.username(), Some("test-user"));
            interaction
                .update("Vote recorded")
                .await
                .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
            Ok(())
        })
        .unwrap();

        bot.dispatch_event(&button_interaction_event("vote:yes"), None)
            .await
            .unwrap();

        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/interactions/123e4567-e89b-12d3-a456-426614174011/button%2Ftoken/callback HTTP/1.1"
        ));

        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["type"], "update");
        assert_eq!(payload["content"], "Vote recorded");
    }

    #[tokio::test]
    async fn button_handler_without_response_is_automatically_acknowledged() {
        let server = CallbackServer::start();
        let mut bot = test_bot(&server);

        bot.on_button("confirm:yes", |_interaction| async { Ok(()) })
            .unwrap();

        bot.dispatch_event(&button_interaction_event("confirm:yes"), None)
            .await
            .unwrap();

        let request = server.finish();
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["type"], "ack");
    }

    #[tokio::test]
    async fn button_handler_can_open_a_typed_modal() {
        let server = CallbackServer::start();
        let mut bot = test_bot(&server);

        bot.on_button("apply", |context| async move {
            let modal = crate::models::Modal::new(
                "application",
                "Apply",
                [crate::models::TextInput::paragraph("why", "Why?")?],
            )
            .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
            context
                .show_modal(&modal)
                .await
                .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
            Ok(())
        })
        .unwrap();

        bot.dispatch_event(&button_interaction_event("apply"), None)
            .await
            .unwrap();

        let request = server.finish();
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["type"], "modal");
        assert_eq!(payload["modal"]["custom_id"], "application");
        assert_eq!(payload["modal"]["components"][0]["custom_id"], "why");
        assert_eq!(payload["modal"]["components"][0]["type"], "text_input");
    }

    #[tokio::test]
    async fn dispatches_select_submission_with_selected_values() {
        use std::sync::{Arc, Mutex};

        let server = CallbackServer::start();
        let mut bot = test_bot(&server);
        let observed = Arc::new(Mutex::new(None));
        let observed_by_handler = observed.clone();

        bot.on_select("food", move |context| {
            let observed = observed_by_handler.clone();
            async move {
                *observed.lock().unwrap() = Some(context.values().clone());
                context
                    .reply("Selection received")
                    .await
                    .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
                Ok(())
            }
        })
        .unwrap();

        let mut event = button_interaction_event("food");
        event.data["type"] = serde_json::json!("select");
        event.data["values"] = serde_json::json!(["pizza", "taco"]);
        bot.dispatch_event(&event, None).await.unwrap();

        let request = server.finish();
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["type"], "reply");
        assert_eq!(
            *observed.lock().unwrap(),
            Some(serde_json::json!(["pizza", "taco"]))
        );
    }

    #[tokio::test]
    async fn dispatches_modal_submission_with_field_values() {
        use std::sync::{Arc, Mutex};

        let server = CallbackServer::start();
        let mut bot = test_bot(&server);
        let observed = Arc::new(Mutex::new(None));
        let observed_by_handler = observed.clone();

        bot.on_modal("application", move |context| {
            let observed = observed_by_handler.clone();
            async move {
                *observed.lock().unwrap() = Some(context.values().clone());
                context
                    .reply("Application received")
                    .await
                    .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
                Ok(())
            }
        })
        .unwrap();

        let mut event = button_interaction_event("application");
        event.data["type"] = serde_json::json!("modal_submit");
        event.data["values"] = serde_json::json!({
            "why": "I enjoy building things",
            "name": "Altix"
        });
        bot.dispatch_event(&event, None).await.unwrap();

        let request = server.finish();
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["type"], "reply");
        assert_eq!(
            *observed.lock().unwrap(),
            Some(serde_json::json!({
                "why": "I enjoy building things",
                "name": "Altix"
            }))
        );
    }

    #[tokio::test]
    async fn slash_command_handler_can_open_a_typed_modal() {
        let server = CallbackServer::start();
        let mut bot = test_bot(&server);

        bot.command("apply", "Open an application form", |context| async move {
            let modal = crate::models::Modal::new(
                "application",
                "Apply",
                [crate::models::TextInput::short("name", "Your name")?],
            )
            .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
            context
                .show_modal(&modal)
                .await
                .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
            Ok(())
        })
        .unwrap();

        let mut event = slash_command_event();
        event.data["command"] = serde_json::json!("apply");
        event.data["args"] = serde_json::json!("");
        bot.dispatch_event(&event, None).await.unwrap();

        let request = server.finish();
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["type"], "modal");
        assert_eq!(payload["modal"]["custom_id"], "application");
    }

    #[tokio::test]
    async fn dispatches_slash_command_reply_through_interaction_callback() {
        let server = CallbackServer::start();
        let mut bot = test_bot(&server);

        bot.command(
            "ping",
            "Check whether the bot is alive",
            |context| async move {
                assert!(context.is_interaction());
                assert_eq!(context.command(), "ping");
                assert_eq!(context.args(), "hello there");
                context
                    .reply("Pong from slash!")
                    .await
                    .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
                Ok(())
            },
        )
        .unwrap();

        bot.dispatch_event(&slash_command_event(), None)
            .await
            .unwrap();

        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/interactions/123e4567-e89b-12d3-a456-426614174010/slash%2Ftoken/callback HTTP/1.1"
        ));

        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["type"], "reply");
        assert_eq!(payload["content"], "Pong from slash!");
    }

    #[tokio::test]
    async fn automatically_acknowledges_slash_command_handlers_without_a_response() {
        let server = CallbackServer::start();
        let mut bot = test_bot(&server);

        bot.command("ping", "Check whether the bot is alive", |_context| async {
            Ok(())
        })
        .unwrap();

        bot.dispatch_event(&slash_command_event(), None)
            .await
            .unwrap();

        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/interactions/123e4567-e89b-12d3-a456-426614174010/slash%2Ftoken/callback HTTP/1.1"
        ));

        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["type"], "ack");
    }

    #[tokio::test]
    async fn dispatches_prefix_message_to_registered_async_handler() {
        use std::sync::{Arc, Mutex};

        let mut bot = Bot::builder()
            .token("yb_test-token")
            .prefixes(["!", "!!"])
            .build()
            .unwrap();

        let received = Arc::new(Mutex::new(None));
        let received_by_handler = received.clone();

        bot.command("ping", "Check whether the bot is alive", move |context| {
            let received = received_by_handler.clone();
            async move {
                *received.lock().unwrap() = Some((
                    context.command().to_owned(),
                    context.args().to_owned(),
                    context.argv().to_vec(),
                ));
                Ok(())
            }
        })
        .unwrap();

        let event = crate::models::EventEnvelope {
            id: 1,
            event_type: "message".to_owned(),
            data: serde_json::json!({
                "id": "123e4567-e89b-12d3-a456-426614174001",
                "channel_id": "123e4567-e89b-12d3-a456-426614174002",
                "author_id": "123e4567-e89b-12d3-a456-426614174003",
                "author_bot": false,
                "content": "!!ping hello there"
            }),
            created_at: None,
        };

        bot.dispatch_event(&event, None).await.unwrap();

        assert_eq!(
            *received.lock().unwrap(),
            Some((
                "ping".to_owned(),
                "hello there".to_owned(),
                vec!["hello".to_owned(), "there".to_owned()]
            ))
        );
    }
    #[tokio::test]
    async fn typed_command_registration_parses_values_before_handler() {
        use std::sync::{Arc, Mutex};

        let mut bot = Bot::builder()
            .token("yb_test-token")
            .prefixes(["!"])
            .build()
            .unwrap();
        let observed = Arc::new(Mutex::new(None));
        let observed_by_handler = observed.clone();

        bot.command_with_options(
            "profile",
            "Update a profile",
            vec![
                crate::models::CommandOption::string("name", "Display name")
                    .unwrap()
                    .required(true),
                crate::models::CommandOption::integer("age", "Age")
                    .unwrap(),
                crate::models::CommandOption::boolean("public", "Public profile")
                    .unwrap(),
            ],
            move |context| {
                let observed = observed_by_handler.clone();
                async move {
                    let options = context.options().expect("typed options are installed");
                    *observed.lock().unwrap() = Some((
                        options.get_string("name").unwrap().to_owned(),
                        options.get_integer("age"),
                        options.get_boolean("public"),
                    ));
                    Ok(())
                }
            },
        )
        .unwrap();

        let event = crate::models::EventEnvelope {
            id: 2,
            event_type: "message".to_owned(),
            data: serde_json::json!({
                "id": "123e4567-e89b-12d3-a456-426614174001",
                "channel_id": "123e4567-e89b-12d3-a456-426614174002",
                "author_id": "123e4567-e89b-12d3-a456-426614174003",
                "author_bot": false,
                "content": r#"!profile name="Altix Dev" age=24 public=true"#
            }),
            created_at: None,
        };

        bot.dispatch_event(&event, None).await.unwrap();
        assert_eq!(
            *observed.lock().unwrap(),
            Some(("Altix Dev".to_owned(), Some(24), Some(true)))
        );
    }

    #[tokio::test]
    async fn invalid_typed_command_arguments_skip_handler() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let server = CallbackServer::start();
        let mut bot = test_bot(&server);
        let invoked = Arc::new(AtomicBool::new(false));
        let invoked_by_handler = invoked.clone();

        bot.command_with_options(
            "count",
            "Set a count",
            vec![crate::models::CommandOption::integer("value", "Count")
                .unwrap()
                .required(true)],
            move |_context| {
                let invoked = invoked_by_handler.clone();
                async move {
                    invoked.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
        .unwrap();

        let event = crate::models::EventEnvelope {
            id: 3,
            event_type: "message".to_owned(),
            data: serde_json::json!({
                "id": "123e4567-e89b-12d3-a456-426614174001",
                "channel_id": "123e4567-e89b-12d3-a456-426614174002",
                "author_id": "123e4567-e89b-12d3-a456-426614174003",
                "author_bot": false,
                "content": "!count value=not-a-number"
            }),
            created_at: None,
        };

        bot.dispatch_event(&event, None).await.unwrap();
        assert!(!invoked.load(Ordering::SeqCst));

        let request = server.finish();
        let body = request.split_once("\r\n\r\n").unwrap().1;
        assert!(body.contains("Invalid arguments"));
        assert!(body.contains("value"));
    }

    #[tokio::test]
    async fn dispatches_general_event_listeners_in_order_and_isolates_failures() {
        use std::sync::{Arc, Mutex};

        let mut bot = Bot::builder().token("yb_test-token").build().unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));

        bot.on_event("member_join", |_event| async {
            Err(Box::new(std::io::Error::other("listener failed")) as crate::HandlerError)
        })
        .unwrap();

        let observed_by_handler = observed.clone();
        bot.on_event("member_join", move |event| {
            let observed = observed_by_handler.clone();
            async move {
                observed
                    .lock()
                    .unwrap()
                    .push(event.data["username"].as_str().unwrap().to_owned());
                Ok(())
            }
        })
        .unwrap();

        let event = crate::models::EventEnvelope {
            id: 25,
            event_type: "member_join".to_owned(),
            data: serde_json::json!({
                "user_id": "123e4567-e89b-12d3-a456-426614174003",
                "username": "new-member",
                "future_field": { "preserved": true }
            }),
            created_at: None,
        };

        bot.dispatch_event(&event, None).await.unwrap();

        assert_eq!(*observed.lock().unwrap(), ["new-member"]);
    }
}

#[cfg(test)]
mod presence_rotation_tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Notify;

    #[tokio::test]
    async fn dropping_rotation_handle_requests_cancellation() {
        let (sender, mut receiver) = watch::channel(false);
        let notified = Arc::new(Notify::new());
        let task_notified = Arc::clone(&notified);
        let task = tokio::spawn(async move {
            let _ = receiver.changed().await;
            task_notified.notify_one();
            Ok(())
        });
        let handle = PresenceRotationHandle {
            sender,
            task: Some(task),
        };

        drop(handle);
        tokio::time::timeout(Duration::from_secs(1), notified.notified())
            .await
            .expect("dropping the handle should wake the rotation task");
    }
    #[tokio::test]
    async fn server_presence_monitor_rejects_zero_interval_before_networking() {
        let bot = Bot::builder().token("yb_test-token").build().unwrap();
        let error = bot
            .monitor_server_count_presence(Duration::ZERO, BotStatus::Online)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::InvalidServerPresenceMonitorInterval));
    }

    #[tokio::test]
    async fn dropping_server_presence_monitor_requests_cancellation() {
        let (sender, mut receiver) = watch::channel(false);
        let notified = Arc::new(Notify::new());
        let task_notified = Arc::clone(&notified);
        let task = tokio::spawn(async move {
            let _ = receiver.changed().await;
            task_notified.notify_one();
            Ok(())
        });
        let handle = ServerPresenceMonitorHandle {
            sender,
            task: Some(task),
        };

        drop(handle);
        tokio::time::timeout(Duration::from_secs(1), notified.notified())
            .await
            .expect("dropping the monitor handle should wake its task");
    }
    #[tokio::test]
    async fn refreshes_server_count_presence_from_the_server_list() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_thread = thread::spawn(move || {
            let responses = [
                serde_json::json!({
                    "servers": [
                        { "name": "One" },
                        { "name": "Two" },
                        { "name": "Three" }
                    ]
                })
                .to_string(),
                r#"{ "ok": true }"#.to_owned(),
            ];
            let mut requests = Vec::new();

            for body in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 1024];
                let mut expected = None;

                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if expected.is_none() {
                        if let Some(position) =
                            bytes.windows(4).position(|window| window == b"\r\n\r\n")
                        {
                            let headers = String::from_utf8_lossy(&bytes[..position]);
                            let length = headers
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            expected = Some(position + 4 + length);
                        }
                    }
                    if expected.is_some_and(|length| bytes.len() >= length) {
                        break;
                    }
                }

                requests.push(String::from_utf8_lossy(&bytes).into_owned());
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }

            requests
        });

        let base_url = format!("http://{address}/api/bot/v1");
        let bot = Bot::builder()
            .token("yb_test-token")
            .api_base_url(base_url)
            .allow_insecure_http_for_tests(true)
            .build()
            .unwrap();

        let count = bot
            .refresh_server_count_presence(BotStatus::Online)
            .await
            .unwrap();
        assert_eq!(count, 3);

        let requests = server_thread.join().unwrap();
        assert!(requests[0].starts_with("GET /api/bot/v1/servers HTTP/1.1"));
        assert!(requests[1].starts_with("PUT /api/bot/v1/me/status HTTP/1.1"));
        let payload: Value =
            serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["status"], "online");
        assert_eq!(payload["activity"], "Overviewing 3 servers");
    }
    #[tokio::test]
    async fn cog_owned_event_errors_include_the_cog_name() {
        struct FailingListenerCog;
        impl crate::cogs::Cog for FailingListenerCog {
            fn name(&self) -> &str {
                "welcome"
            }

            fn register(&self, bot: &mut Bot) -> std::result::Result<(), String> {
                bot.on_event("member_join", |_event| async {
                    Err(Box::new(std::io::Error::other("listener failed")) as crate::HandlerError)
                })
                .map_err(|error| error.to_string())?;
                Ok(())
            }
        }

        let captured = Arc::new(std::sync::Mutex::new(None));
        let captured_handler = Arc::clone(&captured);
        let mut bot = Bot::builder().token("yb_test-token").build().unwrap();
        bot.on_error(move |error| {
            let captured = Arc::clone(&captured_handler);
            async move {
                *captured.lock().unwrap() = Some(error);
                Ok(())
            }
        });
        bot.add_cog(FailingListenerCog).unwrap();

        let event = crate::models::EventEnvelope {
            id: 42,
            event_type: "member_join".to_owned(),
            data: serde_json::json!({ "user_id": "123e4567-e89b-12d3-a456-426614174022" }),
            created_at: None,
        };
        bot.dispatch_event(&event, None).await.unwrap();

        let error = captured.lock().unwrap().clone().unwrap();
        assert_eq!(error.stage, ErrorStage::EventListener);
        assert_eq!(error.event_id, Some(42));
        assert_eq!(error.cog.as_deref(), Some("welcome"));
    }
}
