use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{CategoryId, ChannelId, RoleId, ServerId, UserId};

/// Filter for text or voice channel listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
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

/// Options used to create a channel.
///
/// Access lists are optional and are only sent when explicitly supplied. A
/// private channel can therefore be created with its initial role/user access
/// in the same request instead of briefly exposing it as public.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChannelCreateOptions {
    name: String,
    #[serde(rename = "kind")]
    channel_type: ChannelType,
    #[serde(skip_serializing_if = "Option::is_none")]
    category_id: Option<CategoryId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role_ids: Option<Vec<RoleId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_ids: Option<Vec<UserId>>,
}

impl ChannelCreateOptions {
    /// Start a channel creation request with a name and text/voice kind.
    pub fn new(name: impl Into<String>, channel_type: ChannelType) -> Self {
        Self {
            name: name.into(),
            channel_type,
            category_id: None,
            private: None,
            role_ids: None,
            user_ids: None,
        }
    }

    /// Place the new channel in a category.
    pub fn category(mut self, category_id: CategoryId) -> Self {
        self.category_id = Some(category_id);
        self
    }

    /// Set whether the channel is private.
    pub fn private(mut self, private: bool) -> Self {
        self.private = Some(private);
        self
    }

    /// Set the roles allowed to access a private channel.
    pub fn role_ids(mut self, role_ids: impl IntoIterator<Item = RoleId>) -> Self {
        self.role_ids = Some(role_ids.into_iter().collect());
        self
    }

    /// Set the users allowed to access a private channel (maximum 50).
    pub fn user_ids(mut self, user_ids: impl IntoIterator<Item = UserId>) -> Self {
        self.user_ids = Some(user_ids.into_iter().collect());
        self
    }

    pub(crate) fn validate(&self) -> crate::error::Result<()> {
        if self.name.trim().is_empty() {
            return Err(crate::error::Error::EmptyChannelName);
        }
        if let Some(users) = &self.user_ids {
            if users.len() > 50 {
                return Err(crate::error::Error::TooManyChannelUsers {
                    actual: users.len(),
                });
            }
        }
        Ok(())
    }
}

/// Fields that may be changed on an existing channel.
///
/// Unset fields are omitted. clear_category() explicitly sends a JSON null,
/// and empty access lists are preserved so callers can revoke all listed access.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ChannelEditOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category_id: Option<Option<CategoryId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    role_ids: Option<Vec<RoleId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    read_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    post_role_ids: Option<Vec<RoleId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slowmode: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_ids: Option<Vec<UserId>>,
}

impl ChannelEditOptions {
    /// Start an empty channel edit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Rename the channel.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Move the channel to a category.
    pub fn category(mut self, category_id: CategoryId) -> Self {
        self.category_id = Some(Some(category_id));
        self
    }

    /// Remove the channel from its current category.
    pub fn clear_category(mut self) -> Self {
        self.category_id = Some(None);
        self
    }

    /// Set whether the channel is private.
    pub fn private(mut self, private: bool) -> Self {
        self.private = Some(private);
        self
    }

    /// Replace the roles allowed to view a private channel.
    pub fn role_ids(mut self, role_ids: impl IntoIterator<Item = RoleId>) -> Self {
        self.role_ids = Some(role_ids.into_iter().collect());
        self
    }

    /// Set whether the channel is read-only for ordinary members.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = Some(read_only);
        self
    }

    /// Replace the roles allowed to post in a read-only channel.
    pub fn post_role_ids(mut self, role_ids: impl IntoIterator<Item = RoleId>) -> Self {
        self.post_role_ids = Some(role_ids.into_iter().collect());
        self
    }

    /// Set slowmode in seconds (maximum 21,600).
    pub fn slowmode(mut self, seconds: u64) -> Self {
        self.slowmode = Some(seconds);
        self
    }

    /// Replace the users allowed to view a private channel (maximum 50).
    pub fn user_ids(mut self, user_ids: impl IntoIterator<Item = UserId>) -> Self {
        self.user_ids = Some(user_ids.into_iter().collect());
        self
    }

    /// Whether this edit contains any fields.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.category_id.is_none()
            && self.private.is_none()
            && self.role_ids.is_none()
            && self.read_only.is_none()
            && self.post_role_ids.is_none()
            && self.slowmode.is_none()
            && self.user_ids.is_none()
    }

    pub(crate) fn validate(&self) -> crate::error::Result<()> {
        if self.is_empty() {
            return Err(crate::error::Error::EmptyChannelEditOptions);
        }
        if self
            .name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(crate::error::Error::EmptyChannelName);
        }
        if let Some(seconds) = self.slowmode {
            if seconds > 21_600 {
                return Err(crate::error::Error::InvalidChannelSlowmode { actual: seconds });
            }
        }
        if let Some(users) = &self.user_ids {
            if users.len() > 50 {
                return Err(crate::error::Error::TooManyChannelUsers {\n                    actual: users.len(),\n                });
            }
        }
        Ok(())
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

/// A record describing a user banned from a server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ban {
    /// Banned user identifier.
    #[serde(default)]
    pub user_id: Option<UserId>,
    /// User identifier of the moderator who created the ban.
    #[serde(default)]
    pub banned_by: Option<UserId>,
    /// When the ban was created.
    #[serde(default)]
    pub created_at: Option<String>,
    /// Public profile of the banned user, when included by Yapper.
    #[serde(default)]
    pub user: Option<UserProfile>,
    /// Additional ban fields not yet modelled by Yapcore.
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
    /// Optional role color, usually a CSS-style hexadecimal string.
    #[serde(default)]
    pub color: Option<String>,
    /// Whether the role may be mentioned by members.
    #[serde(default)]
    pub mentionable: Option<bool>,
    /// Permission map returned by Yapper.
    #[serde(default)]
    pub permissions: Option<Value>,
    /// Additional role fields not yet modelled by Yapcore.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Optional fields for editing a role.
///
/// Fields left unset are omitted from the PATCH payload. Use clear_color()
/// to explicitly remove an existing color.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RoleEditOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mentionable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    permissions: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<i64>,
}

impl RoleEditOptions {
    /// Start an empty role edit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Change the role name.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the role color.
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Remove the current role color.
    pub fn clear_color(mut self) -> Self {
        self.color = Some(String::new());
        self
    }

    /// Change whether the role is mentionable.
    pub fn mentionable(mut self, mentionable: bool) -> Self {
        self.mentionable = Some(mentionable);
        self
    }

    /// Replace the supplied permission keys.
    pub fn permissions(mut self, permissions: Value) -> Self {
        self.permissions = Some(permissions);
        self
    }

    /// Set the role's requested hierarchy position.
    pub fn position(mut self, position: i64) -> Self {
        self.position = Some(position);
        self
    }

    /// Whether this edit contains any fields.
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.color.is_none()
            && self.mentionable.is_none()
            && self.permissions.is_none()
            && self.position.is_none()
    }
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
