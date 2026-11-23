# Feature parity matrix

**Target:** `our-websites/yapper.py` version advertised by package metadata: 1.8.5  
**Audit date:** 2026-11-21

This matrix separates the Python framework's API surface from what the current official Yapper documentation confirms. "Documented" means described by <https://www.yapper.website/developers>; "verify" means present in Python source but not clearly supported by that documentation.

## Status legend

- [ ] Not implemented
- [x] Implemented and tested
- [~] Partial / experimental
- **Documented**: current official API guide describes it
- **Verify**: Python source supports it, but official docs do not clearly guarantee it
- **Defer**: requires clarification, platform support or a separate design decision

Implementation status updated 2026-11-23. Stable REST primitives, prefix-command dispatch and the initial polling runtime now exist; permission routes are exposed as experimental source-parity methods pending live confirmation. Documented bot presence/status is implemented.

## 1. Configuration and lifecycle

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Construct bot with explicit token | `Bot::builder().token(...)` |
| [x] Documented | Read `YAPPER_BOT_TOKEN` | `token_from_env` with missing/invalid-token error |
| [x] Documented | Configurable API base URL | `Config.api_base_url`, defaulting to official documented base |
| [x] Documented | Prefix list, default `/` and `!` | `BotBuilder::prefixes`, longest prefix first |
| [ ] Documented | Logging toggle | Integrate with optional `tracing`; no mandatory logger |
| [x] Documented | Ignore bot-authored messages by default | `BotBuilder::ignore_bots`, default true |
| [~] Documented | Start, stop and graceful shutdown | Async `start().await` plus cancellation handle; bounded drain and service-signal helpers remain |
| [ ] Verify | Automatic version check | Optional, non-blocking and opt-out; decide whether it is desirable |
| [ ] Verify | Alternate API hostname fallback | Do not add until host equivalence and retry safety are confirmed |

## 2. HTTP client and errors

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Auth header `Authorization: Bot <token>` | Central request middleware; sensitive header and redirects disabled |
| [x] Documented | JSON request/response handling | Typed Serde models plus raw JSON extension boundaries |
| [x] Documented | Error status and message | Structured `Error` / `ApiError` |
| [x] Verify | Error code and permission fields | Preserve when present in server response |
| [ ] Documented | 429 / 503 Retry-After handling | Shared retry policy; respect server-provided wait |
| [ ] Verify | Proxy retry/fallback behavior | Retry only when safe; never duplicate a message send |
| [x] Documented | Configurable request timeout | Builder/config option; 40-second default |
| [ ] Documented | Multipart/file sending | Bounded streaming upload; verify current per-file/total limits |

Retry safety rules:
- GET can be retried for transient transport/server failures.
- Respect `Retry-After` for 429 and documented retryable 503 responses.
- Do not automatically replay non-idempotent POST requests unless the API supports a stable idempotency key/nonce and the same key is reused.
- Expose retry attempts and final status in diagnostics without leaking credentials.

## 3. Event transport and lifecycle

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Long poll `GET /events?after=&wait=` | `ApiClient::poll_events` and `Bot::run` |
| [ ] Documented | Cursor-based ordered event delivery | Track monotonically increasing event ID |
| [x] Documented | Up to 100 events per poll, up to 25-second wait | Wait capped at 25 seconds; response body limit enforced |
| [ ] Documented | One-day event retention | Explain restart/cursor implications |
| [x] Documented | Optional `max_age` catch-up filter | `poll_events` validates one-day maximum |
| [x] Documented | Ready lifecycle callback | Synthetic `ready` event after profile retrieval and command synchronization |
| [ ] Documented | Error lifecycle callback | Typed lifecycle notification for fatal/runtime errors |
| [ ] Documented | Event webhooks as a server-side delivery option | Not needed for normal outbound-only client; document as deployment mode |
| [ ] Verify | WebSocket gateway `wss://rt.yapper.website/bot` | Defer; current public guide does not document its protocol |
| [ ] Verify | Invite/channel/thread/category/message/server extra event kinds | Preserve unknown event payloads; add typed events only after endpoint confirmation |
| [ ] Verify | DM message event | Official docs say ordinary DMs are not delivered as bot events |
| [x] Documented | Generic asynchronous event listeners | `Bot::on_event` delivers the full event envelope in registration order |
| [x] Documented | Filter bot-authored messages | Configurable bot-message filter, default on |
| [~] Documented | Ignore duplicate/out-of-order event IDs | Deduplicates against the in-memory cursor; persisted cursor/recovery is not implemented |

The official docs say event history is retained for one day. A persistent cursor store is useful for catch-up across restarts, but users must be able to choose a clean-start policy too.

