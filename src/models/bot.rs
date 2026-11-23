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

/// One-line bot activity text accepted by Yapper's presence endpoint.
///
/// Yapper treats this as display text, not a structured Discord activity
/// object. The platform currently limits the line to 60 Unicode characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity(String);

impl Activity {
    /// Create a custom activity line.
    pub fn new(value: impl Into<String>) -> crate::error::Result<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(crate::error::Error::EmptyBotActivity);
        }
        let actual = value.chars().count();
        if actual > 60 {
            return Err(crate::error::Error::BotActivityTooLong { actual });
        }
        Ok(Self(value))
    }

    /// Format a conventional `Playing …` activity line.
    ///
    /// This is a text convenience only; Yapper receives the resulting string.
    pub fn playing(thing: impl AsRef<str>) -> crate::error::Result<Self> {
        let thing = thing.as_ref().trim();
        if thing.is_empty() {
            return Err(crate::error::Error::EmptyBotActivity);
        }
        Self::new(format!("Playing {thing}"))
    }

    /// Return the exact activity line sent to Yapper.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Bot status and optional one-line activity sent to Yapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    status: BotStatus,
    activity: Option<Activity>,
}

impl Presence {
    /// Start a presence with an explicit online state.
    pub fn new(status: BotStatus) -> Self {
        Self {
            status,
            activity: None,
        }
    }

    /// Start an online presence.
    pub fn online() -> Self {
        Self::new(BotStatus::Online)
    }

    /// Start an idle presence.
    pub fn idle() -> Self {
        Self::new(BotStatus::Idle)
    }

    /// Start a do-not-disturb presence.
    pub fn dnd() -> Self {
        Self::new(BotStatus::Dnd)
    }

    /// Attach an activity line to this presence.
    pub fn activity(mut self, activity: Activity) -> Self {
        self.activity = Some(activity);
        self
    }

    /// Return the selected online state.
    pub fn status(&self) -> BotStatus {
        self.status
    }

    /// Return the activity line, if one is configured.
    pub fn activity_line(&self) -> Option<&Activity> {
        self.activity.as_ref()
    }
}
/// Configuration for cycling through a list of bot activity lines.
///
/// The first activity is applied immediately. Remaining activities are
/// applied one at a time at the configured interval, then the sequence
/// repeats from the beginning. Yapper receives each item as plain activity
/// text through its documented HTTP presence endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRotation {
    activities: Vec<Activity>,
    interval: std::time::Duration,
    status: BotStatus,
}

impl ActivityRotation {
    /// Create an online activity rotation.
    ///
    /// At least two activity entries and a non-zero interval are required.
    pub fn new<I>(activities: I, interval: std::time::Duration) -> crate::error::Result<Self>
    where
        I: IntoIterator<Item = Activity>,
    {
        let activities: Vec<_> = activities.into_iter().collect();
        if activities.len() < 2 {
            return Err(crate::error::Error::TooFewRotationActivities {
                actual: activities.len(),
            });
        }
        if interval.is_zero() {
            return Err(crate::error::Error::InvalidPresenceRotationInterval);
        }
        Ok(Self {
            activities,
            interval,
            status: BotStatus::Online,
        })
    }

    /// Set the online state used for every activity in this rotation.
    pub fn status(mut self, status: BotStatus) -> Self {
        self.status = status;
        self
    }

    /// Return the configured activities.
    pub fn activities(&self) -> &[Activity] {
        &self.activities
    }

    /// Return the interval between activity changes.
    pub fn interval(&self) -> std::time::Duration {
        self.interval
    }

    /// Return the status applied to every activity.
    pub fn bot_status(&self) -> BotStatus {
        self.status
    }
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

    #[test]
    fn formats_and_validates_activity_lines() {
        let activity = Activity::playing("Minecraft").unwrap();
        assert_eq!(activity.as_str(), "Playing Minecraft");
        assert_eq!(
            Activity::new("x".repeat(60))
                .unwrap()
                .as_str()
                .chars()
                .count(),
            60
        );
        assert!(matches!(
            Activity::new(" "),
            Err(crate::error::Error::EmptyBotActivity)
        ));
        assert!(matches!(
            Activity::new("x".repeat(61)),
            Err(crate::error::Error::BotActivityTooLong { actual: 61 })
        ));
        assert!(matches!(
            Activity::playing("x".repeat(53)),
            Err(crate::error::Error::BotActivityTooLong { actual: 61 })
        ));
    }

    #[test]
    fn validates_activity_rotation_configuration() {
        use std::time::Duration;

        assert!(matches!(
            ActivityRotation::new(Vec::<Activity>::new(), Duration::from_secs(30)),
            Err(crate::error::Error::TooFewRotationActivities { actual: 0 })
        ));
        assert!(matches!(
            ActivityRotation::new(
                vec![Activity::new("Only one").unwrap()],
                Duration::from_secs(30)
            ),
            Err(crate::error::Error::TooFewRotationActivities { actual: 1 })
        ));

        let activities = vec![
            Activity::playing("Minecraft").unwrap(),
            Activity::new("Monitoring servers").unwrap(),
        ];
        assert!(matches!(
            ActivityRotation::new(activities.clone(), Duration::ZERO),
            Err(crate::error::Error::InvalidPresenceRotationInterval)
        ));

        let rotation = ActivityRotation::new(activities, Duration::from_secs(30))
            .unwrap()
            .status(BotStatus::Idle);
        assert_eq!(rotation.activities().len(), 2);
        assert_eq!(rotation.interval(), Duration::from_secs(30));
        assert_eq!(rotation.bot_status(), BotStatus::Idle);
    }

    #[test]
    fn builds_presence_with_a_status_and_activity() {
        let presence = Presence::online().activity(Activity::playing("Chess").unwrap());
        assert_eq!(presence.status(), BotStatus::Online);
        assert_eq!(presence.activity_line().unwrap().as_str(), "Playing Chess");
    }
}
