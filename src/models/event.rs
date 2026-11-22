use std::fmt;

use serde::Deserialize;
use serde_json::Value;

/// A single event returned by Yapper's event polling endpoint.
///
/// The event name and payload remain open-ended so new platform event types
/// do not break older versions of Yapcore.
#[derive(Clone, PartialEq, Deserialize)]
pub struct EventEnvelope {
    /// Monotonically increasing event sequence number.
    pub id: u64,
    /// Event name from the Yapper event envelope type field.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Event-specific payload, preserved as raw JSON.
    #[serde(default)]
    pub data: Value,
    /// Event creation timestamp, when supplied by Yapper.
    #[serde(default)]
    pub created_at: Option<String>,
}

impl fmt::Debug for EventEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EventEnvelope")
            .field("id", &self.id)
            .field("event_type", &self.event_type)
            .field("data", &redact_sensitive_fields(&self.data))
            .field("created_at", &self.created_at)
            .finish()
    }
}

fn redact_sensitive_fields(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let redacted = fields
                .iter()
                .map(|(key, field)| {
                    let normalized = key.to_ascii_lowercase();
                    let value = if matches!(
                        normalized.as_str(),
                        "token" | "interaction_token" | "webhook_secret" | "authorization"
                    ) {
                        Value::String("[REDACTED]".to_owned())
                    } else {
                        redact_sensitive_fields(field)
                    };

                    (key.clone(), value)
                })
                .collect();

            Value::Object(redacted)
        }
        Value::Array(values) => Value::Array(values.iter().map(redact_sensitive_fields).collect()),
        _ => value.clone(),
    }
}

/// A page returned by GET /events.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EventBatch {
    /// Events in server-delivered order.
    #[serde(default)]
    pub events: Vec<EventEnvelope>,
    /// Latest cursor represented by this response.
    #[serde(default)]
    pub last: u64,
}

impl EventBatch {
    /// Events in server-delivered order.
    pub fn events(&self) -> &[EventEnvelope] {
        &self.events
    }

    /// Latest event cursor represented by this response.
    pub fn last(&self) -> u64 {
        self.last
    }

    /// Whether the server returned no event payloads.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_event_names_and_payloads() {
        let batch: EventBatch = serde_json::from_value(serde_json::json!({
            "events": [{
                "id": 42,
                "type": "future_platform_event",
                "created_at": "2026-11-22T00:00:00Z",
                "data": {
                    "new_field": true,
                    "nested": { "value": 7 }
                }
            }],
            "last": 42
        }))
        .unwrap();

        assert_eq!(batch.last(), 42);
        assert_eq!(batch.events().len(), 1);
        assert_eq!(batch.events()[0].event_type, "future_platform_event");
        assert_eq!(
            batch.events()[0].data["nested"]["value"],
            serde_json::json!(7)
        );
    }

    #[test]
    fn debug_output_redacts_interaction_tokens_recursively() {
        let event: EventEnvelope = serde_json::from_value(serde_json::json!({
            "id": 43,
            "type": "command",
            "data": {
                "command": "ping",
                "token": "top-secret-token",
                "nested": {
                    "interaction_token": "nested-secret-token"
                }
            }
        }))
        .unwrap();

        let debug = format!("{event:?}");
        assert!(!debug.contains("top-secret-token"));
        assert!(!debug.contains("nested-secret-token"));
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("ping"));
    }

    #[test]
    fn defaults_missing_event_collections_safely() {
        let batch: EventBatch = serde_json::from_value(serde_json::json!({
            "last": 99
        }))
        .unwrap();

        assert!(batch.is_empty());
        assert_eq!(batch.last(), 99);
    }
}
