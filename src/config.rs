use std::net::IpAddr;
use std::time::Duration;

use reqwest::Url;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

/// The official Yapper bot API base URL used by default.
pub const DEFAULT_API_BASE_URL: &str = "https://www.yapper.website/api/bot/v1/";

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(40);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Configuration validation failure.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// No bot token was supplied.
    #[error("a bot token is required; pass one or set the configured environment variable")]
    MissingToken,
    /// The token does not have the expected bot-token shape.
    #[error("invalid bot token; expected a non-empty token beginning with yb_")]
    InvalidToken,
    /// The named environment variable could not be read.
    #[error("environment variable {0} is missing or is not valid Unicode")]
    MissingEnvironmentVariable(String),
    /// The API base URL could not be parsed or has no host.
    #[error("the API base URL is invalid")]
    InvalidApiBaseUrl,
    /// API URLs must not contain credentials.
    #[error("the API base URL must not contain username or password information")]
    ApiBaseUrlContainsCredentials,
    /// API URLs must not contain query strings or fragments.
    #[error("the API base URL must not contain a query string or fragment")]
    ApiBaseUrlContainsQueryOrFragment,
    /// Non-HTTPS API URLs are rejected by default.
    #[error("the API base URL must use HTTPS")]
    HttpsRequired,
    /// Plain HTTP is allowed only for explicitly enabled loopback tests.
    #[error("plain HTTP is allowed only for loopback test servers when explicitly enabled")]
    InsecureHttpNotAllowed,
    /// The configured request timeout must be non-zero.
    #[error("the request timeout must be greater than zero")]
    ZeroRequestTimeout,
    /// The configured response body limit must be non-zero.
    #[error("the maximum response body size must be greater than zero")]
    ZeroResponseLimit,
}

/// Validated configuration shared by the bot and its API client.
pub struct Config {
    token: SecretString,
    api_base_url: Url,
    request_timeout: Duration,
    max_response_bytes: usize,
}

impl Config {
    /// Create a configuration builder with Yapcore's safe defaults.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// The configured API base URL, including its trailing slash.
    pub fn api_base_url(&self) -> &Url {
        &self.api_base_url
    }

    /// Maximum time allowed for an individual HTTP request.
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// Maximum number of response bytes Yapcore will read.
    pub fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }

    pub(crate) fn token(&self) -> &SecretString {
        &self.token
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Config")
            .field("token", &"[REDACTED]")
            .field("api_base_url", &self.api_base_url)
            .field("request_timeout", &self.request_timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

/// Builder for validated bot configuration.
#[derive(Debug)]
pub struct ConfigBuilder {
    token: Option<SecretString>,
    api_base_url: String,
    request_timeout: Duration,
    max_response_bytes: usize,
    allow_insecure_http_for_tests: bool,
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            token: None,
            api_base_url: DEFAULT_API_BASE_URL.to_owned(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            allow_insecure_http_for_tests: false,
        }
    }
}

impl ConfigBuilder {
    /// Set the bot token directly.
    ///
    /// The token is wrapped immediately and its Debug representation is redacted.
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(SecretString::from(token.into()));
        self
    }

    /// Read the token from an environment variable.
    pub fn token_from_env(
        mut self,
        variable: impl AsRef<str>,
    ) -> Result<Self, ConfigError> {
        let variable = variable.as_ref();
        let token = std::env::var(variable)
            .map_err(|_| ConfigError::MissingEnvironmentVariable(variable.to_owned()))?;
        self.token = Some(SecretString::from(token));
        Ok(self)
    }

    /// Override the API base URL.
    ///
    /// Production URLs must use HTTPS. Plain HTTP is accepted only for loopback
    /// mock servers when allow_insecure_http_for_tests is enabled.
    pub fn api_base_url(mut self, url: impl Into<String>) -> Self {
        self.api_base_url = url.into();
        self
    }

    /// Set the per-request timeout. Defaults to 40 seconds to accommodate
    /// Yapper's 25-second long-poll endpoint.
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Set the maximum response body size. Defaults to 16 MiB.
    pub fn max_response_bytes(mut self, limit: usize) -> Self {
        self.max_response_bytes = limit;
        self
    }

    /// Permit plain HTTP to loopback addresses for tests only.
    ///
    /// This never permits plain HTTP to a non-loopback host.
    pub fn allow_insecure_http_for_tests(mut self, allow: bool) -> Self {
        self.allow_insecure_http_for_tests = allow;
        self
    }

    /// Validate and build the configuration.
    pub fn build(self) -> Result<Config, ConfigError> {
        let token = self.token.ok_or(ConfigError::MissingToken)?;
        validate_token(token.expose_secret())?;

        if self.request_timeout.is_zero() {
            return Err(ConfigError::ZeroRequestTimeout);
        }
        if self.max_response_bytes == 0 {
            return Err(ConfigError::ZeroResponseLimit);
        }

        let mut api_base_url =
            Url::parse(&self.api_base_url).map_err(|_| ConfigError::InvalidApiBaseUrl)?;

        if api_base_url.host_str().is_none() {
            return Err(ConfigError::InvalidApiBaseUrl);
        }
        if !api_base_url.username().is_empty() || api_base_url.password().is_some() {
            return Err(ConfigError::ApiBaseUrlContainsCredentials);
        }
        if api_base_url.query().is_some() || api_base_url.fragment().is_some() {
            return Err(ConfigError::ApiBaseUrlContainsQueryOrFragment);
        }

        match api_base_url.scheme() {
            "https" => {}
            "http" if self.allow_insecure_http_for_tests && is_loopback(&api_base_url) => {}
            "http" if self.allow_insecure_http_for_tests => {
                return Err(ConfigError::InsecureHttpNotAllowed);
            }
            "http" => return Err(ConfigError::HttpsRequired),
            _ => return Err(ConfigError::HttpsRequired),
        }

        let normalized_path = format!("{}/", api_base_url.path().trim_end_matches('/'));
        api_base_url.set_path(&normalized_path);

        Ok(Config {
            token,
            api_base_url,
            request_timeout: self.request_timeout,
            max_response_bytes: self.max_response_bytes,
        })
    }
}

