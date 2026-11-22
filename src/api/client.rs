use std::sync::Arc;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{Client, Method, Response};
use serde_json::Value;
use secrecy::ExposeSecret;

use crate::config::Config;
use crate::error::{ApiError, Error, Result, parse_retry_after, sanitize_transport_error};
use crate::models::{BotProfile, CommandDefinition, validate_command_list};

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

        let mut request = self.client.request(method, url);
        if let Some(body) = body {
            request = request.json(&body);
        }

        let mut response = request
            .send()
            .await
            .map_err(sanitize_transport_error)?;

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
                            let content_length = header_text.lines().find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            }).unwrap_or(0);
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
