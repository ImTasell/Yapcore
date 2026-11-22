use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{CategoryId, ChannelId, RoleId, ServerId, UserId};

/// Filter for text or voice channel listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    /// Text channels.
    Text,
    /// Voice channels.
    Voice,
}

impl ChannelType {
    pub(crate) fn as_query_value(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
        }
    }
}

/// A server the bot has been added to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Server {
    /// Server identifier.
    #[serde(default)]
    pub id: Option<ServerId>,
    /// Server display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Current member count, if returned by the endpoint.
    #[serde(default)]
    pub member_count: Option<u64>,
    /// Current online count, if live counts are enabled.
    #[serde(default)]
    pub online_count: Option<u64>,
    /// Additional fields not yet modelled by Yapcore.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A visible text or voice channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    /// Channel identifier.
    #[serde(default)]
    pub id: Option<ChannelId>,
    /// Channel name.
    #[serde(default)]
    pub name: Option<String>,
    /// Channel kind, normally text or voice.
    #[serde(default, rename = "type", alias = "kind")]
    pub channel_type: Option<String>,
    /// Owning server, when this is a server channel.
    #[serde(default)]
    pub server_id: Option<ServerId>,
    /// Category containing this channel.
    #[serde(default)]
    pub category_id: Option<CategoryId>,
    /// Sidebar ordering position.
    #[serde(default)]
    pub position: Option<i64>,
    /// Whether the channel is private.
    #[serde(default)]
    pub private: Option<bool>,
    /// Whether ordinary members may only read the channel.
    #[serde(default)]
    pub read_only: Option<bool>,
    /// Slowmode interval in seconds.
    #[serde(default)]
    pub slowmode: Option<u64>,
    /// Additional channel fields not yet modelled by Yapcore.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A channel category returned alongside server channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelCategory {
    /// Category identifier.
    #[serde(default)]
    pub id: Option<CategoryId>,
    /// Category name.
    #[serde(default)]
    pub name: Option<String>,
    /// Sidebar ordering position.
    #[serde(default)]
    pub position: Option<i64>,
    /// Additional category fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Channels and categories returned by a channel-list endpoint.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChannelListing {
    /// Visible channels in sidebar order.
    #[serde(default)]
    pub channels: Vec<Channel>,
    /// Categories in sidebar order, when returned by the endpoint.
    #[serde(default)]
    pub categories: Vec<ChannelCategory>,
}

/// A member of a server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Member {
    /// User identifier.
    #[serde(default, alias = "id")]
    pub user_id: Option<UserId>,
    /// Username.
    #[serde(default)]
    pub username: Option<String>,
    /// Server-specific nickname.
    #[serde(default)]
    pub nickname: Option<String>,
    /// Role IDs assigned to this member.
    #[serde(default, alias = "roles")]
    pub role_ids: Vec<RoleId>,
    /// Active timeout expiry, if present.
    #[serde(default)]
    pub timeout: Option<String>,
    /// Additional member fields not yet modelled by Yapcore.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A server role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Role {
    /// Role identifier.
    #[serde(default)]
    pub id: Option<RoleId>,
    /// Role display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Position in the server role hierarchy.
    #[serde(default)]
    pub position: Option<i64>,
    /// Additional role fields, including permissions not yet modelled.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// A public user profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    /// User identifier.
    #[serde(default)]
    pub id: Option<UserId>,
    /// Username.
    #[serde(default)]
    pub username: Option<String>,
    /// Optional display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Optional biography.
    #[serde(default)]
    pub bio: Option<String>,
    /// Avatar URL, when present.
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Banner URL, when present.
    #[serde(default)]
    pub banner_url: Option<String>,
    /// Additional profile fields not yet modelled by Yapcore.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_server_and_channel_fields_added_by_the_platform() {
        let server: Server = serde_json::from_value(serde_json::json!({
            "id": "123e4567-e89b-12d3-a456-426614174020",
            "name": "Example server",
            "member_count": 12,
            "future_server_flag": true
        }))
        .unwrap();

        assert_eq!(server.name.as_deref(), Some("Example server"));
        assert_eq!(
            server.extra.get("future_server_flag"),
            Some(&serde_json::json!(true))
        );

        let channel: Channel = serde_json::from_value(serde_json::json!({
            "id": "123e4567-e89b-12d3-a456-426614174021",
            "name": "general",
            "type": "text",
            "server_id": "123e4567-e89b-12d3-a456-426614174020",
            "slowmode": 5,
            "new_channel_property": "retained"
        }))
        .unwrap();

        assert_eq!(channel.channel_type.as_deref(), Some("text"));
        assert_eq!(channel.slowmode, Some(5));
        assert_eq!(
            channel.extra.get("new_channel_property"),
            Some(&serde_json::json!("retained"))
        );
    }

    #[test]
    fn deserializes_members_with_aliases_and_category_listing() {
        let member: Member = serde_json::from_value(serde_json::json!({
            "id": "123e4567-e89b-12d3-a456-426614174022",
            "username": "member",
            "roles": ["123e4567-e89b-12d3-a456-426614174023"]
        }))
        .unwrap();

        assert_eq!(member.username.as_deref(), Some("member"));
        assert_eq!(member.role_ids.len(), 1);

        let listing: ChannelListing = serde_json::from_value(serde_json::json!({
            "channels": [],
            "categories": [{
                "id": "123e4567-e89b-12d3-a456-426614174024",
                "name": "Main"
            }]
        }))
        .unwrap();
        assert_eq!(listing.categories[0].name.as_deref(), Some("Main"));
    }
}
