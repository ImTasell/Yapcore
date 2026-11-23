use std::sync::Arc;
use std::sync::atomic::AtomicU8;

use secrecy::SecretString;

use crate::api::ApiClient;
use crate::error::{Error, Result};
use crate::ids::{ChannelId, InteractionId, MessageId, UserId};
use crate::interactions::InteractionSession;
use crate::models::{InteractionResponseType, Modal};

/// Inputs required to create a slash-command context.
#[derive(Debug, Clone)]
pub(crate) struct SlashCommandInvocation {
    pub(crate) command: String,
    pub(crate) content: String,
    pub(crate) args: String,
    pub(crate) interaction_id: InteractionId,
    pub(crate) interaction_token: SecretString,
    pub(crate) channel_id: ChannelId,
    pub(crate) author_id: Option<UserId>,
}

/// Context passed to a registered prefix or slash-command handler.
#[derive(Debug, Clone)]
pub struct CommandContext {
    api: ApiClient,
    command: String,
    content: String,
    args: String,
    argv: Vec<String>,
    message_id: Option<MessageId>,
    channel_id: ChannelId,
    author_id: Option<UserId>,
    interaction: Option<InteractionSession>,
}

impl CommandContext {
    pub(crate) fn new(
        api: ApiClient,
        command: String,
        content: String,
        args: String,
        message_id: MessageId,
        channel_id: ChannelId,
        author_id: Option<UserId>,
    ) -> Self {
        let argv = args.split_whitespace().map(str::to_owned).collect();

        Self {
            api,
            command,
            content,
            args,
            argv,
            message_id: Some(message_id),
            channel_id,
            author_id,
            interaction: None,
        }
    }

    pub(crate) fn new_interaction(api: ApiClient, invocation: SlashCommandInvocation) -> Self {
        let interaction =
            InteractionSession::new(invocation.interaction_id, invocation.interaction_token);

        let argv = invocation
            .args
            .split_whitespace()
            .map(str::to_owned)
            .collect();

        Self {
            api,
            command: invocation.command,
            content: invocation.content,
            args: invocation.args,
            argv,
            message_id: None,
            channel_id: invocation.channel_id,
            author_id: invocation.author_id,
            interaction: Some(interaction),
        }
    }

    /// Name of the command that was invoked.
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Full content of the message or slash-command invocation.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Text after the command name.
    pub fn args(&self) -> &str {
        &self.args
    }

    /// Whitespace-separated arguments after the command name.
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// Parse this invocation against a typed option schema.
    ///
    /// This parser is local to Yapcore and does not alter the remote command
    /// synchronization payload. Named `name=value` and positional arguments
    /// are both accepted.
    pub fn parse_options(
        &self,
        schema: &[crate::models::CommandOption],
    ) -> std::result::Result<crate::models::ParsedCommandOptions, crate::models::CommandOptionError> {
        crate::models::ParsedCommandOptions::parse(schema, &self.args)
    }

    /// ID of the message that invoked this command, or None for slash commands.
    pub fn message_id(&self) -> Option<&MessageId> {
        self.message_id.as_ref()
    }

    /// ID of the channel containing this command.
    pub fn channel_id(&self) -> &ChannelId {
        &self.channel_id
    }

    /// ID of the command author, when supplied by Yapper.
    pub fn author_id(&self) -> Option<&UserId> {
        self.author_id.as_ref()
    }

    /// Whether this command came from Yapper's slash-command menu.
    pub fn is_interaction(&self) -> bool {
        self.interaction.is_some()
    }

    /// Reply to the command.
    ///
    /// Prefix commands create a quoted channel message. Slash commands use
    /// the interaction callback and answer the command invocation itself.
    pub async fn reply(&self, content: impl AsRef<str>) -> Result<()> {
        let content = content.as_ref();

        if self.interaction.is_some() {
            return self
                .answer_interaction(InteractionResponseType::Reply, Some(content))
                .await;
        }

        let message_id = self
            .message_id
            .as_ref()
            .ok_or(Error::MessageContextRequired)?;

        self.api
            .reply_message(&self.channel_id, message_id, content)
            .await?;
        Ok(())
    }

    /// Send a message in the command's channel.
    ///
    /// For prefix commands this is a normal unquoted message. For slash
    /// commands it is equivalent to reply, matching Yapper's Python SDK.
    pub async fn say(&self, content: impl AsRef<str>) -> Result<()> {
        let content = content.as_ref();

        if self.interaction.is_some() {
            return self
                .answer_interaction(InteractionResponseType::Reply, Some(content))
                .await;
        }

        self.api.send_message(&self.channel_id, content).await?;
        Ok(())
    }

    /// Send a response visible only to the person who invoked a slash command.
    pub async fn ephemeral(&self, content: impl AsRef<str>) -> Result<()> {
        self.answer_interaction(InteractionResponseType::Ephemeral, Some(content.as_ref()))
            .await
    }

    /// Open a modal as the slash command's initial response.
    ///
    /// Prefix commands do not have an interaction token and return
    /// Error::InteractionRequired from this method.
    pub async fn show_modal(&self, modal: &Modal) -> Result<()> {
        let Some(interaction) = self.interaction.as_ref() else {
            return Err(Error::InteractionRequired);
        };
        interaction.respond_with_modal(&self.api, modal).await
    }

    /// Acknowledge a slash command without posting a visible message.
    pub async fn ack(&self) -> Result<()> {
        self.answer_interaction(InteractionResponseType::Ack, None)
            .await
    }

    pub(crate) fn interaction_response_count(&self) -> Option<Arc<AtomicU8>> {
        self.interaction
            .as_ref()
            .map(InteractionSession::response_count)
    }

    async fn answer_interaction(
        &self,
        response_type: InteractionResponseType,
        content: Option<&str>,
    ) -> Result<()> {
        let Some(interaction) = self.interaction.as_ref() else {
            return Err(Error::InteractionRequired);
        };

        interaction.respond(&self.api, response_type, content).await
    }
}
