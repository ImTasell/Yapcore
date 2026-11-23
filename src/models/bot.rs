use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::UserId;

/// Presence status accepted by Yapper's bot profile endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BotStatus {
    /// Show the bot as online.
    Online,
    /// Show the bot as idle.
    Idle,
    /// Show the bot as do-not-disturb.
    Dnd,
}

/// The profile returned by the Yapper GET /me endpoint.
///
/// The extra map preserves fields added by the platform that are not yet
/// represented by this version of Yapcore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BotProfile {
    /// Bot account ID.
    #[serde(default)]
    pub id: Option<UserId>,
    /// Bot's username.
    #[serde(default)]
    pub username: Option<String>,
    /// Optional display name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Optional bot bio/description.
    #[serde(default)]
    pub bio: Option<String>,
    /// Optional profile-image URL.
    #[serde(default)]
    pub avatar_url: Option<String>,
    /// Optional banner URL.
    #[serde(default)]
    pub banner_url: Option<String>,
    /// Fields not yet modelled by Yapcore.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unknown_profile_fields() {
        let profile: BotProfile = serde_json::from_value(serde_json::json!({
            "id": "123e4567-e89b-12d3-a456-426614174000",
            "username": "example-bot",
            "future_property": { "enabled": true }
        }))
        .unwrap();

        assert_eq!(profile.username.as_deref(), Some("example-bot"));
        assert_eq!(
            profile.extra.get("future_property"),
            Some(&serde_json::json!({ "enabled": true }))
        );
    }
}
