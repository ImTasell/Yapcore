use serde::Serialize;

/// Supported initial interaction callback types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InteractionResponseType {
    /// Send a new message in the channel.
    Reply,
    /// Send a message visible only to the interaction user.
    Ephemeral,
    /// Update the message the interaction originated from.
    Update,
    /// Acknowledge without sending a visible message.
    Ack,
}
