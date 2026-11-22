use std::future::Future;
use std::sync::atomic::Ordering;
use std::time::Duration;

use secrecy::SecretString;
use serde_json::Value;
use tokio::sync::watch;

use crate::api::ApiClient;
use crate::commands::{CommandContext, CommandRegistry, HandlerResult, SlashCommandInvocation};
use crate::config::{Config, ConfigBuilder, ConfigError};
use crate::error::{Error, Result};
use crate::ids::{ChannelId, InteractionId, MessageId, UserId};
use crate::models::{BotProfile, CommandDefinition, InteractionResponseType, Message};

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
        match event.event_type.as_str() {
            "message" => self.dispatch_message_event(event, bot_id).await,
            "command" => self.dispatch_command_event(event).await,
            _ => Ok(()),
        }
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
            tracing::error!(command = %command, error = %error, "command handler returned an error");
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
                tracing::error!(command = %command, error = %error, "slash command handler returned an error");
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
                    tracing::error!(command = %command, error = %error, "slash command handler failed after automatic acknowledgement");
                }
            }
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

    #[tokio::test]
    async fn dispatches_slash_command_reply_through_interaction_callback() {
        let server = CallbackServer::start();
        let mut bot = test_bot(&server);

        bot.command("ping", "Check whether the bot is alive", |context| async move {
            assert!(context.is_interaction());
            assert_eq!(context.command(), "ping");
            assert_eq!(context.args(), "hello there");
            context.reply("Pong from slash!").await
                .map_err(|error| -> crate::HandlerError { Box::new(error) })?;
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
}
