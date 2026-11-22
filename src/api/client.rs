use std::sync::Arc;

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{Client, Method, Response, Url};
use secrecy::ExposeSecret;
use serde_json::Value;

use crate::config::Config;
use crate::error::{ApiError, Error, Result, parse_retry_after, sanitize_transport_error};
use crate::ids::InteractionId;
use crate::models::{
    Ban, BotProfile, Channel, ChannelListing, ChannelType, CommandDefinition, Embed, EventBatch,
    InteractionResponseType, Member, Message, Role, Server, UserProfile, validate_action_rows,
    validate_command_list,
};

/// Cloneable authenticated client for Yapper's documented bot REST API.
#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    config: Arc<Config>,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApiClient")
            .field("api_base_url", self.config.api_base_url())
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    /// Create a client using a validated configuration.
    ///
    /// The underlying HTTP connection pool is reused across requests.
    pub fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);

        let authorization_value = format!("Bot {}", config.token().expose_secret());
        let mut authorization = HeaderValue::from_str(&authorization_value)
            .map_err(|_| Error::InvalidAuthorizationHeader)?;
        authorization.set_sensitive(true);

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .timeout(config.request_timeout())
            .redirect(Policy::none())
            .default_headers(headers)
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(sanitize_transport_error)?;

        Ok(Self { client, config })
    }

    /// Fetch the current bot profile from GET /me.
    pub async fn get_me(&self) -> Result<BotProfile> {
        let value = self.request_json(Method::GET, "me", None).await?;
        serde_json::from_value(value).map_err(Error::from)
    }

    /// Send a plain-text message to a channel.
    ///
    /// Message content is validated against Yapper's 2,000-character limit.
    /// This operation is not automatically retried because it is a side effect.
    pub async fn send_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: &str,
    ) -> Result<Message> {
        self.send_message_inner(channel_id, content, None).await
    }

    /// Send a message with optional text content and up to five embeds.
    ///
    /// At least one non-empty content string or one embed is required. The
    /// server remains authoritative for image-host allowlists and other
    /// endpoint-specific validation.
    pub async fn send_message_with_embeds(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: Option<&str>,
        embeds: &[Embed],
    ) -> Result<Message> {
        self.send_rich_message(channel_id, content, embeds, &[])
            .await
    }

    /// Send a message with button action rows.
    ///
    /// The platform requires visible text or at least one embed alongside
    /// components; use send_rich_message when combining buttons and embeds.
    pub async fn send_message_with_components(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: Option<&str>,
        components: &[crate::models::ActionRow],
    ) -> Result<Message> {
        self.send_rich_message(channel_id, content, &[], components)
            .await
    }

    /// Send a message containing any supported combination of text, embeds and buttons.
    pub async fn send_rich_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: Option<&str>,
        embeds: &[Embed],
        components: &[crate::models::ActionRow],
    ) -> Result<Message> {
        if embeds.len() > 5 {
            return Err(Error::TooManyEmbeds {
                actual: embeds.len(),
            });
        }

        let content = content.filter(|value| !value.is_empty());
        if let Some(content) = content {
            let actual = content.chars().count();
            if actual > 2_000 {
                return Err(Error::MessageContentTooLong { actual, max: 2_000 });
            }
        }

        if content.is_none() && embeds.is_empty() {
            return Err(Error::EmptyMessagePayload);
        }

        let mut embed_text = 0_usize;
        for embed in embeds {
            embed.validate()?;
            embed_text += embed.text_characters();
        }
        if embed_text > 6_000 {
            return Err(Error::MessageEmbedsTooLong { actual: embed_text });
        }

        validate_action_rows(components)?;

        let mut body = serde_json::Map::new();
        if let Some(content) = content {
            body.insert("content".to_owned(), serde_json::json!(content));
        }
        if !embeds.is_empty() {
            body.insert("embeds".to_owned(), serde_json::to_value(embeds)?);
        }
        if !components.is_empty() {
            body.insert("components".to_owned(), serde_json::to_value(components)?);
        }

        let route = format!("channels/{channel_id}/messages");
        let response = self
            .request_json(Method::POST, &route, Some(Value::Object(body)))
            .await?;

        let message = response.get("message").cloned().unwrap_or(response);
        serde_json::from_value(message).map_err(Error::from)
    }

    /// Reply to an existing message in a channel.
    ///
    /// The reply reference is included in the request body as reply_to.
    pub async fn reply_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        content: &str,
    ) -> Result<Message> {
        self.send_message_inner(channel_id, content, Some(message_id))
            .await
    }

    async fn send_message_inner(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: &str,
        reply_to: Option<&crate::ids::MessageId>,
    ) -> Result<Message> {
        if content.is_empty() {
            return Err(Error::EmptyMessageContent);
        }

        let character_count = content.chars().count();
        if character_count > 2_000 {
            return Err(Error::MessageContentTooLong {
                actual: character_count,
                max: 2_000,
            });
        }

        let route = format!("channels/{channel_id}/messages");
        let mut body = serde_json::json!({ "content": content });
        if let Some(message_id) = reply_to {
            body["reply_to"] = serde_json::json!(message_id.as_str());
        }
        let response = self.request_json(Method::POST, &route, Some(body)).await?;

        let message = response.get("message").cloned().unwrap_or(response);
        serde_json::from_value(message).map_err(Error::from)
    }

    /// List servers the bot has been added to.
    pub async fn list_servers(&self) -> Result<Vec<Server>> {
        let response = self.request_json(Method::GET, "servers", None).await?;
        let servers = response.get("servers").cloned().unwrap_or(response);
        serde_json::from_value(servers).map_err(Error::from)
    }

    /// Fetch details for one server.
    pub async fn get_server(&self, server_id: &crate::ids::ServerId) -> Result<Server> {
        let route = format!("servers/{server_id}");
        let response = self.request_json(Method::GET, &route, None).await?;
        let server = response.get("server").cloned().unwrap_or(response);
        serde_json::from_value(server).map_err(Error::from)
    }

    /// List visible channels globally or within one server.
    ///
    /// The server-specific response also includes its categories.
    pub async fn list_channels(
        &self,
        server_id: Option<&crate::ids::ServerId>,
        channel_type: Option<ChannelType>,
    ) -> Result<ChannelListing> {
        let route = match server_id {
            Some(server_id) => format!("servers/{server_id}/channels"),
            None => "channels".to_owned(),
        };
        let mut url = self
            .config
            .api_base_url()
            .join(&route)
            .map_err(|_| Error::InvalidApiRoute(route.clone()))?;

        if let Some(channel_type) = channel_type {
            url.query_pairs_mut()
                .append_pair("type", channel_type.as_query_value());
        }

        let response = self.request_json_url(Method::GET, url, None).await?;
        serde_json::from_value(response).map_err(Error::from)
    }

    /// Fetch one visible channel.
    pub async fn get_channel(&self, channel_id: &crate::ids::ChannelId) -> Result<Channel> {
        let route = format!("channels/{channel_id}");
        let response = self.request_json(Method::GET, &route, None).await?;
        let channel = response.get("channel").cloned().unwrap_or(response);
        serde_json::from_value(channel).map_err(Error::from)
    }

    /// List server members in user-ID order.
    ///
    /// The server defaults to 200 members and accepts at most 500 per page.
    /// The after cursor is the last user ID from the previous page.
    pub async fn list_members(
        &self,
        server_id: &crate::ids::ServerId,
        limit: Option<usize>,
        after: Option<&crate::ids::UserId>,
    ) -> Result<Vec<Member>> {
        if let Some(limit) = limit {
            if !(1..=500).contains(&limit) {
                return Err(Error::InvalidMemberPageLimit { actual: limit });
            }
        }

        let mut url = self
            .config
            .api_base_url()
            .join(&format!("servers/{server_id}/members"))
            .map_err(|_| Error::InvalidApiRoute("members".to_owned()))?;

        {
            let mut query = url.query_pairs_mut();
            if let Some(limit) = limit {
                query.append_pair("limit", &limit.to_string());
            }
            if let Some(after) = after {
                query.append_pair("after", after.as_str());
            }
        }

        let response = self.request_json_url(Method::GET, url, None).await?;
        let members = response.get("members").cloned().unwrap_or(response);
        serde_json::from_value(members).map_err(Error::from)
    }

    /// Fetch one server member, including nickname, role IDs and timeout data.
    pub async fn get_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
    ) -> Result<Member> {
        let route = format!("servers/{server_id}/members/{user_id}");
        let response = self.request_json(Method::GET, &route, None).await?;
        let member = response.get("member").cloned().unwrap_or(response);
        serde_json::from_value(member).map_err(Error::from)
    }

    /// List roles in a server.
    pub async fn list_roles(&self, server_id: &crate::ids::ServerId) -> Result<Vec<Role>> {
        let route = format!("servers/{server_id}/roles");
        let response = self.request_json(Method::GET, &route, None).await?;
        let roles = response.get("roles").cloned().unwrap_or(response);
        serde_json::from_value(roles).map_err(Error::from)
    }

    /// Fetch one role in a server.
    ///
    /// This route exists in the upstream Python framework but is not listed in
    /// the current public endpoint overview; treat it as a source-parity route.
    pub async fn get_role(
        &self,
        server_id: &crate::ids::ServerId,
        role_id: &crate::ids::RoleId,
    ) -> Result<Role> {
        let route = format!("servers/{server_id}/roles/{role_id}");
        let response = self.request_json(Method::GET, &route, None).await?;
        let role = response.get("role").cloned().unwrap_or(response);
        serde_json::from_value(role).map_err(Error::from)
    }

    /// Create a role below the bot's highest role.
    ///
    /// This route is supported by the Python source but requires live contract
    /// verification against the current Yapper API.
    pub async fn create_role(
        &self,
        server_id: &crate::ids::ServerId,
        name: &str,
        color: Option<&str>,
        mentionable: bool,
        permissions: Option<&Value>,
    ) -> Result<Role> {
        let mut body = serde_json::Map::new();
        body.insert("name".to_owned(), serde_json::json!(name));
        body.insert("mentionable".to_owned(), serde_json::json!(mentionable));
        if let Some(color) = color {
            body.insert("color".to_owned(), serde_json::json!(color));
        }
        if let Some(permissions) = permissions {
            body.insert("permissions".to_owned(), permissions.clone());
        }

        let route = format!("servers/{server_id}/roles");
        let response = self
            .request_json(Method::POST, &route, Some(Value::Object(body)))
            .await?;
        let role = response.get("role").cloned().unwrap_or(response);
        serde_json::from_value(role).map_err(Error::from)
    }

    /// Edit selected fields of a role; fields set to None remain unchanged.
    ///
    /// Pass an empty color string to clear the role color.
    pub async fn edit_role(
        &self,
        server_id: &crate::ids::ServerId,
        role_id: &crate::ids::RoleId,
        name: Option<&str>,
        color: Option<&str>,
        mentionable: Option<bool>,
        permissions: Option<&Value>,
        position: Option<i64>,
    ) -> Result<Role> {
        let mut body = serde_json::Map::new();
        if let Some(name) = name {
            body.insert("name".to_owned(), serde_json::json!(name));
        }
        if let Some(color) = color {
            body.insert("color".to_owned(), serde_json::json!(color));
        }
        if let Some(mentionable) = mentionable {
            body.insert("mentionable".to_owned(), serde_json::json!(mentionable));
        }
        if let Some(permissions) = permissions {
            body.insert("permissions".to_owned(), permissions.clone());
        }
        if let Some(position) = position {
            body.insert("position".to_owned(), serde_json::json!(position));
        }

        let route = format!("servers/{server_id}/roles/{role_id}");
        let response = self
            .request_json(Method::PATCH, &route, Some(Value::Object(body)))
            .await?;
        let role = response.get("role").cloned().unwrap_or(response);
        serde_json::from_value(role).map_err(Error::from)
    }

    /// Delete a role in a server.
    pub async fn delete_role(
        &self,
        server_id: &crate::ids::ServerId,
        role_id: &crate::ids::RoleId,
    ) -> Result<()> {
        let route = format!("servers/{server_id}/roles/{role_id}");
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
    }

    /// Reorder roles from highest to lowest using their IDs.
    ///
    /// Roles omitted by the request retain their relative order below the
    /// supplied roles, subject to the bot's own role hierarchy.
    pub async fn reorder_roles(
        &self,
        server_id: &crate::ids::ServerId,
        role_ids: &[crate::ids::RoleId],
    ) -> Result<Vec<Role>> {
        let route = format!("servers/{server_id}/roles");
        let body = serde_json::json!({
            "ids": role_ids.iter().map(crate::ids::RoleId::as_str).collect::<Vec<_>>()
        });
        let response = self
            .request_json(Method::PATCH, &route, Some(body))
            .await?;
        let roles = response.get("roles").cloned().unwrap_or(response);
        serde_json::from_value(roles).map_err(Error::from)
    }

    /// Fetch one public user profile.
    pub async fn get_user(&self, user_id: &crate::ids::UserId) -> Result<UserProfile> {
        let route = format!("users/{user_id}");
        let response = self.request_json(Method::GET, &route, None).await?;
        let user = response.get("user").cloned().unwrap_or(response);
        serde_json::from_value(user).map_err(Error::from)
    }

    /// Add a role to a server member. Requires Manage Roles permission.
    pub async fn add_role(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
        role_id: &crate::ids::RoleId,
    ) -> Result<()> {
        let route = format!("servers/{server_id}/members/{user_id}/roles/{role_id}");
        self.request_json(Method::PUT, &route, None).await?;
        Ok(())
    }

    /// Remove a role from a server member. Requires Manage Roles permission.
    pub async fn remove_role(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
        role_id: &crate::ids::RoleId,
    ) -> Result<()> {
        let route = format!("servers/{server_id}/members/{user_id}/roles/{role_id}");
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
    }

    /// Set a server member's nickname; an empty string clears it.
    ///
    /// Returns the raw endpoint response because Yapper's response wrapper is
    /// not consistently specified across API guide revisions.
    pub async fn set_nickname(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
        nickname: &str,
    ) -> Result<Value> {
        let actual = nickname.chars().count();
        if actual > 32 {
            return Err(Error::NicknameTooLong { actual });
        }

        let route = format!("servers/{server_id}/members/{user_id}");
        let body = serde_json::json!({ "nickname": nickname });
        self.request_json(Method::PATCH, &route, Some(body)).await
    }

    /// Kick a member from a server. Requires Kick Members permission.
    pub async fn kick_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
    ) -> Result<Value> {
        let route = format!("servers/{server_id}/kick");
        let body = serde_json::json!({ "user_id": user_id.as_str() });
        self.request_json(Method::POST, &route, Some(body)).await
    }

    /// Ban a member from a server. Requires Ban Members permission.
    pub async fn ban_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
    ) -> Result<Value> {
        let route = format!("servers/{server_id}/bans");
        let body = serde_json::json!({ "user_id": user_id.as_str() });
        self.request_json(Method::POST, &route, Some(body)).await
    }

    /// List bans in a server. Requires Ban Members permission.
    pub async fn list_bans(&self, server_id: &crate::ids::ServerId) -> Result<Vec<Ban>> {
        let route = format!("servers/{server_id}/bans");
        let response = self.request_json(Method::GET, &route, None).await?;
        let bans = response.get("bans").cloned().unwrap_or(response);
        serde_json::from_value(bans).map_err(Error::from)
    }

    /// Remove a server ban. Requires Ban Members permission.
    pub async fn unban_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
    ) -> Result<()> {
        let route = format!("servers/{server_id}/bans/{user_id}");
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
    }

    /// Set a member timeout. Use zero minutes to clear the timeout.
    ///
    /// Yapper defaults an omitted duration to ten minutes and allows up to
    /// 40,320 minutes (28 days). A reason is optional and capped at 200 chars.
    pub async fn timeout_member(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
        minutes: Option<u32>,
        reason: Option<&str>,
    ) -> Result<Value> {
        if let Some(minutes) = minutes {
            if minutes > 40_320 {
                return Err(Error::TimeoutTooLong { actual: minutes });
            }
        }

        if let Some(reason) = reason {
            let actual = reason.chars().count();
            if actual > 200 {
                return Err(Error::ModerationReasonTooLong { actual });
            }
        }

        let mut body = serde_json::Map::new();
        body.insert("user_id".to_owned(), serde_json::json!(user_id.as_str()));
        if let Some(minutes) = minutes {
            body.insert("minutes".to_owned(), serde_json::json!(minutes));
        }
        if let Some(reason) = reason {
            body.insert("reason".to_owned(), serde_json::json!(reason));
        }

        let route = format!("servers/{server_id}/timeouts");
        self.request_json(Method::POST, &route, Some(Value::Object(body)))
            .await
    }

    /// Fetch recent channel messages in oldest-first order.
    ///
    /// The server defaults to 50 messages when limit is None and caps pages
    /// at 100. The optional before cursor is an ISO-8601 timestamp.
    pub async fn get_recent_messages(
        &self,
        channel_id: &crate::ids::ChannelId,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<Vec<Message>> {
        if let Some(limit) = limit {
            if !(1..=100).contains(&limit) {
                return Err(Error::InvalidMessagePageLimit { actual: limit });
            }
        }

        let mut url = self
            .config
            .api_base_url()
            .join(&format!("channels/{channel_id}/messages"))
            .map_err(|_| Error::InvalidApiRoute("messages".to_owned()))?;

        {
            let mut query = url.query_pairs_mut();
            if let Some(limit) = limit {
                query.append_pair("limit", &limit.to_string());
            }
            if let Some(before) = before {
                query.append_pair("before", before);
            }
        }

        let response = self.request_json_url(Method::GET, url, None).await?;
        let messages = response.get("messages").cloned().unwrap_or(response);
        serde_json::from_value(messages).map_err(Error::from)
    }

    /// Fetch messages newer than the supplied ISO-8601 timestamp, oldest first.
    pub async fn get_messages_after(
        &self,
        channel_id: &crate::ids::ChannelId,
        after: &str,
    ) -> Result<Vec<Message>> {
        if after.is_empty() {
            return Err(Error::EmptyMessageCursor);
        }

        let mut url = self
            .config
            .api_base_url()
            .join(&format!("channels/{channel_id}/messages"))
            .map_err(|_| Error::InvalidApiRoute("messages".to_owned()))?;
        url.query_pairs_mut().append_pair("after", after);

        let response = self.request_json_url(Method::GET, url, None).await?;
        let messages = response.get("messages").cloned().unwrap_or(response);
        serde_json::from_value(messages).map_err(Error::from)
    }

    /// Fetch one message from a channel.
    pub async fn get_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
    ) -> Result<Message> {
        let route = format!("channels/{channel_id}/messages/{message_id}");
        let response = self.request_json(Method::GET, &route, None).await?;
        let message = response.get("message").cloned().unwrap_or(response);
        serde_json::from_value(message).map_err(Error::from)
    }

    /// Edit the text content of a message.
    ///
    /// The method intentionally edits only content; omitted embed/component
    /// fields remain untouched on the server.
    pub async fn edit_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        content: &str,
    ) -> Result<Message> {
        let character_count = content.chars().count();
        if character_count > 2_000 {
            return Err(Error::MessageContentTooLong {
                actual: character_count,
                max: 2_000,
            });
        }

        let route = format!("channels/{channel_id}/messages/{message_id}");
        let body = serde_json::json!({ "content": content });
        let response = self.request_json(Method::PATCH, &route, Some(body)).await?;
        let message = response.get("message").cloned().unwrap_or(response);
        serde_json::from_value(message).map_err(Error::from)
    }

    /// Delete a message.
    ///
    /// Deleting another user's message requires Manage Messages permission.
    pub async fn delete_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
    ) -> Result<()> {
        let route = format!("channels/{channel_id}/messages/{message_id}");
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
    }

    /// List reactions attached to a message.
    pub async fn get_reactions(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
    ) -> Result<Value> {
        let route = format!("channels/{channel_id}/messages/{message_id}/reactions");
        self.request_json(Method::GET, &route, None).await
    }

    /// Pin a message. Requires Manage Messages permission.
    pub async fn pin_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
    ) -> Result<()> {
        let route = format!("channels/{channel_id}/messages/{message_id}/pin");
        self.request_json(Method::PUT, &route, None).await?;
        Ok(())
    }

    /// Unpin a message. Requires Manage Messages permission.
    pub async fn unpin_message(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
    ) -> Result<()> {
        let route = format!("channels/{channel_id}/messages/{message_id}/pin");
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
    }

    /// Bulk-delete between 1 and 100 messages in a channel.
    ///
    /// The API may skip messages that are too old or otherwise ineligible;
    /// its deleted/skipped result is returned as raw JSON.
    pub async fn bulk_delete_messages(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_ids: &[crate::ids::MessageId],
    ) -> Result<Value> {
        if message_ids.is_empty() || message_ids.len() > 100 {
            return Err(Error::InvalidBulkDeleteCount {
                actual: message_ids.len(),
            });
        }

        let route = format!("channels/{channel_id}/messages/bulk-delete");
        let body = serde_json::json!({ "ids": message_ids });
        self.request_json(Method::POST, &route, Some(body)).await
    }

    /// Add the bot's reaction to a message.
    pub async fn add_reaction(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        emoji: &str,
    ) -> Result<()> {
        self.set_reaction(Method::PUT, channel_id, message_id, emoji)
            .await
    }

    /// Remove the bot's reaction from a message.
    pub async fn remove_reaction(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        emoji: &str,
    ) -> Result<()> {
        self.set_reaction(Method::DELETE, channel_id, message_id, emoji)
            .await
    }

    async fn set_reaction(
        &self,
        method: Method,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        emoji: &str,
    ) -> Result<()> {
        if emoji.is_empty() {
            return Err(Error::InvalidEmoji);
        }

        let mut url = self
            .config
            .api_base_url()
            .join("channels/")
            .map_err(|_| Error::InvalidApiRoute("channels".to_owned()))?;

        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::InvalidApiRoute("channels".to_owned()))?;
            segments
                .pop_if_empty()
                .push(channel_id.as_str())
                .push("messages")
                .push(message_id.as_str())
                .push("reactions")
                .push(emoji);
        }

        self.request_json_url(method, url, None).await?;
        Ok(())
    }

    /// Fetch one page of events using Yapper's documented long-poll endpoint.
    ///
    /// wait_seconds is capped at 25 seconds. Optional max_age_seconds
    /// filters events older than the supplied age and is capped at one day.
    pub async fn poll_events(
        &self,
        after: u64,
        wait_seconds: u8,
        max_age_seconds: Option<u32>,
    ) -> Result<EventBatch> {
        if wait_seconds > 25 {
            return Err(Error::InvalidEventPollWait);
        }

        if max_age_seconds.is_some_and(|age| age > 86_400) {
            return Err(Error::InvalidEventMaxAge);
        }

        let mut url = self
            .config
            .api_base_url()
            .join("events")
            .map_err(|_| Error::InvalidApiRoute("events".to_owned()))?;

        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("after", &after.to_string())
                .append_pair("wait", &wait_seconds.to_string());

            if let Some(max_age) = max_age_seconds {
                query.append_pair("max_age", &max_age.to_string());
            }
        }

        let value = self.request_json_url(Method::GET, url, None).await?;
        serde_json::from_value(value).map_err(Error::from)
    }

    /// Answer a slash-command or button interaction.
    ///
    /// The token is kept in a secret wrapper and inserted as a URL path
    /// segment, which ensures reserved characters are percent-encoded.
    /// Callback requests are never automatically retried.
    pub(crate) async fn respond_interaction(
        &self,
        interaction_id: &InteractionId,
        interaction_token: &secrecy::SecretString,
        response_type: InteractionResponseType,
        content: Option<&str>,
    ) -> Result<Value> {
        let mut url = self
            .config
            .api_base_url()
            .join("interactions/")
            .map_err(|_| Error::InvalidApiRoute("interactions".to_owned()))?;

        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Error::InvalidApiRoute("interactions".to_owned()))?;
            segments
                .pop_if_empty()
                .push(interaction_id.as_str())
                .push(interaction_token.expose_secret())
                .push("callback");
        }

        let mut body = serde_json::json!({ "type": response_type });
        if let Some(content) = content {
            body["content"] = serde_json::json!(content);
        }

        self.request_json_url(Method::POST, url, Some(body)).await
    }

    /// Replace the bot's complete registered command list with PUT /commands.
    ///
    /// Yapper replaces the full list in one request. The list is validated
    /// locally before any network request is sent.
    pub async fn sync_commands(&self, commands: &[CommandDefinition]) -> Result<Value> {
        validate_command_list(commands)?;
        let body = serde_json::json!({ "commands": commands });
        self.request_json(Method::PUT, "commands", Some(body)).await
    }

    async fn request_json(
        &self,
        method: Method,
        route: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let url = self
            .config
            .api_base_url()
            .join(route)
            .map_err(|_| Error::InvalidApiRoute(route.to_owned()))?;

        self.request_json_url(method, url, body).await
    }

    async fn request_json_url(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self.client.request(method, url);
        if let Some(body) = body {
            request = request.json(&body);
        }

        let mut response = request.send().await.map_err(sanitize_transport_error)?;

        let status = response.status();
        let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
        let response_body =
            read_limited_body(&mut response, self.config.max_response_bytes()).await?;

        tracing::debug!(
            http.status = status.as_u16(),
            response.bytes = response_body.len(),
            "received Yapper API response"
        );

        if !status.is_success() {
            return Err(ApiError::from_response(status, retry_after, &response_body).into());
        }

        if response_body.is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_slice(&response_body).map_err(Error::from)
    }
}

