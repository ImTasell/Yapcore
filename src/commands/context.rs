use crate::api::ApiClient;
use crate::error::Result;
use crate::ids::{ChannelId, MessageId, UserId};
use crate::models::Message;

/// Context passed to a registered prefix-command handler.
#[derive(Debug, Clone)]
pub struct CommandContext {
    api: ApiClient,
    command: String,
    content: String,
    args: String,
    argv: Vec<String>,
    message_id: MessageId,
    channel_id: ChannelId,
    author_id: Option<UserId>,
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
            message_id,
            channel_id,
            author_id,
        }
    }

    /// Name of the command that was invoked.
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Full content of the message that invoked the command.
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

    /// ID of the message that invoked the command.
    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }

    /// ID of the channel containing the command message.
    pub fn channel_id(&self) -> &ChannelId {
        &self.channel_id
    }

    /// ID of the message author, when supplied by Yapper.
    pub fn author_id(&self) -> Option<&UserId> {
        self.author_id.as_ref()
    }

    /// Reply to the message that invoked this command.
    pub async fn reply(&self, content: impl AsRef<str>) -> Result<Message> {
        self.api
            .reply_message(&self.channel_id, &self.message_id, content.as_ref())
            .await
    }

    /// Send a new message to the command's channel without a reply reference.
    pub async fn say(&self, content: impl AsRef<str>) -> Result<Message> {
        self.api
            .send_message(&self.channel_id, content.as_ref())
            .await
    }
}
