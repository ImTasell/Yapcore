use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::ids::{ChannelId, MessageId, UserId};

/// A Yapper channel message.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Message {
    /// Message identifier.
    pub id: MessageId,
    /// Channel containing the message.
    pub channel_id: ChannelId,
    /// Text content, when present.
    #[serde(default)]
    pub content: Option<String>,
    /// Author ID, when returned by the endpoint.
    #[serde(default)]
    pub author_id: Option<UserId>,
    /// Creation timestamp, when returned by the endpoint.
    #[serde(default)]
    pub created_at: Option<String>,
    /// Additional server fields not yet modelled by Yapcore.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";
    const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";

    #[test]
    fn deserializes_message_and_keeps_unknown_fields() {
        let message: Message = serde_json::from_value(serde_json::json!({
            "id": MESSAGE_ID,
            "channel_id": CHANNEL_ID,
            "content": "hello",
            "future_field": "preserved"
        }))
        .unwrap();

        assert_eq!(message.id.as_str(), MESSAGE_ID);
        assert_eq!(message.channel_id.as_str(), CHANNEL_ID);
        assert_eq!(message.content.as_deref(), Some("hello"));
        assert_eq!(
            message.extra.get("future_field"),
            Some(&serde_json::json!("preserved"))
        );
    }
}
