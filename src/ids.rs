use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// A value could not be used as a Yapper identifier.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("identifier must be a canonical UUID string")]
pub struct IdError;

fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();

    if bytes.len() != 36 {
        return false;
    }

    for (index, byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }

    true
}

macro_rules! define_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Validate and create an identifier.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();

                if is_uuid(&value) {
                    Ok(Self(value))
                } else {
                    Err(IdError)
                }
            }

            /// Return the identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Consume the identifier and return its owned string.
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(D::Error::custom)
            }
        }
    };
}

define_id!(UserId, "A Yapper user or bot identifier.");
define_id!(ServerId, "A Yapper server identifier.");
define_id!(ChannelId, "A Yapper channel identifier.");
define_id!(MessageId, "A Yapper message identifier.");
define_id!(RoleId, "A Yapper role identifier.");
define_id!(ThreadId, "A Yapper thread identifier.");
define_id!(CategoryId, "A Yapper channel-category identifier.");
define_id!(EmojiId, "A Yapper custom-emoji identifier.");
define_id!(WebhookId, "A Yapper webhook identifier.");

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_ID: &str = "123e4567-e89b-12d3-a456-426614174000";

    #[test]
    fn accepts_canonical_uuid_ids() {
        let id = UserId::new(VALID_ID).unwrap();
        assert_eq!(id.as_str(), VALID_ID);
        assert_eq!(id.to_string(), VALID_ID);
    }

    #[test]
    fn rejects_non_uuid_values() {
        assert_eq!(UserId::new("not-an-id"), Err(IdError));
        assert_eq!(ChannelId::new(""), Err(IdError));
        assert_eq!(ServerId::new("../../../me"), Err(IdError));
    }

    #[test]
    fn serializes_ids_as_strings() {
        let id = MessageId::new(VALID_ID).unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{VALID_ID}\""));
    }

    #[test]
    fn deserialization_validates_ids() {
        let id: RoleId = serde_json::from_str(&format!("\"{VALID_ID}\"")).unwrap();
        assert_eq!(id.as_str(), VALID_ID);

        let invalid = serde_json::from_str::<RoleId>("\"bad-id\"");
        assert!(invalid.is_err());
    }
}
