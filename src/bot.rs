use std::time::Duration;

use crate::api::ApiClient;
use crate::config::{Config, ConfigBuilder, ConfigError};
use crate::error::Result;
use crate::models::BotProfile;

/// Builder for a configured Yapper bot client.
#[derive(Debug, Default)]
pub struct BotBuilder {
    config: ConfigBuilder,
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

    /// Validate configuration and create the bot's shared HTTP client.
    pub fn build(self) -> Result<Bot> {
        let config: Config = self.config.build()?;
        let api = ApiClient::new(config)?;
        Ok(Bot { api })
    }
}

/// Configured client for a Yapper bot account.
///
/// The first milestone provides the authenticated REST client. The event
/// runtime and command dispatcher are added in subsequent implementation
/// milestones.
#[derive(Debug)]
pub struct Bot {
    api: ApiClient,
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

    /// Fetch the current bot profile.
    pub async fn get_me(&self) -> Result<BotProfile> {
        self.api.get_me().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_requires_a_bot_token() {
        let error = Bot::builder().build().unwrap_err();
        assert!(matches!(error, crate::Error::Configuration(ConfigError::MissingToken)));
    }
}
