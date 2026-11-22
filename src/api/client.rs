use std::sync::Arc;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{Client, Method, Response, Url};
use secrecy::ExposeSecret;
use serde_json::Value;

use crate::config::Config;
use crate::error::{ApiError, Error, Result, parse_retry_after, sanitize_transport_error};
use crate::ids::InteractionId;
use crate::models::{
    BotProfile, CommandDefinition, EventBatch, InteractionResponseType, Message,
    validate_command_list,
};

/// Cloneable authenticated client for Yapper's documented bot REST API.
#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    config: Arc<Config>,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApiClient")
            .field("api_base_url", self.config.api_base_url())
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    /// Create a client using a validated configuration.
    ///
    /// The underlying HTTP connection pool is reused across requests.
    pub fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);

        let authorization_value = format!("Bot {}", config.token().expose_secret());
        let mut authorization = HeaderValue::from_str(&authorization_value)
            .map_err(|_| Error::InvalidAuthorizationHeader)?;
        authorization.set_sensitive(true);

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .timeout(config.request_timeout())
            .redirect(Policy::none())
            .default_headers(headers)
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(sanitize_transport_error)?;

        Ok(Self { client, config })
    }

    /// Fetch the current bot profile from GET /me.
    pub async fn get_me(&self) -> Result<BotProfile> {
        let value = self.request_json(Method::GET, "me", None).await?;
        serde_json::from_value(value).map_err(Error::from)
    }

    /// Send a plain-text message to a channel.
    ///
    /// Message content is validated against Yapper's 2,000-character limit.
    /// This operation is not automatically retried because it is a side effect.
    pub async fn send_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: &str,
    ) -> Result<Message> {
        self.send_message_inner(channel_id, content, None).await
    }

    /// Reply to an existing message in a channel.
    ///
    /// The reply reference is included in the request body as reply_to.
    pub async fn reply_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        content: &str,
    ) -> Result<Message> {
        self.send_message_inner(channel_id, content, Some(message_id))
            .await
    }

    async fn send_message_inner(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: &str,
        reply_to: Option<&crate::ids::MessageId>,
    ) -> Result<Message> {
        if content.is_empty() {
            return Err(Error::EmptyMessageContent);
        }

        let character_count = content.chars().count();
        if character_count > 2_000 {
            return Err(Error::MessageContentTooLong {
                actual: character_count,
                max: 2_000,
            });
        }

        let route = format!("channels/{channel_id}/messages");
        let mut body = serde_json::json!({ "content": content });
        if let Some(message_id) = reply_to {
            body["reply_to"] = serde_json::json!(message_id.as_str());
        }
        let response = self.request_json(Method::POST, &route, Some(body)).await?;

        let message = response.get("message").cloned().unwrap_or(response);
        serde_json::from_value(message).map_err(Error::from)
    }

    /// Fetch one message from a channel.
    pub async fn get_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
    ) -> Result<Message> {
        let route = format!("channels/{channel_id}/messages/{message_id}");
        let response = self.request_json(Method::GET, &route, None).await?;
        let message = response.get("message").cloned().unwrap_or(response);
        serde_json::from_value(message).map_err(Error::from)
    }

    /// Edit the text content of a message.
    ///
    /// The method intentionally edits only content; omitted embed/component
    /// fields remain untouched on the server.
    pub async fn edit_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        content: &str,
    ) -> Result<Message> {
        let character_count = content.chars().count();
        if character_count > 2_000 {
            return Err(Error::MessageContentTooLong {
                actual: character_count,
                max: 2_000,
            });
        }

        let route = format!("channels/{channel_id}/messages/{message_id}");
        let body = serde_json::json!({ "content": content });
        let response = self.request_json(Method::PATCH, &route, Some(body)).await?;
        let message = response.get("message").cloned().unwrap_or(response);
        serde_json::from_value(message).map_err(Error::from)
    }

    /// Delete a message.
    ///
    /// Deleting another user's message requires Manage Messages permission.
    pub async fn delete_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
    ) -> Result<()> {
        let route = format!("channels/{channel_id}/messages/{message_id}");
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
    }

    /// Add the bot's reaction to a message.
    pub async fn add_reaction(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        emoji: &str,
    ) -> Result<()> {
        self.set_reaction(Method::PUT, channel_id, message_id, emoji)
            .await
    }

    /// Remove the bot's reaction from a message.
    pub async fn remove_reaction(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        emoji: &str,
    ) -> Result<()> {
        self.set_reaction(Method::DELETE, channel_id, message_id, emoji)
            .await
    }

    async fn set_reaction(
        &self,
        method: Method,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        emoji: &str,
    ) -> Result<()> {
        if emoji.is_empty() {
            return Err(Error::InvalidEmoji);
        }

        let mut url = self
            .config
            .api_base_url()
            .join("channels/")
            .map_err(|_| Error::InvalidApiRoute("channels".to_owned()))?;

        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::InvalidApiRoute("channels".to_owned()))?;
            segments
                .pop_if_empty()
                .push(channel_id.as_str())
                .push("messages")
                .push(message_id.as_str())
                .push("reactions")
                .push(emoji);
        }

        self.request_json_url(method, url, None).await?;
        Ok(())
    }

    /// Fetch one page of events using Yapper's documented long-poll endpoint.
    ///
    /// wait_seconds is capped at 25 seconds. Optional max_age_seconds
    /// filters events older than the supplied age and is capped at one day.
    pub async fn poll_events(
        &self,
        after: u64,
        wait_seconds: u8,
        max_age_seconds: Option<u32>,
    ) -> Result<EventBatch> {
        if wait_seconds > 25 {
            return Err(Error::InvalidEventPollWait);
        }

        if max_age_seconds.is_some_and(|age| age > 86_400) {
            return Err(Error::InvalidEventMaxAge);
        }

        let mut url = self
            .config
            .api_base_url()
            .join("events")
            .map_err(|_| Error::InvalidApiRoute("events".to_owned()))?;

        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("after", &after.to_string())
                .append_pair("wait", &wait_seconds.to_string());

            if let Some(max_age) = max_age_seconds {
                query.append_pair("max_age", &max_age.to_string());
            }
        }

        let value = self.request_json_url(Method::GET, url, None).await?;
        serde_json::from_value(value).map_err(Error::from)
    }

    /// Answer a slash-command or button interaction.
    ///
    /// The token is kept in a secret wrapper and inserted as a URL path
    /// segment, which ensures reserved characters are percent-encoded.
    /// Callback requests are never automatically retried.
    pub(crate) async fn respond_interaction(
        &self,
        interaction_id: &InteractionId,
        interaction_token: &secrecy::SecretString,
        response_type: InteractionResponseType,
        content: Option<&str>,
    ) -> Result<Value> {
        let mut url = self
            .config
            .api_base_url()
            .join("interactions/")
            .map_err(|_| Error::InvalidApiRoute("interactions".to_owned()))?;

        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::InvalidApiRoute("interactions".to_owned()))?;
            segments
                .pop_if_empty()
                .push(interaction_id.as_str())
                .push(interaction_token.expose_secret())
                .push("callback");
        }

        let mut body = serde_json::json!({ "type": response_type });
        if let Some(content) = content {
            body["content"] = serde_json::json!(content);
        }

        self.request_json_url(Method::POST, url, Some(body)).await
    }

    /// Replace the bot's complete registered command list with PUT /commands.
    ///
    /// Yapper replaces the full list in one request. The list is validated
    /// locally before any network request is sent.
    pub async fn sync_commands(&self, commands: &[CommandDefinition]) -> Result<Value> {
        validate_command_list(commands)?;
        let body = serde_json::json!({ "commands": commands });
        self.request_json(Method::PUT, "commands", Some(body)).await
    }

    async fn request_json(
        &self,
        method: Method,
        route: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let url = self
            .config
            .api_base_url()
            .join(route)
            .map_err(|_| Error::InvalidApiRoute(route.to_owned()))?;

        self.request_json_url(method, url, body).await
    }

    async fn request_json_url(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self.client.request(method, url);
        if let Some(body) = body {
            request = request.json(&body);
        }

        let mut response = request.send().await.map_err(sanitize_transport_error)?;

        let status = response.status();
        let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
        let response_body =
            read_limited_body(&mut response, self.config.max_response_bytes()).await?;

        tracing::debug!(
            http.status = status.as_u16(),
            response.bytes = response_body.len(),
            "received Yapper API response"
        );

        if !status.is_success() {
            return Err(ApiError::from_response(status, retry_after, &response_body).into());
        }

        if response_body.is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_slice(&response_body).map_err(Error::from)
    }
}

