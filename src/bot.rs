use std::future::Future;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::watch;

use crate::api::ApiClient;
use crate::commands::{CommandContext, CommandRegistry, HandlerResult};
use crate::config::{Config, ConfigBuilder, ConfigError};
use crate::error::{Error, Result};
use crate::ids::{ChannelId, MessageId, UserId};
use crate::models::{BotProfile, CommandDefinition, Message};

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

/// Builder for a configured Yapper bot.
#[derive(Debug)]
pub struct BotBuilder {
    config: ConfigBuilder,
    prefixes: Vec<String>,
    ignore_bots: bool,
}

impl Default for BotBuilder {
    fn default() -> Self {
        Self {
            config: ConfigBuilder::default(),
            prefixes: vec!["/".to_owned(), "!".to_owned()],
            ignore_bots: true,
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

    /// Validate configuration and create the bot's shared HTTP client.
    pub fn build(self) -> Result<Bot> {
        let config: Config = self.config.build()?;
        let api = ApiClient::new(config)?;
        let (shutdown, _) = watch::channel(false);

        Ok(Bot {
            api,
            commands: CommandRegistry::default(),
            prefixes: self.prefixes,
            ignore_bots: self.ignore_bots,
            shutdown,
        })
    }
}

/// Configured client and prefix-command runtime for a Yapper bot account.
#[derive(Debug)]
pub struct Bot {
    api: ApiClient,
    commands: CommandRegistry,
    prefixes: Vec<String>,
    ignore_bots: bool,
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
        self.commands.register(name, description, handler)?;
        Ok(self)
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
        let profile = self.api.get_me().await?;
        let bot_id = profile.id.clone();

        // Yapper's PUT /commands replaces the full registry, so always sync
        // the complete set once before beginning event polling.
        self.api.sync_commands(&self.commands.definitions()).await?;

        tracing::info!(
            username = profile.username.as_deref().unwrap_or("<unknown>"),
            "Yapcore bot connected"
        );

        let initial = self.api.poll_events(0, 0, None).await?;
        let mut cursor = initial.last();
        let mut shutdown = self.shutdown.subscribe();
        let mut consecutive_failures = 0_u32;

        loop {
            if *shutdown.borrow() {
                break;
            }

            let batch = tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                    continue;
                }
                result = self.api.poll_events(cursor, 25, None) => {
                    match result {
                        Ok(batch) => {
                            consecutive_failures = 0;
                            batch
                        }
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
                                    if changed.is_err() || *shutdown.borrow() {
                                        break;
                                    }
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
                    tracing::error!(
                        event_type = %event.event_type,
                        event_id = event.id,
                        error = %error,
                        "event dispatch failed"
                    );
                }

                cursor = event.id;
                if *shutdown.borrow() {
                    break;
                }
            }

            cursor = cursor.max(batch.last());
        }

        Ok(())
    }

    /// Alias for run.
    pub async fn start(&self) -> Result<()> {
        self.run().await
    }

    async fn dispatch_event(
        &self,
        event: &crate::models::EventEnvelope,
        bot_id: Option<&UserId>,
    ) -> Result<()> {
        if event.event_type != "message" {
            return Ok(());
        }

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
            tracing::error!(command = %command, error = %error, "command handler returned an error");
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
}
