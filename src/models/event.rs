use serde::Deserialize;
use serde_json::Value;

/// A single event returned by Yapper's event polling endpoint.
///
/// The event name and payload remain open-ended so new platform event types
/// do not break older versions of Yapcore.
#[derive(Debug, Clone, PartialEq, Deserialize)]
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
    fn defaults_missing_event_collections_safely() {
        let batch: EventBatch = serde_json::from_value(serde_json::json!({
            "last": 99
        }))
        .unwrap();

        assert!(batch.is_empty());
        assert_eq!(batch.last(), 99);
    }
}
