use std::str::FromStr;
use std::sync::Arc;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{Client, Response};
use secrecy::ExposeSecret;

use crate::config::Config;
use crate::error::{ApiError, Error, Result, parse_retry_after, sanitize_transport_error};
use crate::models::BotProfile;

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
            .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(sanitize_transport_error)?;

        Ok(Self { client, config })
    }

    /// Fetch the current bot profile from GET /me.
    pub async fn get_me(&self) -> Result<BotProfile> {
        let url = self
            .config
            .api_base_url()
            .join("me")
            .map_err(|_| Error::InvalidApiRoute("me".to_owned()))?;

        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(sanitize_transport_error)?;

        let status = response.status();
        let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
        let body = read_limited_body(&mut response, self.config.max_response_bytes()).await?;

        if !status.is_success() {
            return Err(ApiError::from_response(status, retry_after, &body).into());
        }

        serde_json::from_slice(&body).map_err(Error::from)
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
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    const BOT_ID: &str = "123e4567-e89b-12d3-a456-426614174000";

    async fn test_client(server: &MockServer, response_limit: usize) -> ApiClient {
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
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/bot/v1/me"))
            .and(header("Authorization", "Bot yb_test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": BOT_ID,
                "username": "yapcore-test",
                "display_name": "Yapcore Test Bot",
                "bio": "Testing the Rust client"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server, 1024).await;
        let profile = client.get_me().await.unwrap();

        assert_eq!(profile.id.unwrap().as_str(), BOT_ID);
        assert_eq!(profile.username.as_deref(), Some("yapcore-test"));
        assert_eq!(profile.display_name.as_deref(), Some("Yapcore Test Bot"));
    }

    #[tokio::test]
    async fn rejects_response_bodies_over_the_configured_limit() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/bot/v1/me"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(128)))
            .mount(&server)
            .await;

        let client = test_client(&server, 32).await;
        let error = client.get_me().await.unwrap_err();

        assert!(matches!(error, Error::ResponseTooLarge { limit: 32 }));
    }

    #[tokio::test]
    async fn preserves_rate_limit_metadata() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/bot/v1/me"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "7")
                    .set_body_json(json!({
                        "error": "slow down",
                        "code": "rate_limited"
                    })),
            )
            .mount(&server)
            .await;

        let client = test_client(&server, 1024).await;
        let error = client.get_me().await.unwrap_err();

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
    async fn does_not_follow_redirects_for_authenticated_requests() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/bot/v1/me"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("Location", "https://example.com/redirect-target"),
            )
            .mount(&server)
            .await;

        let client = test_client(&server, 1024).await;
        let error = client.get_me().await.unwrap_err();

        assert!(matches!(error, Error::Api(api) if api.status() == reqwest::StatusCode::FOUND));
    }
}
