use std::sync::Arc;
use std::sync::atomic::AtomicU8;

use secrecy::SecretString;
use serde_json::Value;

use crate::api::ApiClient;
use crate::ids::{ChannelId, InteractionId, MessageId, UserId};
use crate::interactions::InteractionSession;
use crate::models::{InteractionResponseType, Modal};

/// Input data for one button, select, or modal interaction event.
#[derive(Debug, Clone)]
pub(crate) struct InteractionInvocation {
    pub(crate) id: InteractionId,
    pub(crate) token: SecretString,
    pub(crate) kind: String,
    pub(crate) custom_id: String,
    pub(crate) message_id: Option<MessageId>,
    pub(crate) channel_id: Option<ChannelId>,
    pub(crate) user_id: Option<UserId>,
    pub(crate) username: Option<String>,
    pub(crate) values: Value,
}

/// Context passed to button and other component interaction handlers.
#[derive(Debug, Clone)]
pub struct InteractionContext {
    api: ApiClient,
    session: InteractionSession,
    kind: String,
    custom_id: String,
    message_id: Option<MessageId>,
    channel_id: Option<ChannelId>,
    user_id: Option<UserId>,
    username: Option<String>,
    values: Value,
}

impl InteractionContext {
    pub(crate) fn new(api: ApiClient, invocation: InteractionInvocation) -> Self {
        Self {
            api,
            session: InteractionSession::new(invocation.id, invocation.token),
            kind: invocation.kind,
            custom_id: invocation.custom_id,
            message_id: invocation.message_id,
            channel_id: invocation.channel_id,
            user_id: invocation.user_id,
            username: invocation.username,
            values: invocation.values,
        }
    }

    /// The component's custom ID.
    pub fn custom_id(&self) -> &str {
        &self.custom_id
    }

    /// Interaction kind, normally button, select, or modal_submit.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Selected values for a select menu, or submitted field values for a modal.
    ///
    /// Select submissions are represented as a JSON array of strings. Modal
    /// submissions retain the server payload as JSON so text and select inputs
    /// remain forward-compatible.
    pub fn values(&self) -> &Value {
        &self.values
    }

    /// ID of the user who triggered the interaction, when supplied.
    pub fn user_id(&self) -> Option<&UserId> {
        self.user_id.as_ref()
    }

    /// Username of the user who triggered the interaction, when supplied.
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }

    /// ID of the message containing the component, when supplied.
    pub fn message_id(&self) -> Option<&MessageId> {
        self.message_id.as_ref()
    }

    /// ID of the channel containing the component, when supplied.
    pub fn channel_id(&self) -> Option<&ChannelId> {
        self.channel_id.as_ref()
    }

    /// Respond with a new message in the channel.
    pub async fn reply(&self, content: impl AsRef<str>) -> crate::Result<()> {
        self.session
            .respond(
                &self.api,
                InteractionResponseType::Reply,
                Some(content.as_ref()),
            )
            .await
    }

    /// Respond with a message visible only to the interaction user.
    pub async fn ephemeral(&self, content: impl AsRef<str>) -> crate::Result<()> {
        self.session
            .respond(
                &self.api,
                InteractionResponseType::Ephemeral,
                Some(content.as_ref()),
            )
            .await
    }

    /// Update the message that contained the component.
    pub async fn update(&self, content: impl AsRef<str>) -> crate::Result<()> {
        self.session
            .respond(
                &self.api,
                InteractionResponseType::Update,
                Some(content.as_ref()),
            )
            .await
    }

    /// Open a modal as this interaction's initial response.
    ///
    /// Modals are only accepted as the first interaction response.
    pub async fn show_modal(&self, modal: &Modal) -> crate::Result<()> {
        if self.kind == "modal_submit" {
            return Err(crate::Error::ModalNotAllowedFromModalSubmit);
        }
        self.session.respond_with_modal(&self.api, modal).await
    }

    /// Acknowledge the interaction without posting a visible message.
    pub async fn ack(&self) -> crate::Result<()> {
        self.session
            .respond(&self.api, InteractionResponseType::Ack, None)
            .await
    }

    pub(crate) fn response_count(&self) -> Arc<AtomicU8> {
        self.session.response_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[tokio::test]
    async fn modal_submission_cannot_open_another_modal() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let api = ApiClient::new(config).unwrap();
        let invocation = InteractionInvocation {
            id: InteractionId::new("123e4567-e89b-12d3-a456-426614174030").unwrap(),
            token: SecretString::from("modal/token"),
            kind: "modal_submit".to_owned(),
            custom_id: "application".to_owned(),
            message_id: None,
            channel_id: None,
            user_id: None,
            username: None,
            values: serde_json::json!({ "name": "Altix" }),
        };
        let context = InteractionContext::new(api, invocation);
        let modal = Modal::new(
            "follow-up",
            "Follow up",
            [TextInput::short("answer", "Answer").unwrap()],
        )
        .unwrap();

        assert!(matches!(
            context.show_modal(&modal).await,
            Err(crate::Error::ModalNotAllowedFromModalSubmit)
        ));
    }
}
