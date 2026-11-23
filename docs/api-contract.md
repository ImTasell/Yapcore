# Yapper bot API contract

**Audit date:** 2026-11-23  
**Primary reference:** [Yapper developer guide](https://www.yapper.website/developers) (page last updated 18 October 2026)  
**Python comparison target:** [our-websites/yapper.py](https://github.com/our-websites/yapper.py), default branch `main`  
**Purpose:** record the public HTTP/event contract Yapcore should implement, and separate confirmed platform behavior from extra Python SDK behavior.

> **Contract source note:** This pass was checked against the human-readable developer guide (last updated 18 October 2026) on 23 November 2026. The linked OpenAPI 3.1 JSON (`/openapi.json`) and detailed `/developers/api` page could not be retrieved by the available browser, so this document does not claim a full schema-level comparison. No live request was made with a dedicated test bot; newly added endpoint behavior is mock-tested but has not yet been smoke-tested against a real Yapper bot.

## 1. Connection and authentication

| Item | Contract |
| --- | --- |
| Base URL | `https://www.yapper.website/api/bot/v1` |
| Transport | HTTPS REST; JSON request/response bodies by default |
| Authentication | `Authorization: Bot <bot-token>` on every bot API request |
| IDs | UUID strings |
| Bot token | Created in Settings → Developer; shown once. Treat as a secret and support token rotation |
| Bot account | Use an actual bot account. Self-bot automation of a normal user account is prohibited |
| Default event transport | Long polling; no public URL or port forwarding required |
| Optional event delivery | Signed HTTPS webhook configured in the Developer settings; polling may run alongside it |

Yapcore's `edit_me`, avatar and banner operations, DM open/send, voice state methods, and `GET /commands` reader are implemented and mock-tested. Avatar/banner uploads support both raw image bodies and multipart form requests using the documented `file` field. Do not use the Python package's alternate configured hostnames as the Rust default. Its current source names `https://bots.yapper.website/api/bot/v1` as `BASE` and `https://www.yapin.gg/api/bot/v1` as a fallback; the current public developer guide explicitly gives the `www.yapper.website` base above. Any compatibility host should be added only after independent verification.

### Cog-wide command checks

A Rust cog may override `check(&CommandContext) -> bool`. Yapcore runs this synchronous predicate for every command owned by that cog, for both prefix and slash invocations, before the command-specific asynchronous check and handler. Returning `false` skips the command without sending a response, matching the upstream cog guard behavior.

## Typed command arguments

Prefix commands can opt into local typed parsing without changing the documented remote command registration payload:

~~~rust
use yapcore::{CommandOption};

bot.command_with_options(
    "profile",
    "Update a profile",
    vec![
        CommandOption::string("display_name", "New display name")?.required(true),
        CommandOption::integer("age", "Age in years")?,
        CommandOption::boolean("public", "Make the profile public")?,
        CommandOption::user("member", "Target member")?,
    ],
    |context| async move {
        let options = context.options().expect("typed schema was registered");
        let name = options.get_string("display_name").unwrap();
        let age = options.get_integer("age");
        let public = options.get_boolean("public").unwrap_or(false);
        context.reply(format!("Updating {name}; age={age:?}; public={public}")).await?;
        Ok(())
    },
)?;
~~~

Options support string, integer, number, boolean, user ID, channel ID and role ID values; required flags; up to 25 typed choices; positional arguments; named `name=value` arguments; and single/double-quoted values. User/channel/role values remain opaque strings. Invalid, duplicate, missing or unknown arguments fail validation before the handler runs. This is a local prefix-argument parser only: publishing structured option metadata and parsing the platform's structured slash-command payload remain unverified until the upstream wire contract is confirmed.

## 2. Documented endpoint inventory

The following is the human-readable REST endpoint index as exposed by the developer guide. Path parameters are shown in braces for readability; actual routes use their ID values.

### Bot identity, profile and command registration

| Method | Route | Request / response notes |
| --- | --- | --- |
| GET | `/me` | Current bot profile/settings; includes avatar/banner URLs and bot metadata |
| PATCH | `/me` | `display_name` (max 32) and/or `bio` (max 190) |
| PUT | `/me/avatar` | Raw image body with image Content-Type or multipart field `file`; max 8 MB. Yapcore supports raw image bodies and validates the limit locally. |
| DELETE | `/me/avatar` | Remove avatar |
| PUT | `/me/banner` | Raw image or multipart field `file`; max 10 MB. Yapcore supports raw image bodies and validates the limit locally. |
| DELETE | `/me/banner` | Remove banner |
| PUT | `/me/status` | `{ "status": "online" | "idle" | "dnd", "activity": "..." }`; activity max 60. Implemented as `Bot::set_status` and typed `Bot::set_presence`; this is HTTP bot presence, not Discord RPC/IPC. |
| GET | `/commands` | Read registered commands; exposed as `Bot::list_commands`. |
| PUT | `/commands` | Replace the entire command list: `{ "commands": [{ "name": "ping", "description": "..." }] }` |

Command registration constraints: the current official developer guide states a maximum of 50 commands per bot; names are 1–32 lowercase letters, numbers, hyphens or underscores; descriptions max 100 characters; names must be unique. The upstream Python source currently defines a 100-command local limit, so its local acceptance limit conflicts with the public API guide. Yapcore follows the documented 50-command API cap to avoid sending command lists the published contract does not promise to accept. `GET /commands` reads the current registration; `PUT /commands` replaces the entire list, so Yapcore synchronizes the complete registry rather than one command at a time.

Typed presence example:

```rust
use yapcore::{Activity, BotStatus, Presence};

let presence = Presence::online().activity(Activity::playing("Minecraft")?);
bot.set_presence(&presence).await?;
```

`Activity::playing` formats ordinary display text (for example, `Playing Minecraft`). Yapper currently accepts one activity line, not Discord-style structured game metadata. Presence can be changed while the bot is running by calling `set_presence` again.

For automatic rotation, create an `ActivityRotation` with at least two validated activities and a non-zero interval, then pass it to `bot.rotate_presence(rotation).await?`. The first activity is applied immediately; the remaining entries cycle in order.

```rust
use std::time::Duration;
use yapcore::{Activity, ActivityRotation, BotStatus};

let rotation = ActivityRotation::new(
    [
        Activity::playing("Minecraft")?,
        Activity::new("Monitoring servers")?,
        Activity::new("Keeping the community safe")?,
    ],
    Duration::from_secs(30),
)?
.status(BotStatus::Online);

let rotation_handle = bot.rotate_presence(rotation).await?;
// Keep the bot running, then cleanly stop the rotation when appropriate.
rotation_handle.stop().await?;
```

Dropping the returned `PresenceRotationHandle` or calling `cancel()` requests cancellation; awaiting `stop()` cancels and waits for the task, returning any API update error. Cancellation wakes the timer or interrupts an in-flight status request. Cancelling the bot's `ShutdownHandle` also stops the rotation.

For a presence based on the bot's current server count, use the documented `GET /servers` endpoint through the built-in helper:

```rust
use std::time::Duration;
use yapcore::BotStatus;

// One-shot refresh: publishes `Overviewing N servers` and returns N.
let count = bot.refresh_server_count_presence(BotStatus::Online).await?;

// Or keep it synchronized automatically, checking the server list every 60s.
let monitor = bot
    .monitor_server_count_presence(Duration::from_secs(60), BotStatus::Online)
    .await?;

// Stop and await it when no longer needed.
monitor.stop().await?;
```

The monitor publishes the current count immediately, then refreshes it on each interval. It only sends a status update when the count changes. Temporary list/status request errors are logged and retried on the next interval. Dropping or cancelling the `ServerPresenceMonitorHandle`, or shutting down the bot, stops the monitor.

### Event polling
| Method | Route | Contract |
| --- | --- | --- |
| GET | `/events?after=<id>&wait=<seconds>[&max_age=<seconds>]` | Fetch events after the cursor |

- Maximum 100 events per response.
- `wait` may be up to 25 seconds.
- Events are ordered and each ID is delivered once.
- Event history is retained for one day.
- Initial cursor discovery: `after=0&wait=0` returns no events and the newest event ID in `last`.
- Normal polling: send the previous response's `last` as the next `after`.
- On a failed poll, repeat the same cursor; do not advance it.
- `max_age` is optional and capped at 86,400 seconds. It filters old events, but skipped events are permanently skipped while the cursor still moves forward.
- When catching up after a restart, slash-command events older than 15 minutes and expired button clicks are skipped.

**Yapcore rule:** keep cursor advancement explicit. Do not commit a later cursor before handling all earlier events in the returned page. Provide a clean-start mode and a resumable-cursor mode; make cursor persistence pluggable rather than requiring a database.

### Servers, channels, members and users

| Method | Route | Contract |
| --- | --- | --- |
| GET | `/servers` | Servers the bot has been added to |
| GET | `/servers/{server_id}` | Server details |
| GET | `/servers/{server_id}/channels?type=text|voice` | Visible channels in sidebar order, plus categories |
| GET | `/channels?type=text|voice` | Same channel listing across all servers |
| GET | `/channels/{channel_id}` | One visible channel |
| GET | `/servers/{server_id}/members?limit=200&after={user_id}` | Members sorted by user ID; default 200, max 500 per page |
| GET | `/servers/{server_id}/members/{user_id}` | One member, including role IDs, nickname and timeout |
| GET | `/servers/{server_id}/roles` | Roles in a server |
| GET | `/users/{user_id}` | Public user profile |

Channel objects documented in the guide include fields such as `id`, `name`, `type`, `server_id`, `position`, `category`, `category_id`, `private`, `read_only` and `slowmode`. Private channels are omitted unless the bot or one of its roles is allowed to see them.

### Permissions (experimental source-parity surface)

The public guide explains that bot actions are authorized through the bot's assigned server roles, but its endpoint overview does not currently enumerate permission-inspection or override-management routes. The upstream Python framework contains these routes and marks effective-permission methods as testing-only. An independent Rust SDK report observed HTTP 403 `feature_off` for some permission reads, so Yapcore exposes these methods as experimental and mock tests must not be treated as proof of live availability.

| Method | Route | Request / response notes |
| --- | --- | --- |
| GET | `/servers/{server_id}/members/{user_id}/permissions` | Effective server permissions for a member |
| GET | `/channels/{channel_id}/permissions/{user_id}` | Effective permissions for a member in a channel, including overrides |
| GET | `/channels/{channel_id}/permissions` | Channel override list |
| PATCH | `/servers/{server_id}/roles/everyone` | `{ "permissions": { ... } }` |
| PUT / DELETE | `/channels/{channel_id}/permissions/{target}` | `{ "allow": [...], "deny": [...] }`; target is `everyone` or a role ID |
| PUT / DELETE | `/servers/{server_id}/categories/{category_id}/permissions/{target}` | Category-level override |

Do not confuse these first-generation `permissions` routes with the newer upstream `overrides` routes (`/channels/{id}/overrides/...`). The latter are also marked testing-only in the Python source and are not implemented here. A live test bot and platform confirmation are required before removing the experimental label.

### Messages and reactions

| Method | Route | Contract |
| --- | --- | --- |
| GET | `/channels/{channel_id}/messages?limit=50&before={iso_time}` | Recent messages, oldest first; default 50, max 100; includes authors |
| GET | `/channels/{channel_id}/messages?after={iso_time}` | Messages forward from a timestamp, oldest first |
| GET | `/channels/{channel_id}/messages/{message_id}` | Fetch one message |
| POST | `/channels/{channel_id}/messages` | Send a message |
| PATCH | `/channels/{channel_id}/messages/{message_id}` | Edit the bot's own message |
| DELETE | `/channels/{channel_id}/messages/{message_id}` | Delete own message; deleting another user's message requires Manage Messages |
| POST | `/channels/{channel_id}/messages/bulk-delete` | Body `{ "ids": [...] }`; max 100; returns `deleted` and `skipped` |
| PUT | `/channels/{channel_id}/messages/{message_id}/pin` | Pin (Manage Messages) |
| DELETE | `/channels/{channel_id}/messages/{message_id}/pin` | Unpin (Manage Messages) |
| GET | `/channels/{channel_id}/messages/{message_id}/reactions` | List reactions |
| PUT | `/channels/{channel_id}/messages/{message_id}/reactions/{emoji}` | Add the bot's reaction |
| DELETE | `/channels/{channel_id}/messages/{message_id}/reactions/{emoji}` | Remove the bot's reaction |

Send payload fields documented: `content`, `reply_to`, `embeds`, `components`. Content is max 2,000 characters; `content` or `embeds` is required. An edit can include `content`, `embeds` and/or `components`; leave a key out to preserve the existing value and use `null` to remove it. A reply is represented by `reply_to`.

The developer guide does not describe ordinary message-file uploads in its bot message endpoint summary. The Python source implements multipart attachments, sticker IDs, nonce handling and a Components V2 send format. Treat those as unverified extensions until the detailed API schema or live test-bot calls confirm them.

### Signed event webhooks

When a bot is hosted behind a public HTTPS URL, Yapper can POST the same events configured for polling. The request carries `X-Yapper-Bot` and `X-Yapper-Signature: sha256=<hex HMAC-SHA256 of the raw request body>`, keyed with the webhook secret from Settings → Developer → Advanced. The envelope is `{ "id": 1234, "event": "message", "data": { ... } }`; it uses `event` rather than the polling envelope's `type`. Yapper waits three seconds for HTTP 200 and does not retry failed deliveries. The ID is shared with polling, so use event-ID deduplication and keep polling as a recovery path when missing events is unacceptable.

Yapcore's `verify_event_webhook(secret, raw_body, signature_header)` validates the HMAC before parsing the envelope and converts it to the common `EventEnvelope` model. It deliberately does not run an HTTP server, choose a web framework, send the 200 response, or maintain a cross-transport deduplication store. The caller must pass the original, unmodified raw body to the verifier.

### Lifecycle error callbacks

The SDK surface also exposes an `error` callback for runtime and handler failures. Yapcore reports these through `Bot::on_error`, using an `ErrorEvent` with a stage, optional event ID/type, command name, button custom ID, optional owning cog name (`cog`), and the error message. Failures inside an error callback are logged and are not recursively reported.

### Interactions

| Method | Route | Contract |
| --- | --- | --- |
| POST | `/interactions/{interaction_id}/{token}/callback` | Answer an interaction |

Documented response types:

- `reply`: new channel message.
- `ephemeral`: response visible only to the clicker.
- `update`: edit the message the click came from; omitted fields remain unchanged and `null` clears a field.
- `ack`: acknowledge without a visible message.

The documented event schema is for button clicks (`type: "button"`). Interaction requirements:

- Respond within 3 seconds.
- Interaction stays open for 15 minutes.
- At most 10 answers per interaction.
- Per clicker: 5 clicks per 10 seconds and 30 clicks per minute.
- The interaction token is sensitive and must not be logged or exposed.

The current public guide documents buttons. It does not document the Python package's select-menu and modal-submit interaction paths in this endpoint summary; keep those behind an unverified/experimental boundary.

Yapcore now exposes source-parity event handlers for those two upstream event types, without claiming they are part of the stable public API contract:

```rust
bot.on_select("food", |interaction| async move {
    // A select submission exposes its selected values as a JSON array.
    let selected = interaction.values().as_array().cloned().unwrap_or_default();
    interaction.reply(format!("Selected: {selected:?}")).await?;
    Ok(())
})?;

bot.on_modal("application", |interaction| async move {
    // Modal values are preserved as JSON keyed by each input's custom ID.
    let fields = interaction.values();
    interaction.reply(format!("Received application fields: {fields}")).await?;
    Ok(())
})?;
```

These handlers are matched by both interaction kind and exact custom ID, so a button with the same ID cannot accidentally trigger a select or modal callback. Select values and modal field payloads are retained as raw JSON for forward compatibility. The upstream Python source describes these payloads, but the current official public interaction summary does not; treat the Rust support as experimental until the full schema or a live test bot confirms it.

A button or select handler can open a typed modal as its first response:

~~~rust
use yapcore::{Modal, TextInput};

bot.on_button("apply", |interaction| async move {
    let modal = Modal::new(
        "application",
        "Apply",
        [TextInput::paragraph("why", "Why do you want to join?")?],
    )?;
    interaction.show_modal(&modal).await?;
    Ok(())
})?;
~~~

A modal response is claimed atomically as the interaction's first answer; trying to open one after replying or acknowledging returns InteractionAlreadyAnswered. The same Modal/TextInput builder can be passed to CommandContext::show_modal for a slash-command invocation; prefix-command contexts return InteractionRequired. Modal-submit handlers may read their values and answer normally, but cannot open another modal.

### Generic authenticated API escape hatch

The official SDKs expose `bot.api(method, path, body?)` for endpoints not wrapped by convenience methods. Yapcore exposes the same capability as `ApiClient::api` and `Bot::api`. The path is relative to the configured bot API base; absolute URLs, origin-relative paths, and traversal segments are rejected so the bot token cannot be redirected to an unrelated host. The response is returned as `serde_json::Value`.

### Member roles and moderation

| Method | Route | Contract |
| --- | --- | --- |
| PUT | `/servers/{server_id}/members/{user_id}/roles/{role_id}` | Add role (Manage Roles) |
| DELETE | `/servers/{server_id}/members/{user_id}/roles/{role_id}` | Remove role (Manage Roles) |
| PATCH | `/servers/{server_id}/members/{user_id}` | Set nickname; max 32, empty string clears (Manage Nicknames) |
| POST | `/servers/{server_id}/kick` | Body `{ "user_id": "..." }` |
| POST | `/servers/{server_id}/bans` | Body `{ "user_id": "..." }` |
| GET | `/servers/{server_id}/bans` | List bans |
| DELETE | `/servers/{server_id}/bans/{user_id}` | Unban |
| POST | `/servers/{server_id}/timeouts` | Body `user_id`, optional `minutes` and `reason` |

Timeouts default to 10 minutes, allow up to 40,320 minutes (28 days), and use 0 to clear. Reason is max 200 characters. Permission checks are performed using the bot's server roles; the library must surface 403 errors clearly rather than imply a successful action.

### Channels and categories

| Method | Route | Contract |
| --- | --- | --- |
| POST | `/servers/{server_id}/channels` | Create `text` or `voice` channel with `name`, optional `category_id` |
| PATCH | `/channels/{channel_id}` | Edit `name`, `category_id`, privacy/role access, read-only/post-role access and `slowmode` |
| DELETE | `/channels/{channel_id}` | Delete channel |
| POST | `/servers/{server_id}/categories` | Create category with `name` |
| PATCH | `/servers/{server_id}/categories/{category_id}` | Rename category |
| DELETE | `/servers/{server_id}/categories/{category_id}` | Delete category |

Manage Channels is required. Channel slowmode is measured in seconds and capped at 21,600.

### Voice state management

| Method | Route | Contract |
| --- | --- | --- |
| GET | `/servers/{server_id}/voice` | Returns voice occupancy: `{ "voice": [{ "channel_id": "...", "user_ids": [...] }] }` |
| POST | `/servers/{server_id}/voice/move` | Body `{ "user_id": "...", "channel_id": "..." }` |
| POST | `/servers/{server_id}/voice/disconnect` | Body `{ "user_id": "..." }` |

Moving/disconnecting requires Move Members and the bot must have sufficient role position; the target must be allowed into the destination channel. Limits: 30 moves/disconnects and 60 voice lookups per minute per bot. **Bots do not join voice calls themselves.** This is voice-state management, not audio streaming.

## 3. Event contract

Every polled event has the envelope `{ id, type, data, created_at }`. `channel_id` and `server_id` are common contextual fields; they may be null for events not tied to a channel/server.

| Event type | Documented data fields / behavior |
| --- | --- |
| `message` | `id, channel_id, channel_name, server_id, author_id, author, content, attachment, reply_to_id, created_at, mentioned, author_bot`; recognized registered prefix commands also include `command` and `args`. Bot's own messages and ordinary DMs are not delivered. |
| `member_join` | `server_id, channel_id: null, user_id, username, joined_at, member_count`; not sent when another bot joins |
| `member_leave` | `server_id, channel_id: null, user_id, username, joined_at: null, member_count` |
| `reaction_add` / `reaction_remove` | `message_id, channel_id, server_id, user_id, username, emoji, message_author_id` |
| `command` | Slash command selected from the `/` menu. The SDK source wraps command events using `id, token, command, args, user_id, username, channel_id`; callback replies use the interaction response route. The public event overview names this event but does not fully enumerate its payload schema, so verify against the current OpenAPI document/test bot. |
| `interaction` | `id, token, type: "button", custom_id, message_id, channel_id, server_id, user_id, username, expires_at` |
| `voice_join` / `voice_leave` | `user_id, channel_id, server_id`; a move may be delivered as one leave and one join in either order |
| `test` | `bot, by, at, hello`; generated by the Developer Portal's Send test event button |

Ordinary DM messages are not delivered as events. Bots may still open a DM with `POST /users/{user_id}/dm` and send to the returned channel ID.

### Optional event webhook delivery

A bot with a public HTTPS endpoint may configure event delivery in Settings → Developer → Advanced. Polling remains available and can be used alongside the webhook.

Headers:
- `X-Yapper-Bot: <bot id>`
- `X-Yapper-Signature: sha256=<hex HMAC-SHA256 of raw request body>`

Signature key: the bot's configured webhook secret. Verify the HMAC over the exact raw body before parsing/trusting the event. The webhook envelope is `{ id, event, data }` (note `event`, rather than the polling envelope's `type`). Respond HTTP 200 quickly; Yapper waits 3 seconds and does not retry. The event ID matches the polling ID, so use deduplication and keep polling as a recovery channel when missed events are unacceptable.

This is distinct from a channel webhook URL used by external services to post messages into a channel. It is also distinct from the Python package's bot-managed webhook CRUD methods.

## 4. Payload limits and rate limits

| Area | Current documented limit |
| --- | --- |
| Polling | 100 events per response; wait up to 25 seconds; event retention one day |
| Catch-up | `max_age` up to 86,400 seconds |
| Commands | 50 commands per bot; name 1–32 chars; description max 100 |
| Message content | 2,000 characters |
| Messages | 30/minute per server and 30/minute overall per bot; verified bots may send 300/minute overall; DM limit is per DM |
| Message history | Default 50; max 100 |
| Member pagination | Default 200; max 500 |
| Bulk delete | 100 IDs |
| Embeds | 5 per message; max 10 fields per embed; 6,000 characters across embeds; title 256; description 4,096; author name 256; field name 256/value 1,024; footer 2,048 |
| Embed URLs | Link URL max 512; links may be HTTP or HTTPS |
| Embed pictures | HTTPS only, and host must be on Yapper's allowlist |
| Buttons | Up to 5 rows, up to 5 buttons per row; label 80; custom ID 1–100 chars and unique within a message; link button URL must be HTTPS |
| Interaction response | Initial answer within 3 seconds; interaction lifetime 15 minutes; max 10 answers |
| Interaction clicks | Per person: 5 per 10 seconds and 30 per minute |
| Profile | Display name 32; bio 190; status activity 60 |
| Avatar/banner upload | Avatar 8 MB; banner 10 MB |
| Nickname | 32 characters |
| Timeout | Up to 40,320 minutes; reason 200 characters |
| Channel slowmode | Up to 21,600 seconds |
| Voice state methods | 30 move/disconnect actions and 60 voice lookups per minute per bot |

Message rate-limit details: replies to a slash command (within 15 minutes, first 5 replies per command) and the first 5 `reply` answers to each button press do not count against the bot-wide overall limit; they still count against the server's limit. Rate-limit responses include `Retry-After` and `retry_after` details.

## 5. Error and retry contract

The public guide documents these statuses:

| HTTP | Meaning |
| --- | --- |
| 400 | Bad input |
| 401 | Missing/invalid bot token |
| 403 | Not allowed: bot not in server or missing permission |
| 404 | Not found or not visible to bot |
| 409 | Conflict, including a target not currently in voice |
| 429 | Rate limit |
| 503 | Yapper busy |

The documented error body is `{ "error": "..." }`. A 429 with `Retry-After` is safe to retry after the specified wait. A 429 without that header may represent a non-retryable condition (for example, duplicate/saturated interaction answers), so do not blindly retry. A 503 with `Retry-After` means the request was not performed and is safe to retry after the wait.

**Rust retry policy:**
1. Yapcore retries HTTP 429 and 503 responses only when a valid `Retry-After` value is present.
2. Retries wait for the server-provided delay and are bounded to two retries per request.
3. A 429 without `Retry-After` is returned immediately.
4. Transport failures are not automatically replayed; there is no blanket retry of requests whose delivery outcome is unknown.
5. The same bounded Retry-After policy is applied to raw avatar/banner uploads.
6. Preserve status, parsed API message and Retry-After in the public error where practical.
7. Never log the bot token, interaction token or event-webhook secret.

The Python source has richer parsing assumptions (`code`, `permission`, nested error dictionaries) than the simple error envelope in the public guide. Preserve unknown error fields if present, but do not require undocumented properties to parse a normal API error.

## 6. Python framework comparison

The upstream Python framework is much broader than the documented overview. The following classification is based on the public developer guide plus calls made by `src/yapper/main.py`.

### Directly covered by the public guide

These should form the stable Rust feature baseline:

- Bot authentication, `GET /me`, profile edits, status, avatar/banner.
- Command registration and the 50-command list.
- Event polling and documented event delivery.
- Message send/edit/delete/fetch, replies, embeds, buttons, reactions, pinning and bulk delete.
- Interaction callback: reply, ephemeral, update and acknowledgement.
- Server/channel/member/role listing.
- Member role assignment/removal and nickname edits.
- Kick, ban, unban, timeout and ban listing.
- Channel/category creation, editing, deletion and listing.
- DM creation and sending (but no incoming DM message events).
- Voice occupancy, move/disconnect and voice join/leave events.
- Optional signed event webhooks.

### Present in Python source but not confirmed by the retrieved public endpoint index

Keep these out of the stable API or mark them experimental until the detailed API reference/OpenAPI file or a live test-bot response confirms them:

| Python API / route family | Concern |
| --- | --- |
| `member_counts` → `GET /servers/{id}/counts` | Counts route not in the overview endpoint table |
| `resolve` → `GET /resolve/{id}` | Resolve route not in the overview endpoint table |
| Role CRUD/reorder → `POST/PATCH/DELETE /servers/{id}/roles`, `PATCH /servers/{id}/roles` | The overview confirms listing roles and assigning/removing roles, not full role management |
| Permission APIs → `/servers/{id}/roles/everyone`, `/channels/{id}/permissions`, `/channels/{id}/overrides`, category override routes | Detailed permission read/write endpoints not in the overview endpoint table |
| Emoji APIs → `/servers/{id}/emojis`, `/channels/{id}/messages/{id}/emojis` | Custom emoji CRUD/lookup not in the overview endpoint table |
| Invite listing → `GET /servers/{id}/invites` | Not in the overview endpoint table |
| Thread APIs → `/channels/{id}/threads`, `/threads/{id}` | Thread routes are not in the endpoint index; threads are also listed as roadmap work |
| Scheduled events → `/servers/{id}/events` | Not in the overview endpoint table |
| Bot-managed webhook CRUD → `/channels/{id}/webhooks`, `/webhooks`, `/webhooks/{id}` | Distinct from documented inbound channel webhooks and signed event webhooks |
| Removing other users' reactions | Python exposes broader deletion/query parameters than the documented add/remove-own-reaction surface |
| Select menu and modal interactions | Python supports `Select`, `TextInput`, `Modal`; public interaction contract describes button clicks |
| Components V2 layouts | Python has `Container`, `Section`, `TextDisplay`, gallery/file/separator components; not specified in the retrieved endpoint overview |
| Message file attachments and stickers | Python builds multipart uploads and carries sticker IDs; endpoint overview only describes text/embed/component payloads |
| Extra event names | Python lists `dm_message`, `command`, invite/channel/thread/category/message-update/delete/server lifecycle events not all present in the documented event table; ordinary DM messages are explicitly not delivered |
| `bot.join_voice` / `bot.leave_voice` and `/voice/join`, `/voice/leave` | Conflicts with explicit public statement that bots do not join calls |
| `Bot.connect_voice` / `VoiceCall` using LiveKit and E2EE key exchange | Not supported by the public bot contract; do not port |

A source comment pointing to a detailed API-reference URL is useful evidence, but is not enough to mark a capability as part of the currently retrieved public contract. Verify it against the full linked reference or the OpenAPI schema first.

### SDK-only behavior (not API endpoints)

These are framework design features rather than Yapper HTTP routes:

- Command argument parser and checks.
- Prefix matching, event dispatch and handler error routing.
- Cog ownership/registration and unload/reload.
- Builder classes for embeds, buttons and components.
- `Data`, `Message`, `Interaction` convenience wrappers.
- Local update/version checks.
- Retry policy and fallback host behavior.
- Optional telemetry client.

The separate telemetry receiver/panel under upstream `server/` is not part of the Python client package and is not part of the Yapper bot API.

## 7. Key corrections for the initial architecture

1. **Use the public API host:** `https://www.yapper.website/api/bot/v1`.
2. **Ship long polling first:** it is the documented default and supports laptops behind NAT without a public listener.
3. **Do not implement the Python WebSocket gateway yet:** it is not present in the current public guide's bot transport contract.
4. **Do not implement LiveKit bot audio:** the official guide explicitly says bots do not join calls.
5. **Treat voice features as moderation/state management:** GET occupancy, move members, disconnect members and consume voice join/leave events.
6. **Implement the simple documented interaction contract first:** button callbacks and the four documented response types.
7. **Keep undocumented upstream surface modular:** add it only after schema/live validation.
8. **Use a method-aware retry policy:** retry-after is not permission to replay every arbitrary POST.
9. **Avoid trusting source-only SDK defaults:** its configured base host, richer error fields and endpoint list do not supersede the live public developer contract.

## 8. Required verification before API parity is declared complete

- [ ] Retrieve and archive the current OpenAPI 3.1 JSON from the linked official URL.
- [ ] Resolve the exact detail schemas for all response envelopes, nullable fields and query parameter types.
- [ ] Compare documented route list with current source-only route families.
- [ ] Confirm whether the `interactions: true` property sent by the Python framework in `PUT /commands` is required, ignored or legacy.
- [ ] Confirm whether the current server accepts multipart message attachments, sticker IDs and nonces.
- [ ] Confirm support and current schemas for selects, modals and Components V2.
- [ ] Confirm whether role CRUD, permission overrides, emojis, invites, threads, scheduled events and bot-managed webhooks are supported public endpoints.
- [ ] Confirm all error response fields and exactly when 429 responses include Retry-After.
- [ ] Run mock-server contract tests in CI and manual smoke tests using a dedicated test bot.
- [ ] Keep all real bot credentials out of Git and test output.

**No live test-bot API request was made for this audit or implementation pass.** The human-readable developer guide was retrieved; the linked OpenAPI schema was not. New route behavior is mock-tested. The final verification checklist remains open for an end-to-end smoke test using a dedicated bot.

## 9. References

- Yapper developer guide: <https://www.yapper.website/developers>
- Yapper OpenAPI 3.1 link: <https://www.yapper.website/openapi.json>
- Yapper detailed API reference: <https://www.yapper.website/developers/api>
- Yapper Terms (bot-account and token requirements): <https://www.yapper.website/terms>
- Python framework README: <https://github.com/our-websites/yapper.py/blob/main/README.md>
- Python framework implementation: <https://github.com/our-websites/yapper.py/blob/main/src/yapper/main.py>
- Python framework package metadata: <https://github.com/our-websites/yapper.py/blob/main/pyproject.toml>