## 4. Commands and argument parsing

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Register command names/descriptions | Validated local command registry |
| [x] Documented | Slash command synchronization | `Bot::run` synchronizes the full command registry before polling |
| [x] Documented | Prefix-command parsing | Longest matching prefix, case-insensitive name handling |
| [x] Documented | `args` and `argv` | Typed `CommandContext` argument access |
| [x] Documented | 50 command maximum per bot (Python SDK local cap is 100) | Rust follows the stricter official platform cap before full-list sync |
| [ ] Documented | Per-command permission/check callback | Async check hook |
| [ ] Verify | Typed options: string/integer/number/boolean/user/channel/role | Typed option schema and parser |
| [ ] Verify | Choices and named `key=value` input | Parser compatibility tests |
| [ ] Verify | Usage/error replies on invalid arguments | Optional built-in validation response |
| [x] Documented | Command event distinct from regular message | Route slash `command` events to interaction-aware command contexts |
| [~] Documented | Auto-acknowledge unanswered slash interactions | Two-second auto-ack fallback and ten-response counter; live timing still needs a test-bot smoke test |

Rust API should favor explicit typed command registration and async handlers. It does not need to imitate Python decorators.

## 5. Messages, models and interactions

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Send plain-text message | `ApiClient::send_message` / `Bot::send_message` |
| [x] Documented | Reply to message | `ApiClient::reply_message`; `CommandContext::reply` |
| [x] Documented | Send embed(s) | `Embed::builder` and `send_message_with_embeds`; validates message-wide limits |
| [x] Documented | Edit/delete own message | `ApiClient::edit_message` / `delete_message`; `Bot` convenience wrappers |
| [x] Documented | Fetch one/recent/forward messages | `get_message`, `get_recent_messages(limit, before)`, and `get_messages_after(after)`; page limits are validated locally |
| [x] Documented | Bulk delete | `bulk_delete_messages` validates the 1–100 ID limit and preserves deleted/skipped results |
| [x] Documented | Pin/unpin | `pin_message` / `unpin_message` REST methods |
| [x] Documented | Add/remove/list reactions | List, add and remove; emoji is encoded as one path segment |
| [ ] Verify | Four-file multipart send and sticker IDs | Confirm current upload contract and limits |
| [x] Documented | Button interactions: reply/ephemeral/update/ack | Exact `on_button` and prefix `button` registrations; shared secret-safe response session and two-second auto-ack |
| [~] Documented | Interaction expiry and answer count limits | Enforces ten-response cap and auto-ack margin; explicit expires_at tracking remains |
| [ ] Verify | Select menu handlers | Python package supports selects; official page's interaction examples focus on buttons |
| [ ] Verify | Modal and text-input components | Present in Python source; confirm current server payload schema |
| [ ] Verify | Components V2 layouts | Present in Python source; confirm official schema before finalizing |
| [x] Documented | Unknown message/event JSON fields | Preserve unknown fields in message models and raw event payloads |

## 6. Embeds, buttons and component builders

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Embed title/description/url/color/author/footer/fields/image/thumbnail/timestamp | Typed `Embed`, `EmbedAuthor`, `EmbedFooter`, and `EmbedField` builders |
| [x] Documented | Up to 5 embeds and 6000 aggregate characters | Local validation across all embeds before sending |
| [x] Documented | Button row and button styles | Typed `Button` and `ActionRow` builders with message-wide validation |
| [x] Documented | Link buttons | Link style validates HTTPS URLs |
| [x] Documented | Disabled buttons and emoji | `disabled` and `emoji` builder methods |
| [ ] Verify | Select menu option builders | Include after schema confirmation |
| [ ] Verify | Modal/TextInput builders | Include after schema confirmation |
| [ ] Verify | Components V2: text, section, gallery, separator, file, container | Dedicated typed component enum once protocol is confirmed |
| [ ] Documented | Plain JSON interoperability | Accept raw `serde_json::Value` as escape hatch |

Do not expose both a huge builder API and dozens of raw JSON methods for the same behavior. The builder layer should serialize to stable API payload types.