fn validate_token(token: &str) -> Result<(), ConfigError> {
    let valid_shape = token.starts_with("yb_")
        && token.len() > 3
        && token
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte));

    if valid_shape {
        Ok(())
    } else {
        Err(ConfigError::InvalidToken)
    }
}

fn is_loopback(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    host.trim_matches(|character| character == '[' || character == ']')
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_official_api_url_and_safe_limits() {
        let config = Config::builder().token("yb_test-token").build().unwrap();

        assert_eq!(
            config.api_base_url().as_str(),
            "https://www.yapper.website/api/bot/v1/"
        );
        assert_eq!(config.request_timeout(), Duration::from_secs(40));
        assert_eq!(config.max_response_bytes(), 16 * 1024 * 1024);
    }

    #[test]
    fn debug_output_does_not_reveal_the_token() {
        let config = Config::builder()
            .token("yb_private-test-token")
            .build()
            .unwrap();

        let debug = format!("{config:?}");
        assert!(!debug.contains("yb_private-test-token"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn rejects_invalid_token_shapes() {
        let error = Config::builder().token("not-a-bot-token").build().unwrap_err();
        assert_eq!(error, ConfigError::InvalidToken);
    }

    #[test]
    fn rejects_plain_http_by_default() {
        let error = Config::builder()
            .token("yb_test")
            .api_base_url("http://127.0.0.1:8080/api/bot/v1")
            .build()
            .unwrap_err();

        assert_eq!(error, ConfigError::HttpsRequired);
    }

    #[test]
    fn allows_plain_http_only_for_explicit_loopback_tests() {
        let config = Config::builder()
            .token("yb_test")
            .api_base_url("http://127.0.0.1:8080/api/bot/v1")
            .allow_insecure_http_for_tests(true)
            .build()
            .unwrap();

        assert_eq!(
            config.api_base_url().as_str(),
            "http://127.0.0.1:8080/api/bot/v1/"
        );
    }

    #[test]
    fn never_allows_plain_http_to_a_remote_host() {
        let error = Config::builder()
            .token("yb_test")
            .api_base_url("http://example.com/api/bot/v1")
            .allow_insecure_http_for_tests(true)
            .build()
            .unwrap_err();

        assert_eq!(error, ConfigError::InsecureHttpNotAllowed);
    }

    #[test]
    fn rejects_base_urls_with_query_parameters() {
        let error = Config::builder()
            .token("yb_test")
            .api_base_url("https://example.com/api?token=not-allowed")
            .build()
            .unwrap_err();

        assert_eq!(error, ConfigError::ApiBaseUrlContainsQueryOrFragment);
    }

    #[test]
    fn rejects_zero_limits() {
        let timeout_error = Config::builder()
            .token("yb_test")
            .request_timeout(Duration::ZERO)
            .build()
            .unwrap_err();
        assert_eq!(timeout_error, ConfigError::ZeroRequestTimeout);

        let response_error = Config::builder()
            .token("yb_test")
            .max_response_bytes(0)
            .build()
            .unwrap_err();
        assert_eq!(response_error, ConfigError::ZeroResponseLimit);
    }
}
