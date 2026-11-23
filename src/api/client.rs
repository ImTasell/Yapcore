use std::sync::Arc;

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER};
use reqwest::redirect::Policy;
use reqwest::{Client, Method, Response, Url};
use secrecy::ExposeSecret;
use serde_json::Value;

use crate::config::Config;
use crate::error::{ApiError, Error, Result, parse_retry_after, sanitize_transport_error};
use crate::ids::InteractionId;
use crate::models::{
    Ban, BotProfile, BotProfileEdit, BotStatus, Channel, ChannelCategory, ChannelCreateOptions,
    ChannelEditOptions, ChannelListing, ChannelType, CommandDefinition, Embed, EventBatch,
    ImageAsset, ImageFormat, InteractionResponseType, Member, Message, PermissionTarget, Presence,
    Role, Server, UserProfile, VoiceChannelState, validate_action_rows, validate_command_list,
    validate_layout, validate_component_rows, LayoutComponent, MessageComponentRow,
    COMPONENTS_V2_FLAG,
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

    /// Edit the bot's display name and/or bio through PATCH /me.
    pub async fn edit_me(&self, changes: &BotProfileEdit) -> Result<BotProfile> {
        changes.validate()?;
        let value = serde_json::to_value(changes)?;
        let response = self.request_json(Method::PATCH, "me", Some(value)).await?;
        serde_json::from_value(response).map_err(Error::from)
    }

    /// Upload a new bot avatar using the documented raw-image request body.
    pub async fn set_avatar(&self, bytes: &[u8], format: ImageFormat) -> Result<ImageAsset> {
        self.upload_profile_image("me/avatar", bytes, format, 8 * 1024 * 1024, false)
            .await
    }

    /// Upload a bot avatar using the documented multipart file field.
    pub async fn set_avatar_multipart(
        &self,
        bytes: &[u8],
        format: ImageFormat,
    ) -> Result<ImageAsset> {
        self.upload_profile_image("me/avatar", bytes, format, 8 * 1024 * 1024, true)
            .await
    }

    /// Remove the bot's avatar.
    pub async fn delete_avatar(&self) -> Result<()> {
        self.request_json(Method::DELETE, "me/avatar", None).await?;
        Ok(())
    }

    /// Upload a new bot banner using the documented raw-image request body.
    pub async fn set_banner(&self, bytes: &[u8], format: ImageFormat) -> Result<ImageAsset> {
        self.upload_profile_image("me/banner", bytes, format, 10 * 1024 * 1024, false)
            .await
    }

    /// Upload a bot banner using the documented multipart file field.
    pub async fn set_banner_multipart(
        &self,
        bytes: &[u8],
        format: ImageFormat,
    ) -> Result<ImageAsset> {
        self.upload_profile_image("me/banner", bytes, format, 10 * 1024 * 1024, true)
            .await
    }

    /// Remove the bot's banner.
    pub async fn delete_banner(&self) -> Result<()> {
        self.request_json(Method::DELETE, "me/banner", None).await?;
        Ok(())
    }

    /// Set the bot's online state and optional activity line.
    ///
    /// Yapper keeps this presence while the bot polls. This is the documented
    /// HTTP presence API, not a Discord-compatible local RPC/IPC connection.
    /// Perform an authenticated request to a relative bot API route.
    ///
    /// Absolute URLs, origin-relative paths and path traversal are rejected to
    /// prevent accidentally sending the bot token to another host.
    pub async fn api(&self, method: Method, route: &str, body: Option<Value>) -> Result<Value> {
        if route.is_empty()
            || route.starts_with('/')
            || route.contains("://")
            || route.split('/').any(|part| part == "." || part == "..")
        {
            return Err(Error::InvalidApiRoute(route.to_owned()));
        }

        let base = self.config.api_base_url();
        let url = base
            .join(route)
            .map_err(|_| Error::InvalidApiRoute(route.to_owned()))?;
        let base_path = base.path().trim_end_matches('/');
        let allowed_path = format!("{base_path}/");
        if url.origin() != base.origin() || !url.path().starts_with(&allowed_path) {
            return Err(Error::InvalidApiRoute(route.to_owned()));
        }

        self.request_json_url(method, url, body).await
    }

    pub async fn set_status(&self, status: BotStatus, activity: Option<&str>) -> Result<Value> {
        if let Some(activity) = activity {
            let actual = activity.chars().count();
            if actual > 60 {
                return Err(Error::BotActivityTooLong { actual });
            }
        }
        let mut body = serde_json::Map::new();
        body.insert("status".to_owned(), serde_json::to_value(status)?);
        if let Some(activity) = activity {
            body.insert("activity".to_owned(), serde_json::json!(activity));
        }
        self.request_json(Method::PUT, "me/status", Some(Value::Object(body)))
            .await
    }

    /// Set the bot's status using the typed presence builder.
    ///
    /// Calling this while the bot is running updates visible presence without
    /// restarting or interrupting the event poller.
    pub async fn set_presence(&self, presence: &Presence) -> Result<Value> {
        let mut body = serde_json::Map::new();
        body.insert(
            "status".to_owned(),
            serde_json::to_value(presence.status())?,
        );
        if let Some(activity) = presence.activity_line() {
            let actual = activity.as_str().chars().count();
            if actual > 60 {
                return Err(Error::BotActivityTooLong { actual });
            }
            body.insert("activity".to_owned(), serde_json::json!(activity.as_str()));
        }
        self.request_json(Method::PUT, "me/status", Some(Value::Object(body)))
            .await
    }

    /// Return voice occupancy for every voice channel the bot can see in a server.
    pub async fn get_voice_states(
        &self,
        server_id: &crate::ids::ServerId,
    ) -> Result<Vec<VoiceChannelState>> {
        let route = format!("servers/{server_id}/voice");
        let response = self.request_json(Method::GET, &route, None).await?;
        let voice = response.get("voice").cloned().unwrap_or(response);
        serde_json::from_value(voice).map_err(Error::from)
    }

    /// Move a member who is in voice to another visible voice channel.
    pub async fn move_member_to_voice(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
        channel_id: &crate::ids::ChannelId,
    ) -> Result<Value> {
        let route = format!("servers/{server_id}/voice/move");
        let body = serde_json::json!({
            "user_id": user_id,
            "channel_id": channel_id,
        });
        self.request_json(Method::POST, &route, Some(body)).await
    }

    /// Disconnect a member from their current voice channel.
    pub async fn disconnect_member_from_voice(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
    ) -> Result<Value> {
        let route = format!("servers/{server_id}/voice/disconnect");
        let body = serde_json::json!({ "user_id": user_id });
        self.request_json(Method::POST, &route, Some(body)).await
    }

    /// Fetch a member's effective server permissions (experimental source-parity route).
    pub async fn get_member_permissions(
        &self,
        server_id: &crate::ids::ServerId,
        user_id: &crate::ids::UserId,
    ) -> Result<Value> {
        let route = format!("servers/{server_id}/members/{user_id}/permissions");
        let response = self.request_json(Method::GET, &route, None).await?;
        Ok(response.get("permissions").cloned().unwrap_or(response))
    }

    /// Fetch a member's effective channel permissions (experimental source-parity route).
    pub async fn get_channel_member_permissions(
        &self,
        channel_id: &crate::ids::ChannelId,
        user_id: &crate::ids::UserId,
    ) -> Result<Value> {
        let route = format!("channels/{channel_id}/permissions/{user_id}");
        let response = self.request_json(Method::GET, &route, None).await?;
        Ok(response.get("permissions").cloned().unwrap_or(response))
    }

    /// List channel permission overrides (experimental source-parity route).
    pub async fn list_channel_permission_overrides(
        &self,
        channel_id: &crate::ids::ChannelId,
    ) -> Result<Value> {
        let route = format!("channels/{channel_id}/permissions");
        let response = self.request_json(Method::GET, &route, None).await?;
        Ok(response.get("overrides").cloned().unwrap_or(response))
    }

    /// Set a channel permission override (experimental source-parity route).
    pub async fn set_channel_permissions(
        &self,
        channel_id: &crate::ids::ChannelId,
        target: &PermissionTarget,
        allow: &[String],
        deny: &[String],
    ) -> Result<Value> {
        let route = format!(
            "channels/{channel_id}/permissions/{}",
            target.as_path_segment()
        );
        let body = serde_json::json!({ "allow": allow, "deny": deny });
        self.request_json(Method::PUT, &route, Some(body)).await
    }

    /// Clear a channel permission override (experimental source-parity route).
    pub async fn clear_channel_permissions(
        &self,
        channel_id: &crate::ids::ChannelId,
        target: &PermissionTarget,
    ) -> Result<()> {
        let route = format!(
            "channels/{channel_id}/permissions/{}",
            target.as_path_segment()
        );
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
    }

    /// Set server-wide @everyone permissions (experimental source-parity route).
    pub async fn set_everyone_permissions(
        &self,
        server_id: &crate::ids::ServerId,
        permissions: &Value,
    ) -> Result<Value> {
        let route = format!("servers/{server_id}/roles/everyone");
        self.request_json(
            Method::PATCH,
            &route,
            Some(serde_json::json!({ "permissions": permissions })),
        )
        .await
    }

    /// Set a category permission override (experimental source-parity route).
    pub async fn set_category_permissions(
        &self,
        server_id: &crate::ids::ServerId,
        category_id: &crate::ids::CategoryId,
        target: &PermissionTarget,
        allow: &[String],
        deny: &[String],
    ) -> Result<Value> {
        let route = format!(
            "servers/{server_id}/categories/{category_id}/permissions/{}",
            target.as_path_segment()
        );
        let body = serde_json::json!({ "allow": allow, "deny": deny });
        self.request_json(Method::PUT, &route, Some(body)).await
    }

    /// Clear a category permission override (experimental source-parity route).
    pub async fn clear_category_permissions(
        &self,
        server_id: &crate::ids::ServerId,
        category_id: &crate::ids::CategoryId,
        target: &PermissionTarget,
    ) -> Result<()> {
        let route = format!(
            "servers/{server_id}/categories/{category_id}/permissions/{}",
            target.as_path_segment()
        );
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
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

    /// Send a message with button and/or select-menu action rows.
    ///
    /// Each row contains either one to five buttons or exactly one select menu.
    /// This source-parity extension is not fully described by the public endpoint guide.
    pub async fn send_message_with_component_rows(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: Option<&str>,
        components: &[MessageComponentRow],
    ) -> Result<Message> {
        self.send_rich_message_with_component_rows(channel_id, content, &[], components)
            .await
    }

    /// Send a rich message with mixed button/select rows.
    pub async fn send_rich_message_with_component_rows(
        &self,
        channel_id: &crate::ids::ChannelId,
        content: Option<&str>,
        embeds: &[Embed],
        components: &[MessageComponentRow],
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

        validate_component_rows(components)?;

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

    /// Send an experimental Components V2 layout as the complete message body.
    ///
    /// Components V2 messages use the dedicated flag and do not combine with
    /// ordinary message content or embeds. This path mirrors the upstream
    /// Python SDK; the public endpoint guide does not currently document the
    /// full layout schema.
    pub async fn send_layout(
        &self,
        channel_id: &crate::ids::ChannelId,
        components: &[LayoutComponent],
        reply_to: Option<&crate::ids::MessageId>,
    ) -> Result<Message> {
        validate_layout(components)?;
        let route = format!("channels/{channel_id}/messages");
        let mut body = serde_json::json!({
            "flags": COMPONENTS_V2_FLAG,
            "components": components,
        });
        if let Some(message_id) = reply_to {
            body["reply_to"] = serde_json::json!(message_id.as_str());
        }
        let response = self.request_json(Method::POST, &route, Some(body)).await?;
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

    /// Create a text or voice channel in a server.
    ///
    /// Channel creation supports initial privacy and role/user access settings
    /// so private channels can be created without a temporary public state.
    pub async fn create_channel(
        &self,
        server_id: &crate::ids::ServerId,
        options: &ChannelCreateOptions,
    ) -> Result<Channel> {
        options.validate()?;
        let route = format!("servers/{server_id}/channels");
        let response = self
            .request_json(Method::POST, &route, Some(serde_json::to_value(options)?))
            .await?;
        let channel = response.get("channel").cloned().unwrap_or(response);
        serde_json::from_value(channel).map_err(Error::from)
    }

    /// Edit selected fields of a channel.
    pub async fn edit_channel(
        &self,
        channel_id: &crate::ids::ChannelId,
        changes: &ChannelEditOptions,
    ) -> Result<Channel> {
        changes.validate()?;
        let route = format!("channels/{channel_id}");
        let response = self
            .request_json(Method::PATCH, &route, Some(serde_json::to_value(changes)?))
            .await?;
        let channel = response.get("channel").cloned().unwrap_or(response);
        serde_json::from_value(channel).map_err(Error::from)
    }

    /// Delete a channel.
    pub async fn delete_channel(&self, channel_id: &crate::ids::ChannelId) -> Result<()> {
        let route = format!("channels/{channel_id}");
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
    }

    /// Create a category in a server.
    pub async fn create_category(
        &self,
        server_id: &crate::ids::ServerId,
        name: &str,
    ) -> Result<ChannelCategory> {
        if name.trim().is_empty() {
            return Err(Error::EmptyCategoryName);
        }
        let route = format!("servers/{server_id}/categories");
        let response = self
            .request_json(
                Method::POST,
                &route,
                Some(serde_json::json!({ "name": name })),
            )
            .await?;
        let category = response.get("category").cloned().unwrap_or(response);
        serde_json::from_value(category).map_err(Error::from)
    }

    /// Rename a category in a server.
    pub async fn rename_category(
        &self,
        server_id: &crate::ids::ServerId,
        category_id: &crate::ids::CategoryId,
        name: &str,
    ) -> Result<ChannelCategory> {
        if name.trim().is_empty() {
            return Err(Error::EmptyCategoryName);
        }
        let route = format!("servers/{server_id}/categories/{category_id}");
        let response = self
            .request_json(
                Method::PATCH,
                &route,
                Some(serde_json::json!({ "name": name })),
            )
            .await?;
        let category = response.get("category").cloned().unwrap_or(response);
        serde_json::from_value(category).map_err(Error::from)
    }

    /// Delete a category in a server.
    pub async fn delete_category(
        &self,
        server_id: &crate::ids::ServerId,
        category_id: &crate::ids::CategoryId,
    ) -> Result<()> {
        let route = format!("servers/{server_id}/categories/{category_id}");
        self.request_json(Method::DELETE, &route, None).await?;
        Ok(())
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
        changes: &crate::models::RoleEditOptions,
    ) -> Result<Role> {
        if changes.is_empty() {
            return Err(Error::EmptyRoleEditOptions);
        }

        let body = serde_json::to_value(changes)?;
        let route = format!("servers/{server_id}/roles/{role_id}");
        let response = self.request_json(Method::PATCH, &route, Some(body)).await?;
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
        let response = self.request_json(Method::PATCH, &route, Some(body)).await?;
        let roles = response.get("roles").cloned().unwrap_or(response);
        serde_json::from_value(roles).map_err(Error::from)
    }

    /// Open or find a direct-message channel with a user.
    pub async fn open_dm(&self, user_id: &crate::ids::UserId) -> Result<crate::ids::ChannelId> {
        let route = format!("users/{user_id}/dm");
        let response = self.request_json(Method::POST, &route, None).await?;
        let channel_id = response
            .get("channel_id")
            .and_then(Value::as_str)
            .ok_or(Error::MissingDmChannelId)?;
        crate::ids::ChannelId::new(channel_id).map_err(Error::from)
    }

    /// Open a DM and send a plain-text message to it.
    pub async fn send_dm(
        &self,
        user_id: &crate::ids::UserId,
        content: impl AsRef<str>,
    ) -> Result<Message> {
        let channel_id = self.open_dm(user_id).await?;
        self.send_message(&channel_id, content.as_ref()).await
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

    /// Replace the layout of an experimental Components V2 message.
    pub async fn edit_layout(
        &self,
        channel_id: &crate::ids::ChannelId,
        message_id: &crate::ids::MessageId,
        components: &[LayoutComponent],
    ) -> Result<Message> {
        validate_layout(components)?;
        let route = format!("channels/{channel_id}/messages/{message_id}");
        let body = serde_json::json!({
            "flags": COMPONENTS_V2_FLAG,
            "components": components,
        });
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
        modal: Option<&Value>,
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
        if let Some(modal) = modal {
            body["modal"] = modal.clone();
        }

        self.request_json_url(Method::POST, url, Some(body)).await
    }

    /// Read the bot's currently registered slash commands.
    pub async fn list_commands(&self) -> Result<Vec<CommandDefinition>> {
        let response = self.request_json(Method::GET, "commands", None).await?;
        let commands = response.get("commands").cloned().unwrap_or(response);
        serde_json::from_value(commands).map_err(Error::from)
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
        let mut retries = 0_u8;
        loop {
            let mut request = self.client.request(method.clone(), url.clone());
            if let Some(body) = &body {
                request = request.json(body);
            }

            let mut response = request.send().await.map_err(sanitize_transport_error)?;
            let status = response.status();
            let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
            let response_body =
                read_limited_body(&mut response, self.config.max_response_bytes()).await?;

            tracing::debug!(
                http.status = status.as_u16(),
                response.bytes = response_body.len(),
                retry_attempt = retries,
                "received Yapper API response"
            );

            if !status.is_success() {
                let error = ApiError::from_response(status, retry_after, &response_body);
                let retryable_status = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    || status == reqwest::StatusCode::SERVICE_UNAVAILABLE;
                if retryable_status && error.retry_after().is_some() && retries < 2 {
                    let delay = error.retry_after().expect("checked above");
                    retries += 1;
                    tracing::warn!(
                        http.status = status.as_u16(),
                        retry_seconds = delay.as_secs_f64(),
                        retry_attempt = retries,
                        "Yapper supplied Retry-After; retrying request"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(error.into());
            }

            if response_body.is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_slice(&response_body).map_err(Error::from);
        }
    }

    async fn upload_profile_image(
        &self,
        route: &str,
        bytes: &[u8],
        format: ImageFormat,
        maximum_bytes: usize,
        multipart: bool,
    ) -> Result<ImageAsset> {
        if bytes.is_empty() {
            return Err(Error::EmptyImageUpload);
        }
        if bytes.len() > maximum_bytes {
            return Err(Error::ImageUploadTooLarge {
                actual: bytes.len(),
                max: maximum_bytes,
            });
        }

        let url = self
            .config
            .api_base_url()
            .join(route)
            .map_err(|_| Error::InvalidApiRoute(route.to_owned()))?;
        let mut retries = 0_u8;

        loop {
            let request = self.client.put(url.clone());
            let request = if multipart {
                let filename = if route.ends_with("avatar") {
                    format!("avatar.{}", format.extension())
                } else {
                    format!("banner.{}", format.extension())
                };
                let part = reqwest::multipart::Part::bytes(bytes.to_vec())
                    .file_name(filename)
                    .mime_str(format.content_type())
                    .map_err(sanitize_transport_error)?;
                request.multipart(reqwest::multipart::Form::new().part("file", part))
            } else {
                request
                    .header(CONTENT_TYPE, format.content_type())
                    .body(bytes.to_vec())
            };
            let mut response = request.send().await.map_err(sanitize_transport_error)?;

            let status = response.status();
            let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
            let response_body =
                read_limited_body(&mut response, self.config.max_response_bytes()).await?;

            if !status.is_success() {
                let error = ApiError::from_response(status, retry_after, &response_body);
                let retryable_status = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    || status == reqwest::StatusCode::SERVICE_UNAVAILABLE;
                if retryable_status && error.retry_after().is_some() && retries < 2 {
                    let delay = error.retry_after().expect("checked above");
                    retries += 1;
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(error.into());
            }

            return serde_json::from_slice(&response_body).map_err(Error::from);
        }
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

    #[test]
    fn preserves_rate_limit_metadata() {
        let body = serde_json::json!({
            "error": "slow down",
            "code": "rate_limited"
        });
        let api_error = ApiError::from_response(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            Some(std::time::Duration::from_secs(7)),
            body.to_string().as_bytes(),
        );

        assert_eq!(api_error.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(api_error.message(), "slow down");
        assert_eq!(api_error.code(), Some("rate_limited"));
        assert_eq!(
            api_error.retry_after(),
            Some(std::time::Duration::from_secs(7))
        );
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
                None,
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
    async fn send_layout_uses_components_v2_flag_and_reply_reference() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";
        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "message": {
                    "id": MESSAGE_ID,
                    "channel_id": CHANNEL_ID
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 2048).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let reply_to = crate::ids::MessageId::new(MESSAGE_ID).unwrap();
        let layout = vec![
            crate::models::TextDisplay::new("# Hello").unwrap().into(),
            crate::models::Container::new([
                crate::models::TextDisplay::new("A V2 card").unwrap().into(),
                crate::models::Separator::new(
                    true,
                    crate::models::SeparatorSpacing::Small,
                )
                .into(),
            ])
            .into(),
        ];

        let message = client
            .send_layout(&channel_id, &layout, Some(&reply_to))
            .await
            .unwrap();
        assert_eq!(message.id.as_str(), MESSAGE_ID);

        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["flags"], 1 << 15);
        assert_eq!(payload["reply_to"], MESSAGE_ID);
        assert_eq!(payload["components"][0]["type"], 10);
        assert_eq!(payload["components"][1]["type"], 17);
        assert_eq!(payload["components"][1]["components"][1]["type"], 14);
        assert!(payload.get("content").is_none());
        assert!(payload.get("embeds").is_none());
    }

    #[tokio::test]
    async fn edit_layout_patches_components_v2_flag_and_layout() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174002";
        const MESSAGE_ID: &str = "123e4567-e89b-12d3-a456-426614174001";
        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "message": {
                    "id": MESSAGE_ID,
                    "channel_id": CHANNEL_ID
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 2048).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let message_id = crate::ids::MessageId::new(MESSAGE_ID).unwrap();
        let layout = [crate::models::TextDisplay::new("Updated").unwrap().into()];

        client
            .edit_layout(&channel_id, &message_id, &layout)
            .await
            .unwrap();

        let request = server.finish();
        assert!(request.starts_with(
            "PATCH /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174002/messages/123e4567-e89b-12d3-a456-426614174001 HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["flags"], 1 << 15);
        assert_eq!(payload["components"][0]["type"], 10);
    }

    #[tokio::test]
    async fn send_layout_rejects_an_empty_layout_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174002").unwrap();

        assert!(matches!(
            client.send_layout(&channel_id, &[], None).await,
            Err(Error::LayoutValidation(
                crate::models::LayoutValidationError::EmptyLayout
            ))
        ));
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
    async fn gets_and_creates_roles_with_source_api_shapes() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const ROLE_ID: &str = "123e4567-e89b-12d3-a456-426614174023";

        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let role_id = crate::ids::RoleId::new(ROLE_ID).unwrap();

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "role": {
                    "id": ROLE_ID,
                    "name": "Moderator",
                    "position": 2
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let role = client.get_role(&server_id, &role_id).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/roles/123e4567-e89b-12d3-a456-426614174023 HTTP/1.1"
        ));
        assert_eq!(role.name.as_deref(), Some("Moderator"));

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "role": {
                    "id": ROLE_ID,
                    "name": "Moderator",
                    "position": 2
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let permissions = serde_json::json!({ "kick_members": true });
        let role = client
            .create_role(
                &server_id,
                "Moderator",
                Some("#5865f2"),
                true,
                Some(&permissions),
            )
            .await
            .unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/roles HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["name"], "Moderator");
        assert_eq!(payload["color"], "#5865f2");
        assert_eq!(payload["mentionable"], true);
        assert_eq!(payload["permissions"]["kick_members"], true);
        assert_eq!(role.position, Some(2));
    }

    #[tokio::test]
    async fn edits_and_deletes_roles() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const ROLE_ID: &str = "123e4567-e89b-12d3-a456-426614174023";

        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let role_id = crate::ids::RoleId::new(ROLE_ID).unwrap();

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "role": {
                    "id": ROLE_ID,
                    "name": "Senior Moderator",
                    "position": 4,
                    "mentionable": false
                }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let changes = crate::models::RoleEditOptions::new()
            .name("Senior Moderator")
            .clear_color()
            .mentionable(false)
            .position(4);
        let role = client
            .edit_role(&server_id, &role_id, &changes)
            .await
            .unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "PATCH /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/roles/123e4567-e89b-12d3-a456-426614174023 HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["name"], "Senior Moderator");
        assert_eq!(payload["color"], "");
        assert_eq!(payload["mentionable"], false);
        assert_eq!(payload["position"], 4);
        assert!(payload.get("permissions").is_none());
        assert_eq!(role.position, Some(4));

        let server = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&server, 1024).await;
        client.delete_role(&server_id, &role_id).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "DELETE /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/roles/123e4567-e89b-12d3-a456-426614174023 HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn rejects_empty_role_edit_options_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let server_id = crate::ids::ServerId::new("123e4567-e89b-12d3-a456-426614174020").unwrap();
        let role_id = crate::ids::RoleId::new("123e4567-e89b-12d3-a456-426614174023").unwrap();

        assert!(matches!(
            client
                .edit_role(&server_id, &role_id, &crate::models::RoleEditOptions::new())
                .await,
            Err(Error::EmptyRoleEditOptions)
        ));
    }

    #[tokio::test]
    async fn reorders_roles_with_ids_and_decodes_returned_order() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const ROLE_ONE: &str = "123e4567-e89b-12d3-a456-426614174023";
        const ROLE_TWO: &str = "123e4567-e89b-12d3-a456-426614174025";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "roles": [
                    { "id": ROLE_TWO, "name": "Admin", "position": 1 },
                    { "id": ROLE_ONE, "name": "Mod", "position": 2 }
                ]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let role_ids = [
            crate::ids::RoleId::new(ROLE_TWO).unwrap(),
            crate::ids::RoleId::new(ROLE_ONE).unwrap(),
        ];

        let roles = client.reorder_roles(&server_id, &role_ids).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "PATCH /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/roles HTTP/1.1"
        ));
        let body = request.split_once("\r\n\r\n").unwrap().1;
        let payload: Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["ids"][0], ROLE_TWO);
        assert_eq!(payload["ids"][1], ROLE_ONE);
        assert_eq!(roles[0].name.as_deref(), Some("Admin"));
        assert_eq!(roles[1].name.as_deref(), Some("Mod"));
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

    #[tokio::test]
    async fn creates_private_channel_with_initial_access_in_one_request() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const CATEGORY_ID: &str = "123e4567-e89b-12d3-a456-426614174024";
        const ROLE_ID: &str = "123e4567-e89b-12d3-a456-426614174023";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174021";

        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "channel": { "id": CHANNEL_ID, "name": "ticket-7", "type": "text", "private": true }
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let category_id = crate::ids::CategoryId::new(CATEGORY_ID).unwrap();
        let role_id = crate::ids::RoleId::new(ROLE_ID).unwrap();
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();
        let options = ChannelCreateOptions::new("ticket-7", ChannelType::Text)
            .category(category_id)
            .private(true)
            .role_ids([role_id])
            .user_ids([user_id]);

        let channel = client.create_channel(&server_id, &options).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/channels HTTP/1.1"
        ));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["name"], "ticket-7");
        assert_eq!(payload["kind"], "text");
        assert_eq!(payload["private"], true);
        assert_eq!(payload["category_id"], CATEGORY_ID);
        assert_eq!(payload["role_ids"][0], ROLE_ID);
        assert_eq!(payload["user_ids"][0], USER_ID);
        assert_eq!(channel.name.as_deref(), Some("ticket-7"));
    }

    #[tokio::test]
    async fn edits_channel_and_preserves_explicit_empty_access_lists() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174021";
        let server = LocalMockServer::start(200, &[], serde_json::json!({
            "channel": { "id": CHANNEL_ID, "name": "announcements", "type": "text", "slowmode": 30 }
        }).to_string());
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let changes = ChannelEditOptions::new()
            .name("announcements")
            .clear_category()
            .private(true)
            .role_ids(Vec::<crate::ids::RoleId>::new())
            .read_only(true)
            .post_role_ids(Vec::<crate::ids::RoleId>::new())
            .slowmode(30)
            .user_ids(Vec::<crate::ids::UserId>::new());

        let channel = client.edit_channel(&channel_id, &changes).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "PATCH /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174021 HTTP/1.1"
        ));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert!(payload["category_id"].is_null());
        assert_eq!(payload["role_ids"], serde_json::json!([]));
        assert_eq!(payload["post_role_ids"], serde_json::json!([]));
        assert_eq!(payload["user_ids"], serde_json::json!([]));
        assert_eq!(payload["read_only"], true);
        assert_eq!(payload["slowmode"], 30);
        assert_eq!(channel.slowmode, Some(30));
    }

    #[tokio::test]
    async fn validates_channel_edit_options_before_sending() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let channel_id =
            crate::ids::ChannelId::new("123e4567-e89b-12d3-a456-426614174021").unwrap();

        assert!(matches!(
            client
                .edit_channel(&channel_id, &ChannelEditOptions::new())
                .await,
            Err(Error::EmptyChannelEditOptions)
        ));
        assert!(matches!(
            client
                .edit_channel(&channel_id, &ChannelEditOptions::new().slowmode(21_601))
                .await,
            Err(Error::InvalidChannelSlowmode { actual: 21_601 })
        ));
        assert!(matches!(
            client
                .edit_channel(&channel_id, &ChannelEditOptions::new().name(" "))
                .await,
            Err(Error::EmptyChannelName)
        ));
    }

    #[tokio::test]
    async fn creates_renames_and_deletes_categories() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const CATEGORY_ID: &str = "123e4567-e89b-12d3-a456-426614174024";
        let create = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "category": { "id": CATEGORY_ID, "name": "Community" }
            })
            .to_string(),
        );
        let client = test_client(&create, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let category = client
            .create_category(&server_id, "Community")
            .await
            .unwrap();
        let request = create.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/categories HTTP/1.1"
        ));
        assert_eq!(category.name.as_deref(), Some("Community"));

        let rename = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "category": { "id": CATEGORY_ID, "name": "Staff" }
            })
            .to_string(),
        );
        let client = test_client(&rename, 1024).await;
        let category_id = crate::ids::CategoryId::new(CATEGORY_ID).unwrap();
        let category = client
            .rename_category(&server_id, &category_id, "Staff")
            .await
            .unwrap();
        let request = rename.finish();
        assert!(request.starts_with(
            "PATCH /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/categories/123e4567-e89b-12d3-a456-426614174024 HTTP/1.1"
        ));
        assert_eq!(category.name.as_deref(), Some("Staff"));

        let delete = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&delete, 1024).await;
        client
            .delete_category(&server_id, &category_id)
            .await
            .unwrap();
        let request = delete.finish();
        assert!(request.starts_with(
            "DELETE /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/categories/123e4567-e89b-12d3-a456-426614174024 HTTP/1.1"
        ));
    }

    #[tokio::test]
    async fn deletes_channel_and_rejects_empty_category_names() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174021";
        let server = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&server, 1024).await;
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        client.delete_channel(&channel_id).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "DELETE /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174021 HTTP/1.1"
        ));

        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        assert!(matches!(
            client.create_category(&server_id, "  ").await,
            Err(Error::EmptyCategoryName)
        ));
        assert!(matches!(
            client
                .rename_category(
                    &server_id,
                    &crate::ids::CategoryId::new("123e4567-e89b-12d3-a456-426614174024").unwrap(),
                    ""
                )
                .await,
            Err(Error::EmptyCategoryName)
        ));
    }

    #[tokio::test]
    async fn sets_bot_presence_through_the_documented_status_endpoint() {
        let server = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&server, 1024).await;

        client
            .set_status(BotStatus::Online, Some("Playing chess"))
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with("PUT /api/bot/v1/me/status HTTP/1.1"));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["status"], "online");
        assert_eq!(payload["activity"], "Playing chess");
    }

    #[tokio::test]
    async fn sets_typed_presence_with_a_formatted_activity_line() {
        let server = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&server, 1024).await;
        let presence =
            Presence::dnd().activity(crate::models::Activity::playing("Minecraft").unwrap());

        client.set_presence(&presence).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with("PUT /api/bot/v1/me/status HTTP/1.1"));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["status"], "dnd");
        assert_eq!(payload["activity"], "Playing Minecraft");
    }

    #[tokio::test]
    async fn rejects_overlong_bot_activity_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();

        assert!(matches!(
            client
                .set_status(BotStatus::Dnd, Some(&"x".repeat(61)))
                .await,
            Err(Error::BotActivityTooLong { actual: 61 })
        ));
    }

    #[tokio::test]
    async fn reads_member_permissions_from_the_source_parity_route() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";
        let server = LocalMockServer::start(
            200,
            &[],
            r#"{ "permissions": { "owner": false, "administrator": true, "permissions": { "manage_roles": true } } }"#.to_owned(),
        );
        let client = test_client(&server, 1024).await;
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();

        let permissions = client
            .get_member_permissions(&server_id, &user_id)
            .await
            .unwrap();
        let request = server.finish();

        assert!(request.starts_with(
            "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/members/123e4567-e89b-12d3-a456-426614174022/permissions HTTP/1.1"
        ));
        assert_eq!(permissions["administrator"], true);
        assert_eq!(permissions["permissions"]["manage_roles"], true);
    }

    #[tokio::test]
    async fn sets_and_clears_channel_permission_overrides() {
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174021";
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();
        let allow = vec!["send_messages".to_owned()];
        let deny = vec!["view_channel".to_owned()];

        let set = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&set, 1024).await;
        let target = PermissionTarget::Everyone;
        client
            .set_channel_permissions(&channel_id, &target, &allow, &deny)
            .await
            .unwrap();
        let request = set.finish();
        assert!(request.starts_with(
            "PUT /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174021/permissions/everyone HTTP/1.1"
        ));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["allow"][0], "send_messages");
        assert_eq!(payload["deny"][0], "view_channel");

        let clear = LocalMockServer::start(204, &[], String::new());
        let client = test_client(&clear, 1024).await;
        client
            .clear_channel_permissions(&channel_id, &target)
            .await
            .unwrap();
        let request = clear.finish();
        assert!(request.starts_with(
            "DELETE /api/bot/v1/channels/123e4567-e89b-12d3-a456-426614174021/permissions/everyone HTTP/1.1"
        ));
    }
    #[tokio::test]
    async fn patches_bot_profile_fields() {
        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "id": BOT_ID,
                "username": "profile-test",
                "display_name": "Yapcore",
                "bio": "A Rust bot"
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let edit = BotProfileEdit::new()
            .display_name("Yapcore")
            .bio("A Rust bot");
        let profile = client.edit_me(&edit).await.unwrap();
        let request = server.finish();

        assert!(request.starts_with("PATCH /api/bot/v1/me HTTP/1.1"));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["display_name"], "Yapcore");
        assert_eq!(payload["bio"], "A Rust bot");
        assert_eq!(profile.username.as_deref(), Some("profile-test"));
    }

    #[tokio::test]
    async fn uploads_and_deletes_bot_avatar_and_banner() {
        let avatar = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({ "path": "avatar/path.png", "url": "https://files.yapper.website/avatars/avatar/path.png" }).to_string(),
        );
        let client = test_client(&avatar, 1024).await;
        let asset = client
            .set_avatar(b"png-bytes", ImageFormat::Png)
            .await
            .unwrap();
        let request = avatar.finish();
        assert!(request.starts_with("PUT /api/bot/v1/me/avatar HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: image/png")
        );
        assert!(request.ends_with("png-bytes"));
        assert_eq!(asset.path, "avatar/path.png");

        let banner = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({ "path": "banner/path.webp", "url": "https://files.yapper.website/banners/banner/path.webp" }).to_string(),
        );
        let client = test_client(&banner, 1024).await;
        let asset = client
            .set_banner(b"webp-bytes", ImageFormat::Webp)
            .await
            .unwrap();
        let request = banner.finish();
        assert!(request.starts_with("PUT /api/bot/v1/me/banner HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: image/webp")
        );
        assert_eq!(asset.path, "banner/path.webp");

        let delete_avatar = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&delete_avatar, 1024).await;
        client.delete_avatar().await.unwrap();
        assert!(
            delete_avatar
                .finish()
                .starts_with("DELETE /api/bot/v1/me/avatar HTTP/1.1")
        );

        let delete_banner = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&delete_banner, 1024).await;
        client.delete_banner().await.unwrap();
        assert!(
            delete_banner
                .finish()
                .starts_with("DELETE /api/bot/v1/me/banner HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn uploads_profile_images_with_the_multipart_file_field() {
        let avatar = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "path": "avatar/multipart.png",
                "url": "https://files.yapper.website/avatars/avatar/multipart.png"
            })
            .to_string(),
        );
        let client = test_client(&avatar, 1024).await;
        let asset = client
            .set_avatar_multipart(b"avatar-image", ImageFormat::Png)
            .await
            .unwrap();
        let request = avatar.finish();
        assert!(request.starts_with("PUT /api/bot/v1/me/avatar HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: multipart/form-data; boundary=")
        );
        assert!(request.contains("name=\"file\""));
        assert!(request.contains("filename=\"avatar.png\""));
        assert!(request.contains("Content-Type: image/png"));
        assert!(request.contains("avatar-image"));
        assert_eq!(asset.path, "avatar/multipart.png");

        let banner = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "path": "banner/multipart.webp",
                "url": "https://files.yapper.website/banners/banner/multipart.webp"
            })
            .to_string(),
        );
        let client = test_client(&banner, 1024).await;
        let asset = client
            .set_banner_multipart(b"banner-image", ImageFormat::Webp)
            .await
            .unwrap();
        let request = banner.finish();
        assert!(request.starts_with("PUT /api/bot/v1/me/banner HTTP/1.1"));
        assert!(request.contains("name=\"file\""));
        assert!(request.contains("filename=\"banner.webp\""));
        assert!(request.contains("Content-Type: image/webp"));
        assert!(request.contains("banner-image"));
        assert_eq!(asset.path, "banner/multipart.webp");
    }

    #[tokio::test]
    async fn rejects_invalid_profile_and_image_uploads_before_networking() {
        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        assert!(matches!(
            client.edit_me(&BotProfileEdit::new()).await,
            Err(Error::EmptyBotProfileEdit)
        ));
        assert!(matches!(
            client.set_avatar(b"", ImageFormat::Png).await,
            Err(Error::EmptyImageUpload)
        ));
        assert!(matches!(
            client
                .set_avatar(&vec![0; 8 * 1024 * 1024 + 1], ImageFormat::Png)
                .await,
            Err(Error::ImageUploadTooLarge { max: 8_388_608, .. })
        ));
        assert!(matches!(
            client
                .set_banner(&vec![0; 10 * 1024 * 1024 + 1], ImageFormat::Jpeg)
                .await,
            Err(Error::ImageUploadTooLarge {
                max: 10_485_760,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn opens_direct_message_channel() {
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174031";
        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({ "channel_id": CHANNEL_ID }).to_string(),
        );
        let client = test_client(&server, 1024).await;
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();
        let channel_id = client.open_dm(&user_id).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/users/123e4567-e89b-12d3-a456-426614174022/dm HTTP/1.1"
        ));
        assert_eq!(channel_id.as_str(), CHANNEL_ID);
    }

    #[tokio::test]
    async fn reads_and_manages_voice_state() {
        const SERVER_ID: &str = "123e4567-e89b-12d3-a456-426614174020";
        const USER_ID: &str = "123e4567-e89b-12d3-a456-426614174022";
        const CHANNEL_ID: &str = "123e4567-e89b-12d3-a456-426614174021";
        let server_id = crate::ids::ServerId::new(SERVER_ID).unwrap();
        let user_id = crate::ids::UserId::new(USER_ID).unwrap();
        let channel_id = crate::ids::ChannelId::new(CHANNEL_ID).unwrap();

        let list = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({ "voice": [{ "channel_id": CHANNEL_ID, "user_ids": [USER_ID] }] })
                .to_string(),
        );
        let client = test_client(&list, 1024).await;
        let voice = client.get_voice_states(&server_id).await.unwrap();
        let request = list.finish();
        assert!(request.starts_with(
            "GET /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/voice HTTP/1.1"
        ));
        assert_eq!(voice.len(), 1);
        assert_eq!(voice[0].channel_id, channel_id);
        assert_eq!(voice[0].user_ids, vec![user_id.clone()]);

        let move_server = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&move_server, 1024).await;
        client
            .move_member_to_voice(&server_id, &user_id, &channel_id)
            .await
            .unwrap();
        let request = move_server.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/voice/move HTTP/1.1"
        ));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["user_id"], USER_ID);
        assert_eq!(payload["channel_id"], CHANNEL_ID);

        let disconnect = LocalMockServer::start(200, &[], r#"{ "ok": true }"#.to_owned());
        let client = test_client(&disconnect, 1024).await;
        client
            .disconnect_member_from_voice(&server_id, &user_id)
            .await
            .unwrap();
        let request = disconnect.finish();
        assert!(request.starts_with(
            "POST /api/bot/v1/servers/123e4567-e89b-12d3-a456-426614174020/voice/disconnect HTTP/1.1"
        ));
        let payload: Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["user_id"], USER_ID);
    }

    #[tokio::test]
    async fn lists_registered_commands() {
        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({
                "commands": [
                    { "name": "ping", "description": "Check latency" },
                    { "name": "help", "description": "Show help" }
                ]
            })
            .to_string(),
        );
        let client = test_client(&server, 1024).await;
        let commands = client.list_commands().await.unwrap();
        let request = server.finish();
        assert!(request.starts_with("GET /api/bot/v1/commands HTTP/1.1"));
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].name(), "ping");
        assert_eq!(commands[1].name(), "help");
    }

    #[tokio::test]
    async fn retries_a_429_only_when_retry_after_is_present() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server_thread = thread::spawn(move || {
            for (status, body, retry_after) in [
                (429, r#"{ "error": "slow down" }"#, Some("0")),
                (200, r#"{ "username": "retry-ok" }"#, None),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 512];
                loop {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read]);
                    if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let reason = if status == 429 {
                    "Too Many Requests"
                } else {
                    "OK"
                };
                let mut response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                if let Some(delay) = retry_after {
                    response.push_str(&format!("Retry-After: {delay}\r\n"));
                }
                response.push_str("\r\n");
                response.push_str(body);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        let config = Config::builder()
            .token("yb_test-token")
            .api_base_url(format!("http://{address}/api/bot/v1"))
            .allow_insecure_http_for_tests(true)
            .build()
            .unwrap();
        let client = ApiClient::new(config).unwrap();
        let profile = client.get_me().await.unwrap();
        server_thread.join().unwrap();
        assert_eq!(profile.username.as_deref(), Some("retry-ok"));
    }

    #[tokio::test]
    async fn does_not_retry_a_429_without_retry_after() {
        let server = LocalMockServer::start(429, &[], r#"{ "error": "not retryable" }"#.to_owned());
        let client = test_client(&server, 1024).await;
        let error = client.get_me().await.unwrap_err();
        let request = server.finish();
        assert!(request.starts_with("GET /api/bot/v1/me HTTP/1.1"));
        assert!(matches!(error, Error::Api(api) if api.retry_after().is_none()));
    }
    #[tokio::test]
    async fn raw_api_uses_authenticated_relative_routes_only() {
        let server = LocalMockServer::start(
            200,
            &[],
            serde_json::json!({ "username": "raw-api" }).to_string(),
        );
        let client = test_client(&server, 1024).await;
        let response = client.api(Method::GET, "me", None).await.unwrap();
        let request = server.finish();
        assert!(request.starts_with("GET /api/bot/v1/me HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bot yb_test-token")
        );
        assert_eq!(response["username"], "raw-api");

        let config = Config::builder().token("yb_test-token").build().unwrap();
        let client = ApiClient::new(config).unwrap();
        assert!(matches!(
            client
                .api(Method::GET, "https://attacker.invalid/steal", None)
                .await,
            Err(Error::InvalidApiRoute(_))
        ));
        assert!(matches!(
            client.api(Method::GET, "../outside", None).await,
            Err(Error::InvalidApiRoute(_))
        ));
        assert!(matches!(
            client.api(Method::GET, "/me", None).await,
            Err(Error::InvalidApiRoute(_))
        ));
    }
}