async fn read_limited_body(response: &mut Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(Error::ResponseTooLarge { limit });
    }

    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(limit);
    let mut body = Vec::with_capacity(initial_capacity);

    // Response::chunk() works without Reqwest's optional streaming feature,
    // so we can enforce a hard body limit without stream helper dependencies.
    while let Some(chunk) = response.chunk().await.map_err(sanitize_transport_error)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(Error::ResponseTooLarge { limit });
        }

        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Receiver};
    use std::thread::{self, JoinHandle};

    use serde_json::json;

    use super::*;
    use crate::config::Config;

    const BOT_ID: &str = "123e4567-e89b-12d3-a456-426614174000";

    struct LocalMockServer {
        uri: String,
        request: Receiver<String>,
        thread: Option<JoinHandle<()>>,
    }

    impl LocalMockServer {
        fn start(status: u16, headers: &[(&str, &str)], body: String) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let uri = format!("http://{address}");
            let (sender, request) = mpsc::channel();
            let headers = headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect::<Vec<_>>();

            let thread = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();

                let mut request_bytes = Vec::new();
                let mut buffer = [0_u8; 1024];

                let mut expected_request_length = None;
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }

                    request_bytes.extend_from_slice(&buffer[..read]);

                    if expected_request_length.is_none() {
                        if let Some(header_start) = request_bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                        {
                            let body_start = header_start + 4;
                            let header_text =
                                String::from_utf8_lossy(&request_bytes[..header_start]);
                            let content_length = header_text
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            expected_request_length = Some(body_start + content_length);
                        }
                    }

                    if expected_request_length
                        .is_some_and(|expected| request_bytes.len() >= expected)
                    {
                        break;
                    }
                }

                let request_text = String::from_utf8_lossy(&request_bytes).into_owned();
                let _ = sender.send(request_text);

                let reason = match status {
                    200 => "OK",
                    302 => "Found",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    409 => "Conflict",
                    429 => "Too Many Requests",
                    503 => "Service Unavailable",
                    _ => "Mock Response",
                };

                let mut response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n",
                    body.len()
                );

                for (name, value) in headers {
                    response.push_str(&format!("{name}: {value}\r\n"));
                }

                response.push_str("\r\n");
                response.push_str(&body);
                stream.write_all(response.as_bytes()).unwrap();
            });

            Self {
                uri,
                request,
                thread: Some(thread),
            }
        }

        fn uri(&self) -> &str {
            &self.uri
        }

        fn finish(mut self) -> String {
            self.thread.take().unwrap().join().unwrap();
            self.request.recv().unwrap()
        }
    }

    async fn test_client(server: &LocalMockServer, response_limit: usize) -> ApiClient {
        let base_url = format!("{}/api/bot/v1", server.uri());
        let config = Config::builder()
            .token("yb_test-token")
            .api_base_url(base_url)
            .allow_insecure_http_for_tests(true)
            .max_response_bytes(response_limit)
            .build()
            .unwrap();

        ApiClient::new(config).unwrap()
    }

    #[tokio::test]
    async fn get_me_uses_documented_route_and_bot_authorization() {
        let server = LocalMockServer::start(
            200,
            &[],
            json!({
                "id": BOT_ID,
                "username": "yapcore-test",
                "display_name": "Yapcore Test Bot",
                "bio": "Testing the Rust client"
            })
            .to_string(),
        );

        let client = test_client(&server, 1024).await;
        let profile = client.get_me().await.unwrap();
        let request = server.finish();

        assert!(request.starts_with("GET /api/bot/v1/me HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bot yb_test-token")
        );
        assert_eq!(profile.id.unwrap().as_str(), BOT_ID);
        assert_eq!(profile.username.as_deref(), Some("yapcore-test"));
        assert_eq!(profile.display_name.as_deref(), Some("Yapcore Test Bot"));
    }

    #[tokio::test]
    async fn rejects_response_bodies_over_the_configured_limit() {
        let server = LocalMockServer::start(200, &[], "x".repeat(128));
        let client = test_client(&server, 32).await;
        let error = client.get_me().await.unwrap_err();
        let _request = server.finish();

        assert!(matches!(error, Error::ResponseTooLarge { limit: 32 }));
    }

    #[tokio::test]
    async fn preserves_rate_limit_metadata() {
        let server = LocalMockServer::start(
            429,
            &[("Retry-After", "7")],
            json!({
                "error": "slow down",
                "code": "rate_limited"
            })
            .to_string(),
        );

        let client = test_client(&server, 1024).await;
        let error = client.get_me().await.unwrap_err();
        let _request = server.finish();

        match error {
            Error::Api(api_error) => {
                assert_eq!(api_error.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
                assert_eq!(api_error.message(), "slow down");
                assert_eq!(api_error.code(), Some("rate_limited"));
                assert_eq!(
                    api_error.retry_after(),
                    Some(std::time::Duration::from_secs(7))
                );
            }
            other => panic!("expected an API error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn responds_to_interaction_with_encoded_token_and_callback_body() {
        let server = LocalMockServer::start(200, &[], r#"{"ok":true}"#.to_owned());
        let client = test_client(&server, 1024).await;
        let interaction_id = InteractionId::new("123e4567-e89b-12d3-a456-426614174010").unwrap();
        let interaction_token = secrecy::SecretString::from("token-with/slash");

        let response = client
            .respond_interaction(
                &interaction_id,
                &interaction_token,
                InteractionResponseType::Ephemeral,
                Some("Only you can see this"),
            )
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "POST /api/bot/v1/interactions/123e4567-e89b-12d3-a456-426614174010/token-with%2Fslash/callback HTTP/1.1"
        ));
        let request_body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(request_body).unwrap();
        assert_eq!(payload["type"], "ephemeral");
        assert_eq!(payload["content"], "Only you can see this");
        assert!(response["ok"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn send_message_with_components_serializes_button_rows() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "id": MESSAGE_ID,
                "channel_id": CHANNEL_ID,
                "content": "Vote below"
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let row = crate::models::ActionRow::new([
            crate::models::Button::success("Yes", "vote:yes").unwrap(),
            crate::models::Button::danger("No", "vote:no").unwrap(),
            crate::models::Button::link("Rules", "https://example.com/rules").unwrap(),
        ])
        .unwrap();

        let _message = client
            .send_message_with_components(&channel_id, Some("Vote below"), &[row])
            .await
            .unwrap();
        let request = server.finish();

        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["components"][0]["type"], "row");
        assert_eq!(
            payload["components"][0]["components"][0]["style"],
            "success"
        );
        assert_eq!(
            payload["components"][0]["components"][1]["custom_id"],
            "vote:no"
        );
        assert_eq!(
            payload["components"][0]["components"][2]["url"],
            "https://example.com/rules"
        );
    }

    #[tokio::test]
    async fn send_rich_message_rejects_duplicate_custom_ids_across_rows() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();
        let rows = [
            crate::models::ActionRow::new([
                crate::models::Button::primary("First", "duplicate").unwrap()
            ])
            .unwrap(),
            crate::models::ActionRow::new([crate::models::Button::secondary(
                "Second",
                "duplicate",
            )
            .unwrap()])
            .unwrap(),
        ];

        assert!(matches!(
            client
                .send_message_with_components(&channel_id, Some("Buttons"), &rows)
                .await,
            Err(Error::ComponentValidation(
                crate::models::ComponentValidationError::DuplicateCustomId
            ))
        ));
    }

    #[tokio::test]
    async fn send_message_with_embeds_serializes_builder_output() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "id": MESSAGE_ID,
                "channel_id": CHANNEL_ID,
                "content": "Look at this",
                "embeds": [{ "title": "Yapcore" }]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let embed = Embed::builder()
            .title("Yapcore")
            .description("Native Rust framework")
            .color(0x0058_65FF)
            .field(crate::models::EmbedField::new("Language", "Rust").unwrap())
            .build()
            .unwrap();

        let message = client
            .send_message_with_embeds(&channel_id, Some("Look at this"), &[embed])
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "POST /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["content"], "Look at this");
        assert_eq!(payload["embeds"][0]["title"], "Yapcore");
        assert_eq!(payload["embeds"][0]["fields"][0]["name"], "Language");
        assert_eq!(message.content.as_deref(), Some("Look at this"));
    }

    #[tokio::test]
    async fn send_message_with_embeds_rejects_missing_content_and_embeds() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();

        assert!(matches!(
            client
                .send_message_with_embeds(&channel_id, None, &[])
                .await,
            Err(Error::EmptyMessagePayload)
        ));
    }

    #[tokio::test]
    async fn send_message_with_embeds_enforces_message_wide_text_limit() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();
        let embed = Embed::builder()
            .description("x".repeat(4_000))
            .build()
            .unwrap();
        let embeds = [embed.clone(), embed];

        assert!(matches!(
            client
                .send_message_with_embeds(&channel_id, None, &embeds)
                .await,
            Err(Error::MessageEmbedsTooLong { actual: 8_000 })
        ));
    }

    #[tokio::test]
    async fn send_message_with_embeds_rejects_more_than_five_embeds() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();
        let embed = Embed::builder().title("Yapcore").build().unwrap();
        let embeds = vec![embed; 6];

        assert!(matches!(
            client
                .send_message_with_embeds(&channel_id, Some("text"), &embeds)
                .await,
            Err(Error::TooManyEmbeds { actual: 6 })
        ));
    }

    #[tokio::test]
    async fn send_message_posts_content_and_deserializes_the_returned_message() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "message": {
                    "id": MESSAGE_ID,
                    "channel_id": CHANNEL_ID,
                    "content": "pong"
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();

        let message = client.send_message(&channel_id, "pong").await.unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "POST /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages HTTP/1.1"
        ));
        let request_body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(request_body).unwrap();

        assert_eq!(payload["content"], "pong");
        assert_eq!(message.id.as_str(), MESSAGE_ID);
        assert_eq!(message.channel_id.as_str(), CHANNEL_ID);
        assert_eq!(message.content.as_deref(), Some("pong"));
    }

    #[tokio::test]
    async fn send_message_rejects_empty_or_oversized_content_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();

        assert!(matches!(
            client.send_message(&channel_id, "").await,
            Err(Error::EmptyMessageContent)
        ));
        assert!(matches!(
            client.send_message(&channel_id, &"x".repeat(2_001)).await,
            Err(Error::MessageContentTooLong {
                actual: 2_001,
                max: 2_000
            })
        ));
    }

    #[tokio::test]
    async fn edits_message_content_with_patch() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "id": MESSAGE_ID,
                "channel_id": CHANNEL_ID,
                "content": "edited"
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        let message = client
            .edit_message(&channel_id, &message_id, "edited")
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "PATCH /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001 HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["content"], "edited");
        assert_eq!(message.content.as_deref(), Some("edited"));
    }

    #[tokio::test]
    async fn lists_reactions_for_a_message() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "reactions": [{
                    "emoji": "🔥",
                    "count": 2
                }]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        let reactions = client
            .get_reactions(&channel_id, &message_id)
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "GET /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001/reactions HTTP/1.1"
        ));
        assert_eq!(reactions["reactions"][0]["count"], 2);
    }

    #[tokio::test]
    async fn bulk_delete_sends_documented_ids_and_preserves_result() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ONE: &str = "123e4567-e89b-12d3-a456-426614174001";
        const MESSAGE_TWO: &str = "123e4567-e89b-12d3-a456-426614174003";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "deleted": [MESSAGE_ONE],
                "skipped": [MESSAGE_TWO]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_ids = [
            crate::ids::MessageId::new(MESSAGE_ONE).unwrap(),
            crate::ids::MessageId::new(MESSAGE_TWO).unwrap(),
        ];

        let result = client
            .bulk_delete_messages(&channel_id, &message_ids)
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "POST /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/bulk-delete HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["ids"].as_array().unwrap().len(), 2);
        assert_eq!(result["deleted"][0], MESSAGE_ONE);
        assert_eq!(result["skipped"][0], MESSAGE_TWO);
    }

    #[tokio::test]
    async fn bulk_delete_rejects_invalid_id_counts_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();
        let one_id = crate::ids::MessageId::new("123e4567-e89b-12d3-a456-426614174001").unwrap();
        let too_many = vec![one_id; 101];

        assert!(matches!(
            client.bulk_delete_messages(&channel_id, &[]).await,
            Err(Error::InvalidBulkDeleteCount { actual: 0 })
        ));
        assert!(matches!(
            client.bulk_delete_messages(&channel_id, &too_many).await,
            Err(Error::InvalidBulkDeleteCount { actual: 101 })
        ));
    }

    #[tokio::test]
    async fn pins_a_message_with_put() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        client.pin_message(&channel_id, &message_id).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "PUT /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001/pin HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn unpins_a_message_with_delete() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        client
            .unpin_message(&channel_id, &message_id)
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "DELETE /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001/pin HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn adds_reactions_using_one_encoded_emoji_path_segment() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        client
            .add_reaction(&channel_id, &message_id, "🔥/up")
            .await
            .unwrap();
        let request = server.finish();

        let request_line = request.lines().next().unwrap();
        assert!(request_line.starts_with("PUT /api/bot/v1/channels/"));
        assert!(request_line.contains("/reactions/"));
        assert!(request_line.contains("%2Fup"));
        assert!(!request_line.contains("🔥/up"));
    }

    #[tokio::test]
    async fn rejects_empty_reaction_identifiers_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();
        let message_id =
            crate::ids::MessageId::new("123e4567-e89b-12d3-a456-426614174001").unwrap();

        assert!(matches!(
            client.add_reaction(&channel_id, &message_id, "").await,
            Err(Error::InvalidEmoji)
        ));
    }

    #[tokio::test]
    async fn fetches_recent_messages_with_limit_and_before_cursor() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "messages": [{
                    "id": MESSAGE_ID,
                    "channel_id": CHANNEL_ID,
                    "content": "recent"
                }]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();

        let messages = client
            .get_recent_messages(&channel_id, Some(50), Some("2026-11-21T12:00:00Z"))
            .await
            .unwrap();
        let request = server.finish();
        let request_line = request.lines().next().unwrap();

        assert!(request_line.starts_with("GET /api/bot/v1/channels/"));
        assert!(request_line.contains("/messages?limit=50&before=2026-11-21T12%3A00%3A00Z"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_deref(), Some("recent"));
    }

    #[tokio::test]
    async fn fetches_messages_after_timestamp_cursor() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!([{
                "id": MESSAGE_ID,
                "channel_id": CHANNEL_ID,
                "content": "forward"
            }])
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();

        let messages = client
            .get_messages_after(&channel_id, "2026-11-21T12:00:00Z")
            .await
            .unwrap();
        let request = server.finish();
        let request_line = request.lines().next().unwrap();

        assert!(request_line.contains("/messages?after=2026-11-21T12%3A00%3A00Z"));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content.as_deref(), Some("forward"));
    }

    #[tokio::test]
    async fn validates_message_page_sizes_and_forward_cursors() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();

        assert!(matches!(
            client.get_recent_messages(&channel_id, Some(0), None).await,
            Err(Error::InvalidMessagePageLimit { actual: 0 })
        ));
        assert!(matches!(
            client
                .get_recent_messages(&channel_id, Some(101), None)
                .await,
            Err(Error::InvalidMessagePageLimit { actual: 101 })
        ));
        assert!(matches!(
            client.get_messages_after(&channel_id, "").await,
            Err(Error::EmptyMessageCursor)
        ));
    }

    #[tokio::test]
    async fn fetches_server_details_from_wrapped_response() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "server": {
                    "id": SERVER_ID,
                    "name": "Example server",
                    "member_count": 42
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();

        let details = client.get_server(&server_id).await.unwrap();
        let request = server.finish();

        assert!(
            request.starts_with(
                "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020 HTTP/1.1"
            )
        );
        assert_eq!(details.name.as_deref(), Some("Example server"));
    }

    #[tokio::test]
    async fn fetches_one_channel_from_wrapped_response() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174021";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "channel": {
                    "id": CHANNEL_ID,
                    "name": "general",
                    "type": "text"
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();

        let channel = client.get_channel(&channel_id).await.unwrap();
        let request = server.finish();

        assert!(
            request.starts_with(
                "GET /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174021 HTTP/1.1"
            )
        );
        assert_eq!(channel.name.as_deref(), Some("general"));
    }

    #[tokio::test]
    async fn fetches_one_member_with_roles_and_nickname() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";
        const ROLE_ID: &str = "123e4567-e89b-12d3-a456-426614174023";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "member": {
                    "user_id": USER_ID,
                    "username": "member",
                    "nickname": "Member",
                    "role_ids": [ROLE_ID]
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();

        let member = client.get_member(&server_id, &user_id).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/members/123e4567-e89b-12d3-a456-426614174022 HTTP/1.1"
        ));
        assert_eq!(member.nickname.as_deref(), Some("Member"));
        assert_eq!(member.role_ids.len(), 1);
    }

    #[tokio::test]
    async fn lists_server_roles() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const ROLE_ID: &str = "123e4567-e89b-12d3-a456-426614174023";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "roles": [{
                    "id": ROLE_ID,
                    "name": "Moderator",
                    "position": 3
                }]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();

        let roles = client.list_roles(&server_id).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/roles HTTP/1.1"
        ));
        assert_eq!(roles[0].name.as_deref(), Some("Moderator"));
        assert_eq!(roles[0].position, Some(3));
    }

    #[tokio::test]
    async fn fetches_public_user_profile() {
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "user": {
                    "id": USER_ID,
                    "username": "example-user",
                    "display_name": "Example"
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();

        let user = client.get_user(&user_id).await.unwrap();
        let request = server.finish();

        assert!(
            request
                .starts_with("GET /api/bot/v1/users/123e4567-e89b-12d3-a456-426614174022 HTTP/1.1")
        );
        assert_eq!(user.username.as_deref(), Some("example-user"));
        assert_eq!(user.display_name.as_deref(), Some("Example"));
    }

    #[tokio::test]
    async fn adds_and_removes_member_roles_with_documented_routes() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";
        const ROLE_ID: &str = "123e4567-e89b-12d3-a456-426614174023";

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();
        let role_id = crate::ids::RoleId::new(ROLE_ID).unwrap();

        client
            .add_role(&server_id, &user_id, &role_id)
            .await
            .unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "PUT /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/members/123e4567-e89b-12d3-a456-426614174022/roles/123e4567-e89b-12d3-a456-426614174023 HTTP/1.1"
        ));

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        client
            .remove_role(&server_id, &user_id, &role_id)
            .await
            .unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "DELETE /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/members/123e4567-e89b-12d3-a456-426614174022/roles/123e4567-e89b-12d3-a456-426614174023 HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn sets_member_nickname_and_validates_its_length() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";

        let server =
            LocalMockServer::start(200, &[], serde_json::json!({ "ok": true }).to_string());
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();

        client
            .set_nickname(&server_id, &user_id, "Member")
            .await
            .unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "PATCH /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/members/123e4567-e89b-12d3-a456-426614174022 HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["nickname"], "Member");

        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        assert!(matches!(
            client
                .set_nickname(&server_id, &user_id, &"x".repeat(33))
                .await,
            Err(Error::NicknameTooLong { actual: 33 })
        ));
    }

    #[tokio::test]
    async fn kicks_and_bans_members_with_user_id_payloads() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";

        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();

        let server = LocalMockServer::start(200, &[], r#"{"ok":true}"#.to_owned());
        let client = test_client(&server, 1024).await;
        client.kick_member(&server_id, &user_id).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/kick HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["user_id"], USER_ID);

        let server = LocalMockServer::start(200, &[], r#"{"ok":true}"#.to_owned());
        let client = test_client(&server, 1024).await;
        client.ban_member(&server_id, &user_id).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/bans HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["user_id"], USER_ID);
    }

    #[tokio::test]
    async fn lists_bans_and_unbans_member() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "bans": [{
                    "user_id": USER_ID,
                    "banned_by": "123e4567-e89b-12d3-a456-426614174003",
                    "created_at": "2026-11-22T00:00:00Z",
                    "user": { "id": USER_ID, "username": "banned-user" }
                }]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();

        let bans = client.list_bans(&server_id).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/bans HTTP/1.1"
        ));
        assert_eq!(bans.len(), 1);
        assert_eq!(
            bans[0]
                .user
                .as_ref()
                .and_then(|user| user.username.as_deref()),
            Some("banned-user")
        );

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();
        client.unban_member(&server_id, &user_id).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "DELETE /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/bans/123e4567-e89b-12d3-a456-426614174022 HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn sets_and_clears_timeouts_with_reason_and_limits() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";

        let server = LocalMockServer::start(200, &[], r#"{"ok":true}"#.to_owned());
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();

        client
            .timeout_member(&server_id, &user_id, Some(30), Some("Rule violation"))
            .await
            .unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/timeouts HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["user_id"], USER_ID);
        assert_eq!(payload["minutes"], 30);
        assert_eq!(payload["reason"], "Rule violation");

        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        assert!(matches!(
            client
                .timeout_member(&server_id, &user_id, Some(40_321), None)
                .await,
            Err(Error::TimeoutTooLong { actual: 40_321 })
        ));
        assert!(matches!(
            client
                .timeout_member(&server_id, &user_id, Some(1), Some(&"x".repeat(201)))
                .await,
            Err(Error::ModerationReasonTooLong { actual: 201 })
        ));
    }

    #[tokio::test]
    async fn lists_servers_from_the_documented_response_wrapper() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "servers": [{
                    "id": SERVER_ID,
                    "name": "Example",
                    "member_count": 12
                }]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;

        let servers = client.list_servers().await.unwrap();
        let request = server.finish();

        assert!(request.starts_with("GET /api/bot/v1/servers HTTP/1.1"));
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name.as_deref(), Some("Example"));
        assert_eq!(servers[0].member_count, Some(12));
    }

    #[tokio::test]
    async fn lists_server_channels_with_type_and_categories() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174021";
        const CATEGORY_ID: &str = "123e4567-e89b-12d3-a456-426614174024";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "channels": [{
                    "id": CHANNEL_ID,
                    "name": "general",
                    "type": "text",
                    "server_id": SERVER_ID
                }],
                "categories": [{
                    "id": CATEGORY_ID,
                    "name": "Community"
                }]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();

        let listing = client
            .list_channels(Some(&server_id), Some(ChannelType::Text))
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/channels?type=text HTTP/1.1"
        ));
        assert_eq!(listing.channels.len(), 1);
        assert_eq!(listing.channels[0].channel_type.as_deref(), Some("text"));
        assert_eq!(listing.categories[0].name.as_deref(), Some("Community"));
    }

    #[tokio::test]
    async fn lists_members_with_limit_and_after_user_cursor() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "members": [{
                    "id": USER_ID,
                    "username": "new-member",
                    "role_ids": []
                }]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let after = crate::ids::UserId::new(USER_ID).unwrap();

        let members = client
            .list_members(&server_id, Some(500), Some(&after))
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/members?limit=500&after=123e4567-e89b-12d3-a456-426614174022 HTTP/1.1"
        ));
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].username.as_deref(), Some("new-member"));
    }

    #[tokio::test]
    async fn rejects_invalid_member_page_sizes_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let server_id = crate::ids::ServerId::new("123e4567-e89b-12d3-a456-426614174020").unwrap();

        assert!(matches!(
            client.list_members(&server_id, Some(0), None).await,
            Err(Error::InvalidMemberPageLimit { actual: 0 })
        ));
        assert!(matches!(
            client.list_members(&server_id, Some(501), None).await,
            Err(Error::InvalidMemberPageLimit { actual: 501 })
        ));
    }

    #[tokio::test]
    async fn fetches_one_message_with_get() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "id": MESSAGE_ID,
                "channel_id": CHANNEL_ID,
                "content": "fetched"
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        let message = client.get_message(&channel_id, &message_id).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "GET /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001 HTTP/1.1"
        ));
        assert_eq!(message.content.as_deref(), Some("fetched"));
    }

    #[tokio::test]
    async fn deletes_one_message_with_empty_success_response() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();

        client
            .delete_message(&channel_id, &message_id)
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "DELETE /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001 HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn poll_events_uses_cursor_wait_and_max_age_query_parameters() {
        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "events": [{
                    "id": 12,
                    "type": "new_event_type",
                    "data": { "channel_id": "channel-1" }
                }],
                "last": 12
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;

        let batch = client.poll_events(10, 25, Some(3_600)).await.unwrap();
        let request = server.finish();

        assert!(
            request.starts_with("GET /api/bot/v1/events?after=10&wait=25&max_age=3600 HTTP/1.1")
        );
        assert_eq!(batch.last(), 12);
        assert_eq!(batch.events().len(), 1);
        assert_eq!(batch.events()[0].event_type, "new_event_type");
    }

    #[tokio::test]
    async fn poll_events_rejects_wait_and_max_age_above_platform_limits() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();

        assert!(matches!(
            client.poll_events(0, 26, None).await,
            Err(Error::InvalidEventPollWait)
        ));
        assert!(matches!(
            client.poll_events(0, 0, Some(86_401)).await,
            Err(Error::InvalidEventMaxAge)
        ));
    }

    #[tokio::test]
    async fn sync_commands_replaces_the_full_registry_with_the_documented_payload() {
        let server = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&server, 1024).await;
        let commands = vec![
            CommandDefinition::new("ping", "Check whether the bot is alive").unwrap(),
            CommandDefinition::new("about", "Show information about the bot").unwrap(),
        ];

        let response = client.sync_commands(&commands).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with("PUT /api/bot/v1/commands HTTP/1.1"));
        let request_body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(request_body).unwrap();

        assert_eq!(payload["commands"][0]["name"], "ping");
        assert_eq!(
            payload["commands"][1]["description"],
            "Show information about the bot"
        );
        assert!(payload.get("interactions").is_none());
        assert!(response["ok"].as_bool().unwrap());
    }

    #[tokio::test]
    async fn does_not_follow_redirects_for_authenticated_requests() {
        let server = LocalMockServer::start(
            302,
            &[("Location", "https://example.com/redirect-target")],
            String::new(),
        );

        let client = test_client(&server, 1024).await;
        let error = client.get_me().await.unwrap_err();
        let request = server.finish();

        assert!(request.starts_with("GET /api/bot/v1/me HTTP/1.1"));
        assert!(matches!(error, Error::Api(api) if api.status() == reqwest::StatusCode::FOUND));
    }
}
