//! Verification helpers for optional Yapper event-webhook delivery.
//!
//! This module does not run an HTTP server. Read the exact request body and
//! `X-Yapper-Signature` header in your server framework, verify them here,
//! then acknowledge the request quickly and process the event asynchronously.

use std::fmt;

use ring::hmac;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::models::EventEnvelope;

/// Errors returned while validating a signed event-webhook request.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum WebhookVerificationError {
    /// The signature header must be `sha256=` followed by 64 hexadecimal characters.
    #[error("invalid Yapper webhook signature header")]
    InvalidSignatureHeader,
    /// The supplied HMAC does not match the raw request body.
    #[error("Yapper webhook signature did not match the request body")]
    InvalidSignature,
    /// A correctly signed body was not a valid webhook event envelope.
    #[error("signed Yapper webhook body was not a valid event envelope")]
    InvalidPayload,
}

/// Event envelope delivered to a configured event-webhook URL.
///
/// Webhook envelopes use `event` rather than the polling response's `type`.
#[derive(Clone, PartialEq, Deserialize)]
pub struct EventWebhook {
    /// Same monotonically increasing event ID used by long polling.
    pub id: u64,
    /// Yapper event name.
    pub event: String,
    /// Event-specific fields, preserved as JSON.
    #[serde(default)]
    pub data: Value,
}

impl fmt::Debug for EventWebhook {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let envelope = EventEnvelope {
            id: self.id,
            event_type: self.event.clone(),
            data: self.data.clone(),
            created_at: None,
        };
        formatter
            .debug_struct("EventWebhook")
            .field("envelope", &envelope)
            .finish()
    }
}

impl EventWebhook {
    /// Convert a verified webhook into the common long-poll event model.
    pub fn into_event_envelope(self) -> EventEnvelope {
        EventEnvelope {
            id: self.id,
            event_type: self.event,
            data: self.data,
            created_at: None,
        }
    }
}

/// Verify a Yapper webhook signature and deserialize the exact signed body.
///
/// `signature_header` is the value of `X-Yapper-Signature`, including the
/// `sha256=` prefix. The HMAC is calculated over the unmodified raw body using
/// the webhook secret configured under Settings → Developer → Advanced.
///
/// Do not parse and re-serialize the JSON before verification: even harmless
/// whitespace changes alter the signature. Do not log the secret or header.
pub fn verify_event_webhook(
    webhook_secret: &str,
    raw_body: &[u8],
    signature_header: &str,
) -> Result<EventWebhook, WebhookVerificationError> {
    let signature = signature_header
        .strip_prefix("sha256=")
        .ok_or(WebhookVerificationError::InvalidSignatureHeader)?;
    let signature = decode_hex_signature(signature)?;

    let key = hmac::Key::new(hmac::HMAC_SHA256, webhook_secret.as_bytes());
    hmac::verify(&key, raw_body, &signature)
        .map_err(|_| WebhookVerificationError::InvalidSignature)?;

    serde_json::from_slice(raw_body).map_err(|_| WebhookVerificationError::InvalidPayload)
}

fn decode_hex_signature(value: &str) -> Result<[u8; 32], WebhookVerificationError> {
    if value.len() != 64 {
        return Err(WebhookVerificationError::InvalidSignatureHeader);
    }

    let mut output = [0_u8; 32];
    let bytes = value.as_bytes();
    for (index, output_byte) in output.iter_mut().enumerate() {
        let high = decode_hex_digit(bytes[index * 2])
            .ok_or(WebhookVerificationError::InvalidSignatureHeader)?;
        let low = decode_hex_digit(bytes[index * 2 + 1])
            .ok_or(WebhookVerificationError::InvalidSignatureHeader)?;
        *output_byte = (high << 4) | low;
    }
    Ok(output)
}

fn decode_hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(secret: &[u8], body: &[u8]) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        let tag = hmac::sign(&key, body);
        let mut header = String::from("sha256=");
        for byte in tag.as_ref() {
            use std::fmt::Write as _;
            write!(&mut header, "{byte:02x}").unwrap();
        }
        header
    }

    #[test]
    fn verifies_signature_over_exact_raw_body() {
        let body = br#"{"id":1234,"event":"message","data":{"content":"hello"}}"#;
        let header = signature(b"webhook-secret", body);
        let event = verify_event_webhook("webhook-secret", body, &header).unwrap();
        assert_eq!(event.id, 1234);
        assert_eq!(event.event, "message");
        assert_eq!(event.data["content"], "hello");

        let envelope = event.into_event_envelope();
        assert_eq!(envelope.event_type, "message");
        assert_eq!(envelope.id, 1234);
    }

    #[test]
    fn rejects_wrong_secret_and_modified_body() {
        let body = br#"{"id":1,"event":"test","data":{}}"#;
        let header = signature(b"correct-secret", body);
        assert_eq!(
            verify_event_webhook("wrong-secret", body, &header),
            Err(WebhookVerificationError::InvalidSignature)
        );
        assert_eq!(
            verify_event_webhook("correct-secret", b"{}", &header),
            Err(WebhookVerificationError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_malformed_headers_and_signed_invalid_json() {
        assert_eq!(
            verify_event_webhook("secret", b"{}", "sha1=abc"),
            Err(WebhookVerificationError::InvalidSignatureHeader)
        );
        assert_eq!(
            verify_event_webhook("secret", b"{}", "sha256=1234"),
            Err(WebhookVerificationError::InvalidSignatureHeader)
        );

        let body = b"not-json";
        let header = signature(b"secret", body);
        assert_eq!(
            verify_event_webhook("secret", body, &header),
            Err(WebhookVerificationError::InvalidPayload)
        );
    }

    #[test]
    fn debug_output_redacts_interaction_tokens() {
        let body = br#"{"id":1,"event":"interaction","data":{"token":"hidden-secret"}}"#;
        let header = signature(b"secret", body);
        let event = verify_event_webhook("secret", body, &header).unwrap();
        let debug = format!("{event:?}");
        assert!(!debug.contains("hidden-secret"));
        assert!(debug.contains("[REDACTED]"));
    }
}