async fn read_limited_body(response: &mut Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Error::ResponseTooLarge { limit });
    }

    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(limit);
    let mut body = Vec::with_capacity(initial_capacity);

    // Response::chunk() works without Reqwest's optional streaming feature,
    // so we can enforce a hard body limit without stream helper dependencies.
    while let Some(chunk) = response.chunk().await.map_err(sanitize_transport_error)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(Error::ResponseTooLarge { limit });
        }

        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Receiver};
    use std::thread::{self, JoinHandle};

    use serde_json::json;

    use super::*;
    use crate::config::Config;

    const BOT_ID: &str = "123e4567-e89b-12d3-a456-426614174000";

    struct LocalMockServer {
        uri: String,
        request: Receiver<String>,
        thread: Option<JoinHandle<()>>,
    }

    impl LocalMockServer {
        fn start(status: u16, headers: &[(&str, &str)], body: String) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let uri = format!("http://{address}");
            let (sender, request) = mpsc::channel();
            let headers = headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<Vec<_>>();

            let thread = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();

                let mut request_bytes = Vec::new();
                let mut buffer = [0_u8; 1024];

                let mut expected_request_length = None;
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }

                    request_bytes.extend_from_slice(&buffer[..read]);

                    if expected_request_length.is_none() {
                        if let Some(header_start) = request_bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                        {
                            let body_start = header_start + 4;
                            let header_text =
                                String::from_utf8_lossy(&request_bytes[..header_start]);
                            let content_length = header_text
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            expected_request_length = Some(body_start + content_length);
                        }
                    }

                    if expected_request_length
                        .is_some_and(|expected| request_bytes.len() >= expected)
                    {
                        break;
                    }
                }

                let request_text = String::from_utf8_lossy(&request_bytes).into_owned();
                let _ = sender.send(request_text);

                let reason = match status {
                    200 => "OK",
                    302 => "Found",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    409 => "Conflict",
                    429 => "Too Many Requests",
                    503 => "Service Unavailable",
                    _ => "Mock Response",
                };

                let mut response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n",
                    body.len()
                );

                for (name, value) in headers {
                    response.push_str(&format!("{name}: {value}\r\n"));
                }

                response.push_str("\r\n");
                response.push_str(&body);
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

    async fn test_client(server: &LocalMockServer, response_limit: usize) -> ApiClient {
        let base_url = format!("{}/api/bot/v1", server.uri());
        let config = Config::builder()
            .token("yb_test-token")
            .api_base_url(base_url)
            .allow_insecure_http_for_tests(true)
            .max_response_bytes(response_limit)
            .build()
            .unwrap();

        ApiClient::new(config).unwrap()
    }

    #[tokio::test]
    async fn get_me_uses_documented_route_and_bot_authorization() {
        let server = LocalMockServer::start(
            200,
            &[],
            json!({
                "id": BOT_ID,
                "username": "yapcore-test",
                "display_name": "Yapcore Test Bot",
                "bio": "Testing the Rust client"
            })
            .to_string(),
        );

        let client = test_client(&server, 1024).await;
        let profile = client.get_me().await.unwrap();
        let request = server.finish();

        assert!(request.starts_with("GET /api/bot/v1/me HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bot yb_test-token")
        );
        assert_eq!(profile.id.unwrap().as_str(), BOT_ID);
        assert_eq!(profile.username.as_deref(), Some("yapcore-test"));
        assert_eq!(profile.display_name.as_deref(), Some("Yapcore Test Bot"));
    }

    #[tokio::test]
    async fn rejects_response_bodies_over_the_configured_limit() {
        let server = LocalMockServer::start(200, &[], "x".repeat(128));
        let client = test_client(&server, 32).await;
        let error = client.get_me().await.unwrap_err();
        let _request = server.finish();

        assert!(matches!(error, Error::ResponseTooLarge { limit: 32 }));
    }

    #[tokio::test]
    async fn preserves_rate_limit_metadata() {
        let server = LocalMockServer::start(
            429,
            &[("Retry-After", "7")],
            json!({
                "error": "slow down",
                "code": "rate_limited"
            })
            .to_string(),
        );

        let client = test_client(&server, 1024).await;
        let error = client.get_me().await.unwrap_err();
        let _request = server.finish();

        match error {
            Error::Api(api_error) => {
                assert_eq!(api_error.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(api_error.message(), "slow down");
                assert_eq!(api_error.code(), Some("rate_limited"));
                assert_eq!(
                    api_error.retry_after(),
                    Some(std::time::Duration::from_secs(7))
                );
            }
            other => panic!("expected an API error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn responds_to_interaction_with_encoded_token_and_callback_body() {
        let server = LocalMockServer::start(200, &[], r#"{"ok":true}"#.to_owned());
        let client = test_client(&server, 1024).await;
        let interaction_id = InteractionId::new("123e4567-e89b-12d3-a456-426614174010").unwrap();
        let interaction_token = secrecy::SecretString::from("token-with/slash");

        let response = client
            .respond_interaction(
                &interaction_id,
                &interaction_token,
                InteractionResponseType::Ephemeral,
                Some("Only you can see this"),
            )
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "POST /api/bot/v1/interactions/123e4567-e89b-12d3-a456-426614174010/token-with%2Fslash/callback HTTP/1.1"
        ));
        let request_body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(request_body).unwrap();
        assert_eq!(payload["type"], "ephemeral");
        assert_eq!(payload["content"], "Only you can see this");
        assert!(response["ok"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn send_message_posts_content_and_deserializes_the_returned_message() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "message": {
                    "id": MESSAGE_ID,
                    "channel_id": CHANNEL_ID,
                    "content": "pong"
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();

        let message = client.send_message(&channel_id, "pong").await.unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "POST /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages HTTP/1.1"
        ));
        let request_body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(request_body).unwrap();

        assert_eq!(payload["content"], "pong");
        assert_eq!(message.id.as_str(), MESSAGE_ID);
        assert_eq!(message.channel_id.as_str(), CHANNEL_ID);
        assert_eq!(message.content.as_deref(), Some("pong"));
    }

    #[tokio::test]
    async fn send_message_rejects_empty_or_oversized_content_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();

        assert!(matches!(
            client.send_message(&channel_id, "").await,
            Err(Error::EmptyMessageContent)
        ));
        assert!(matches!(
            client.send_message(&channel_id, &"x".repeat(2_001)).await,
            Err(Error::MessageContentTooLong {
                actual: 2_001,
                max: 2_000
            })
        ));
    }

    #[tokio::test]
    async fn edits_message_content_with_patch() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "id": MESSAGE_ID,
                "channel_id": CHANNEL_ID,
                "content": "edited"
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        let message = client
            .edit_message(&channel_id, &message_id, "edited")
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "PATCH /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001 HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["content"], "edited");
        assert_eq!(message.content.as_deref(), Some("edited"));
    }

    #[tokio::test]
    async fn adds_reactions_using_one_encoded_emoji_path_segment() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        client
            .add_reaction(&channel_id, &message_id, "🔥/up")
            .await
            .unwrap();
        let request = server.finish();

        let request_line = request.lines().next().unwrap();
        assert!(request_line.starts_with("PUT /api/bot/v1/channels/"));
        assert!(request_line.contains("/reactions/"));
        assert!(request_line.contains("%2Fup"));
        assert!(!request_line.contains("🔥/up"));
    }

    #[tokio::test]
    async fn rejects_empty_reaction_identifiers_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();
        let message_id =
            crate::ids::MessageId::new("123e4567-e89b-12d3-a456-426614174001").unwrap();

        assert!(matches!(
            client.add_reaction(&channel_id, &message_id, "").await,
            Err(Error::InvalidEmoji)
        ));
    }

    #[tokio::test]
    async fn fetches_one_message_with_get() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "id": MESSAGE_ID,
                "channel_id": CHANNEL_ID,
                "content": "fetched"
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        let message = client.get_message(&channel_id, &message_id).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "GET /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001 HTTP/1.1"
        ));
        assert_eq!(message.content.as_deref(), Some("fetched"));
    }

    #[tokio::test]
    async fn deletes_one_message_with_empty_success_response() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        client.delete_message(&channel_id, &message_id).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "DELETE /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001 HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn poll_events_uses_cursor_wait_and_max_age_query_parameters() {
        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "events": [{
                    "id": 12,
                    "type": "new_event_type",
                    "data": { "channel_id": "channel-1" }
                }],
                "last": 12
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;

        let batch = client.poll_events(10, 25, Some(3_600)).await.unwrap();
        let request = server.finish();

        assert!(
            request.starts_with("GET /api/bot/v1/events?after=10&wait=25&max_age=3600 HTTP/1.1")
        );
        assert_eq!(batch.last(), 12);
        assert_eq!(batch.events().len(), 1);
        assert_eq!(batch.events()[0].event_type, "new_event_type");
    }

    #[tokio::test]
    async fn poll_events_rejects_wait_and_max_age_above_platform_limits() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();

        assert!(matches!(
            client.poll_events(0, 26, None).await,
            Err(Error::InvalidEventPollWait)
        ));
        assert!(matches!(
            client.poll_events(0, 0, Some(86_401)).await,
            Err(Error::InvalidEventMaxAge)
        ));
    }

    #[tokio::test]
    async fn sync_commands_replaces_the_full_registry_with_the_documented_payload() {
        let server = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&server, 1024).await;
        let commands = vec![
            CommandDefinition::new("ping", "Check whether the bot is alive").unwrap(),
            CommandDefinition::new("about", "Show information about the bot").unwrap(),
        ];

        let response = client.sync_commands(&commands).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with("PUT /api/bot/v1/commands HTTP/1.1"));
        let request_body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(request_body).unwrap();

        assert_eq!(payload["commands"][0]["name"], "ping");
        assert_eq!(
            payload["commands"][1]["description"],
            "Show information about the bot"
        );
        assert!(payload.get("interactions").is_none());
        assert!(response["ok"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn does_not_follow_redirects_for_authenticated_requests() {
        let server = LocalMockServer::start(
            302,
            &[("Location", "https://example.com/redirect-target")],
            String::new(),
        );

        let client = test_client(&server, 1024).await;
        let error = client.get_me().await.unwrap_err();
        let request = server.finish();

        assert!(request.starts_with("GET /api/bot/v1/me HTTP/1.1"));
        assert!(matches!(error, Error::Api(api) if api.status() == reqwest::StatusCode::FOUND));
    }
}
