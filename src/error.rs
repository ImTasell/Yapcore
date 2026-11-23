use std::time::{Duration, SystemTime};

use reqwest::StatusCode;
use reqwest::header::HeaderValue;
use serde_json::Value;
use thiserror::Error;

use crate::config::ConfigError;
use crate::ids::IdError;
use crate::models::{CommandDefinitionError, ComponentValidationError, EmbedValidationError};

/// Result type used by Yapcore's public API.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A structured error returned by the Yapper API.
#[derive(Debug, Clone, Error)]
#[error("Yapper API returned HTTP {status}: {message}")]
pub struct ApiError {
    status: StatusCode,
    message: String,
    code: Option<String>,
    permission: Option<String>,
    retry_after: Option<Duration>,
}

impl ApiError {
    pub(crate) fn from_response(
        status: StatusCode,
        retry_after: Option<Duration>,
        body: &[u8],
    ) -> Self {
        let value = serde_json::from_slice::<Value>(body).unwrap_or(Value::Null);
        let error_value = value.get("error");

        let message = error_value
            .and_then(Value::as_str)
            .or_else(|| value.get("message").and_then(Value::as_str))
            .or_else(|| {
                error_value
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
            })
            .unwrap_or_else(|| status.canonical_reason().unwrap_or("request failed"));

        let code = value
            .get("code")
            .and_then(Value::as_str)
            .or_else(|| {
                error_value
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);

        let permission = value
            .get("permission")
            .and_then(Value::as_str)
            .or_else(|| {
                error_value
                    .and_then(|error| error.get("permission"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);

        Self {
            status,
            message: truncate_message(message, 2_048),
            code,
            permission,
            retry_after,
        }
    }

    /// HTTP status returned by Yapper.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Human-readable API error message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Optional stable API error code, when supplied by the server.
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Optional permission name associated with a permission failure.
    pub fn permission(&self) -> Option<&str> {
        self.permission.as_deref()
    }

    /// Server-provided retry delay, when present and valid.
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }
}

/// Errors produced by Yapcore.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// A string could not be decoded as a Yapper identifier.
    #[error(transparent)]
    Identifier(#[from] IdError),

    /// Invalid or incomplete local configuration.
    #[error(transparent)]
    Configuration(#[from] ConfigError),

    /// A command definition or command list violates the documented local constraints.
    #[error(transparent)]
    CommandDefinition(#[from] CommandDefinitionError),

    /// An embed or embed field violates a documented size or structure limit.
    #[error(transparent)]
    EmbedValidation(#[from] EmbedValidationError),

    /// A button or action row violates a documented component limit.
    #[error(transparent)]
    ComponentValidation(#[from] ComponentValidationError),

    /// A message may contain no more than five embeds.
    #[error("a message may contain at most 5 embeds; received {actual}")]
    TooManyEmbeds {
        /// Number of supplied embeds.
        actual: usize,
    },

    /// The aggregate text across all embeds exceeded 6,000 characters.
    #[error("combined embed text is {actual} characters; the maximum is 6000")]
    MessageEmbedsTooLong {
        /// Actual combined character count.
        actual: usize,
    },

    /// A message must contain non-empty text or at least one embed.
    #[error("a message must contain non-empty content or at least one embed")]
    EmptyMessagePayload,

    /// The event poll wait exceeds Yapper's 25-second maximum.
    #[error("event poll wait must be between 0 and 25 seconds")]
    InvalidEventPollWait,

    /// The event catch-up age exceeds the one-day maximum.
    #[error("event max_age must not exceed 86,400 seconds")]
    InvalidEventMaxAge,

    /// The requested response operation requires an interaction context.
    #[error("this response is only available for an interaction")]
    InteractionRequired,

    /// A message reply was attempted without a source message ID.
    #[error("this command context has no source message to reply to")]
    MessageContextRequired,

    /// The interaction already received a response attempt.
    #[error("this interaction has reached its 10-response limit")]
    InteractionAlreadyAnswered,

    /// Plain-text messages cannot be empty.
    #[error("message content must not be empty")]
    EmptyMessageContent,

    /// Plain-text message content is longer than Yapper's 2,000-character limit.
    #[error("message content is {actual} characters; the maximum is {max}")]
    MessageContentTooLong {
        /// Actual character count.
        actual: usize,
        /// Maximum allowed character count.
        max: usize,
    },

    /// A reaction identifier must not be empty.
    #[error("reaction emoji identifier must not be empty")]
    InvalidEmoji,

    /// A recent-message page size must be between 1 and 100.
    #[error("message page limit must be between 1 and 100; received {actual}")]
    InvalidMessagePageLimit {
        /// Requested page size.
        actual: usize,
    },

    /// A forward-message cursor must not be empty.
    #[error("message cursor must not be empty")]
    EmptyMessageCursor,

    /// A bulk-delete request must contain between 1 and 100 message IDs.
    #[error("bulk delete requires 1-100 message IDs; received {actual}")]
    InvalidBulkDeleteCount {
        /// Number of message IDs supplied.
        actual: usize,
    },

    /// A member page size must be between 1 and 500.
    #[error("member page limit must be between 1 and 500; received {actual}")]
    InvalidMemberPageLimit {
        /// Requested member page size.
        actual: usize,
    },

    /// A channel name must not be empty.
    #[error("channel name must not be empty")]
    EmptyChannelName,

    /// A category name must not be empty.
    #[error("category name must not be empty")]
    EmptyCategoryName,

    /// A channel edit must include at least one changed field.
    #[error("channel edit options must contain at least one field")]
    EmptyChannelEditOptions,

    /// Channel slowmode cannot exceed six hours.
    #[error("channel slowmode must be between 0 and 21,600 seconds; received {actual}")]
    InvalidChannelSlowmode { actual: u64 },

    /// A private channel may list at most 50 explicitly allowed users.
    #[error("a channel may list at most 50 allowed users; received {actual}")]
    TooManyChannelUsers { actual: usize },

    /// An activity rotation must contain at least two entries.
    #[error("activity rotation requires at least 2 entries; received {actual}")]
    TooFewRotationActivities { actual: usize },

    /// A server-presence monitor interval must be greater than zero.
    #[error("server presence monitor interval must be greater than zero")]
    InvalidServerPresenceMonitorInterval,

    /// A server-presence monitor task panicked while running.
    #[error("server presence monitor task failed: {0}")]
    ServerPresenceMonitorTaskFailed(String),

    /// An activity rotation interval must be greater than zero.
    #[error("activity rotation interval must be greater than zero")]
    InvalidPresenceRotationInterval,

    /// An activity rotation task panicked while running.
    #[error("presence rotation task failed: {0}")]
    PresenceRotationTaskFailed(String),

    /// A bot profile patch must change at least one field.
    #[error("bot profile update must set a display name or bio")]
    EmptyBotProfileEdit,

    /// The bot's display name exceeds 32 characters.
    #[error("bot display name is {actual} characters; the maximum is 32")]
    BotDisplayNameTooLong { actual: usize },

    /// The bot's bio exceeds 190 characters.
    #[error("bot bio is {actual} characters; the maximum is 190")]
    BotBioTooLong { actual: usize },

    /// An uploaded image must not be empty.
    #[error("image upload must not be empty")]
    EmptyImageUpload,

    /// An image upload is larger than the endpoint's maximum size.
    #[error("image upload is {actual} bytes; the maximum is {max} bytes")]
    ImageUploadTooLarge { actual: usize, max: usize },

    /// A DM-open response did not contain a valid channel identifier.
    #[error("Yapper's DM response did not contain a valid channel_id")]
    MissingDmChannelId,

    /// A bot activity string must contain visible text.
    #[error("bot activity must not be empty")]
    EmptyBotActivity,

    /// A bot activity string is longer than Yapper's 60-character limit.
    #[error("bot activity is {actual} characters; the maximum is 60")]
    BotActivityTooLong { actual: usize },

    /// A role edit must include at least one changed field.
    #[error("role edit options must contain at least one field")]
    EmptyRoleEditOptions,

    /// A nickname is longer than Yapper's 32-character limit.
    #[error("nickname is {actual} characters; the maximum is 32")]
    NicknameTooLong {
        /// Actual nickname length.
        actual: usize,
    },

    /// A moderation reason is longer than Yapper's 200-character limit.
    #[error("moderation reason is {actual} characters; the maximum is 200")]
    ModerationReasonTooLong {
        /// Actual reason length.
        actual: usize,
    },

    /// A timeout duration is above the platform's 28-day maximum.
    #[error("timeout duration must be between 0 and 40,320 minutes; received {actual}")]
    TimeoutTooLong {
        /// Requested timeout duration.
        actual: u32,
    },

    /// The request failed before a usable HTTP response was received.
    ///
    /// The underlying Reqwest error has its URL removed to avoid accidental
    /// leakage of sensitive query strings in logs.
    #[error("HTTP transport failed: {0}")]
    Transport(#[source] reqwest::Error),

    /// The API response exceeded the configured body-size limit.
    #[error("API response exceeded the configured limit of {limit} bytes")]
    ResponseTooLarge {
        /// Maximum number of bytes allowed.
        limit: usize,
    },

    /// The server returned a non-success HTTP response.
    #[error(transparent)]
    Api(#[from] ApiError),

    /// The successful API response was not valid JSON for the expected type.
    #[error("could not decode the Yapper API response: {0}")]
    Json(#[from] serde_json::Error),

    /// Yapcore could not safely construct the requested API URL.
    #[error("invalid API route: {0}")]
    InvalidApiRoute(String),

    /// A validated token could not be converted to an HTTP header.
    #[error("the bot token could not be encoded as an HTTP authorization header")]
    InvalidAuthorizationHeader,
}

pub(crate) fn sanitize_transport_error(error: reqwest::Error) -> Error {
    Error::Transport(error.without_url())
}

pub(crate) fn parse_retry_after(header: Option<&HeaderValue>) -> Option<Duration> {
    let value = header?.to_str().ok()?.trim();

    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let retry_at = httpdate::parse_http_date(value).ok()?;
    retry_at.duration_since(SystemTime::now()).ok()
}

fn truncate_message(message: &str, maximum_characters: usize) -> String {
    let mut result: String = message.chars().take(maximum_characters).collect();

    if message.chars().count() > maximum_characters {
        result.push('…');
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_retry_after_seconds() {
        let header = HeaderValue::from_static("12");
        assert_eq!(
            parse_retry_after(Some(&header)),
            Some(Duration::from_secs(12))
        );
    }

    #[test]
    fn parses_retry_after_http_date() {
        let target = SystemTime::now() + Duration::from_secs(10);
        let header = HeaderValue::from_str(&httpdate::fmt_http_date(target)).unwrap();
        let delay = parse_retry_after(Some(&header)).unwrap();

        assert!(delay <= Duration::from_secs(10));
        assert!(delay >= Duration::from_secs(8));
    }

    #[test]
    fn rejects_invalid_retry_after_values() {
        let header = HeaderValue::from_static("not-a-date");
        assert_eq!(parse_retry_after(Some(&header)), None);
    }

    #[test]
    fn extracts_structured_api_error_fields() {
        let error = ApiError::from_response(
            StatusCode::FORBIDDEN,
            None,
            br#"{"error":"missing permission","code":"missing_permission","permission":"manage_roles"}"#,
        );

        assert_eq!(error.status(), StatusCode::FORBIDDEN);
        assert_eq!(error.message(), "missing permission");
        assert_eq!(error.code(), Some("missing_permission"));
        assert_eq!(error.permission(), Some("manage_roles"));
    }

    #[test]
    fn falls_back_to_status_for_non_json_error_bodies() {
        let error = ApiError::from_response(StatusCode::BAD_GATEWAY, None, b"proxy failure");
        assert_eq!(error.message(), "Bad Gateway");
    }

    #[test]
    fn bounds_server_error_messages() {
        let long_message = "x".repeat(3_000);
        let body = serde_json::json!({ "error": long_message });
        let bytes = serde_json::to_vec(&body).unwrap();

        let error = ApiError::from_response(StatusCode::BAD_REQUEST, None, &bytes);
        assert!(error.message().chars().count() <= 2_049);
        assert!(error.message().ends_with('…'));
    }
}