## 7. Server and member management

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | List/get servers | `list_servers` / `get_server` with forward-compatible models |
| [x] Documented | List server/global channels and channel details | `list_channels` with text/voice filters, categories, and `get_channel` |
| [x] Documented | List members with pagination / fetch one member | `list_members(limit, after)` and `get_member`; validates 1–500 page size |
| [~] Mixed | Roles list/get/create/edit/delete/reorder | `list_roles` is documented; get/create/edit/delete/reorder are implemented from upstream source, mock-tested, and await live contract verification. `RoleEditOptions` provides a typed patch builder |
| [x] Documented | Add/remove role | `add_role` / `remove_role` with shared 403 error handling |
| [~] Verify | Get effective permissions, @everyone permissions and channel/category overrides | Source-parity routes added as experimental; some are testing-only and a separate SDK report observed `feature_off`; require live confirmation before stable support |
| [x] Documented | Kick/ban/unban/list bans/timeout | Typed ban models, role-safe REST methods and local timeout/reason validation |
| [x] Documented | Edit nickname | `set_nickname` validates 32 characters; empty string clears |
| [x] Documented | Create/edit/delete channel | `create_channel`, `edit_channel` and `delete_channel`; typed create/edit options validate names, private access lists and the 21,600-second slowmode cap |
| [x] Documented | Create/rename/delete category | `create_category`, `rename_category` and `delete_category` |
| [ ] Documented | Open DM/send DM | User DM endpoints |
| [ ] Verify | Voice state lookup and move/disconnect members | Source-only routes; verify availability and payloads |
| [x] Documented | Set bot status and activity | `Bot::set_status` plus typed `Bot::set_presence(&Presence)`; `Activity::playing` formats the plain activity line |
| [x] SDK feature | Automatic activity rotation | `ActivityRotation` cycles validated activities; `Bot::rotate_presence` returns a cancellable `PresenceRotationHandle` |
| [ ] Verify | Scheduled server events | Source has list/create; verify current public API schema |
| [ ] Verify | Thread list/create/edit/delete | Source has methods; verify endpoint availability |
| [ ] Verify | Emoji create/edit/delete/steal/message emoji lookup | Source has methods; verify endpoint availability |
| [ ] Verify | Fine-grained channel/category permission overrides | Source has methods; verify routes and payload schema |
| [ ] Verify | Invite create/delete event surface | Source event list has event names; verify routes/data |

All permission-dependent actions must return a useful structured 403 error. The library must not imply that a bot can act without the corresponding server role/permission.

## 8. Webhooks and voice

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | Event webhooks (inbound HTTPS POST to user's server) | Documentation/reference only unless user wants a webhook server helper |
| [ ] Verify | Bot-managed channel webhooks: create/list/send/delete | Validate source endpoints and permission requirements |
| [ ] Verify | Webhook wrapper `send` and `delete` | Include after API confirmation |
| [ ] Documented | Voice presence, member move and disconnect | Implement as normal REST API |
| [ ] Defer | Bot joining calls and sending audio through LiveKit | Official docs currently state bots do not join calls; require explicit confirmation |
| [ ] Defer | E2EE voice key exchange and `VoiceCall` lifecycle | Security-sensitive and undocumented in the current public contract |

Do not treat voice presence management as equivalent to voice media streaming. They are separate capabilities.

## 9. Cogs, extensions and telemetry

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | Cog groups commands/listeners/buttons | A `Cog` trait or builder-based registration group |
| [ ] Documented | Setup function/extensions | Explicit setup function or module registration |
| [ ] Documented | Load/unload/reload command groups | Runtime registration lifecycle |
| [ ] Verify | File-system discovery of `.py` cogs | Rust cannot import source files dynamically; defer or design plugin ABI separately |
| [ ] Documented | Error attribution to cog/extension | Handler metadata in diagnostics |
| [ ] Verify | Opt-in telemetry client | Separate optional feature, off by default, report preview |
| [ ] Out of scope | Telemetry collection web server/panel under upstream `server/` | Not part of a client SDK port unless explicitly requested |

## 10. Suggested milestone order

1. **M0 — Decisions: COMPLETED.** GPL-3.0-only selected; source audit, feature inventory, API contract, architecture and dependency decisions are documented.
2. **M1 — Core client:** configuration, token handling, reqwest client, error types, typed IDs and raw API escape hatch.
3. **M2 — Event loop:** official long polling, cursor management, event models, graceful shutdown and error propagation.
4. **M3 — Commands:** command registry, synchronization, prefix parser, typed option parser and async handlers.
5. **M4 — Messaging:** messages, embeds, replies, editing/deleting, reactions and buttons/interactions.
6. **M5 — Guild API:** servers, members, channels, roles, permissions and moderation.
7. **M6 — Components and cogs:** builders, select/modal schema (after verification), modular handler registration.
8. **M7 — Extended API:** webhooks, threads, scheduled events, emojis and other undocumented features after live validation.
9. **M8 — Optional integrations:** telemetry opt-in; voice media only if platform support is confirmed.
10. **M9 — Release quality:** docs, examples, compatibility tests, CI, crate metadata and versioning.

