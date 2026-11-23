# Feature parity matrix

**Target:** `our-websites/yapper.py` version advertised by package metadata: 1.8.5  
**Audit date:** 2026-11-23

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
| [x] SDK feature | Logging toggle | `BotBuilder::logging(bool)` gates Yapcore lifecycle and handler logs; applications still choose their global `tracing` subscriber |
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
| [x] Documented | 429 / 503 Retry-After handling | Shared request layer retries 429/503 only when `Retry-After` is present; at most two retries, no transport-error replay |
| [ ] Verify | Proxy retry/fallback behavior | Retry only when safe; never duplicate a message send |
| [x] Documented | Configurable request timeout | Builder/config option; 40-second default |
| [~] Documented | Multipart profile image upload | Raw-body avatar/banner upload is implemented; multipart field `file` remains unimplemented. Message attachments are not in the published bot-message endpoint contract. |

Retry safety rules:
- GET can be retried for transient transport/server failures.
- Respect `Retry-After` for 429 and documented retryable 503 responses.
- Do not automatically replay non-idempotent POST requests unless the API supports a stable idempotency key/nonce and the same key is reused.
- Expose retry attempts and final status in diagnostics without leaking credentials.

## 3. Event transport and lifecycle

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Long poll `GET /events?after=&wait=` | `ApiClient::poll_events` and `Bot::run` |
| [x] Documented | Cursor-based ordered event delivery | Monotonic event IDs, in-memory checkpoint, `Bot::last_event_id()` and `Bot::run_from(saved_cursor)` for caller-managed resume |
| [x] Documented | Up to 100 events per poll, up to 25-second wait | Wait capped at 25 seconds; response body limit enforced |
| [x] Documented | One-day event retention | `run_from` resumes from a caller-persisted cursor within Yapper's one-day retention; persistence remains application-owned |
| [x] Documented | Optional `max_age` catch-up filter | `poll_events` validates one-day maximum |
| [x] Documented | Ready lifecycle callback | Synthetic `ready` event after profile retrieval and command synchronization |
| [x] Documented | Error lifecycle callback | `Bot::on_error` receives typed `ErrorEvent` values for fatal startup/poll failures and dispatch, listener, command and button-handler failures |
| [x] Documented | Event webhooks as a server-side delivery option | `verify_event_webhook` authenticates the raw body using the documented HMAC-SHA256 header and parses `EventWebhook`; the application owns its HTTPS receiver, fast acknowledgement and deduplication |
| [ ] Verify | WebSocket gateway `wss://rt.yapper.website/bot` | Defer; current public guide does not document its protocol |
| [x] Documented | Member, reaction, voice and test events | Generic `Bot::on_event` supports the documented `member_join`, `member_leave`, `reaction_add`, `reaction_remove`, `voice_join`, `voice_leave` and `test` envelopes |
| [ ] Verify | Invite/channel/thread/category/message/server extra event kinds | Preserve unknown event payloads; add typed events only after endpoint confirmation |
| [ ] Verify | DM message event | Official docs say ordinary DMs are not delivered as bot events |
| [x] Documented | Generic asynchronous event listeners | `Bot::on_event` delivers the full event envelope in registration order |
| [x] Documented | Filter bot-authored messages | Configurable bot-message filter, default on |
| [x] Documented | Ignore duplicate/out-of-order event IDs | Deduplicates against the committed cursor; `run_from` enables recovery when the application persists `last_event_id()` externally |

The official docs say event history is retained for one day. A persistent cursor store is useful for catch-up across restarts, but users must be able to choose a clean-start policy too.

## 4. Commands and argument parsing

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Register command names/descriptions | Validated local command registry |
| [x] Documented | Read and synchronize registered commands | `Bot::list_commands` reads GET `/commands`; `Bot::run` synchronizes the full registry with PUT before polling |
| [x] Documented | Prefix-command parsing | Longest matching prefix, case-insensitive name handling |
| [x] Documented | `args` and `argv` | Typed `CommandContext` argument access |
| [x] Documented | 100 command maximum per bot | `validate_command_list` enforces the current official 100-command cap before full-list sync |
| [x] Documented | Per-command permission/check callback | `Bot::command_with_check` runs an async allow/deny predicate before prefix or slash handlers |
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
| [x] Documented | Plain JSON interoperability | `ApiClient::api` / `Bot::api` provide authenticated relative-route access with origin and traversal checks |

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
| [x] Documented | Open DM/send DM | `Bot::open_dm` and `Bot::send_dm` use POST `/users/{user_id}/dm` and the returned channel ID |
| [x] Documented | Voice state lookup and move/disconnect members | `get_voice_states`, `move_member_to_voice` and `disconnect_member_from_voice` use the documented server voice routes |
| [x] Documented | Bot profile, avatar and banner | `edit_me`, `set_avatar`/`delete_avatar`, `set_banner`/`delete_banner`; raw image uploads are locally capped at the documented 8 MiB/10 MiB limits |
| [x] Documented | Set bot status and activity | `Bot::set_status` plus typed `Bot::set_presence(&Presence)`; `Activity::playing` formats the plain activity line |
| [x] SDK feature | Automatic activity rotation | `ActivityRotation` cycles validated activities; `Bot::rotate_presence` returns a cancellable `PresenceRotationHandle` |
| [x] SDK feature | Dynamic server-count presence | `Activity::overviewing_servers`, one-shot `refresh_server_count_presence`, and periodic `monitor_server_count_presence` only publish when the server count changes |
| [ ] Verify | Scheduled server events | Source has list/create; verify current public API schema |
| [ ] Verify | Thread list/create/edit/delete | Source has methods; verify endpoint availability |
| [ ] Verify | Emoji create/edit/delete/steal/message emoji lookup | Source has methods; verify endpoint availability |
| [ ] Verify | Fine-grained channel/category permission overrides | Source has methods; verify routes and payload schema |
| [ ] Verify | Invite create/delete event surface | Source event list has event names; verify routes/data |

All permission-dependent actions must return a useful structured 403 error. The library must not imply that a bot can act without the corresponding server role/permission.

## 8. Webhooks and voice

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] Documented | Event webhooks (inbound HTTPS POST to user's server) | Reusable signature verifier and typed envelope are provided; no HTTP server framework is imposed on library users |
| [ ] Verify | Bot-managed channel webhooks: create/list/send/delete | Validate source endpoints and permission requirements |
| [ ] Verify | Webhook wrapper `send` and `delete` | Include after API confirmation |
| [x] Documented | Voice presence, member move and disconnect | Typed `VoiceChannelState` plus REST lookup, move and disconnect methods; bots still cannot join audio calls |
| [ ] Defer | Bot joining calls and sending audio through LiveKit | Official docs currently state bots do not join calls; require explicit confirmation |
| [ ] Defer | E2EE voice key exchange and `VoiceCall` lifecycle | Security-sensitive and undocumented in the current public contract |

Do not treat voice presence management as equivalent to voice media streaming. They are separate capabilities.

## 9. Cogs, extensions and telemetry

| Status | Python feature | Rust target |
| --- | --- | --- |
| [x] SDK feature | Cog groups commands/listeners/buttons | `Cog` trait plus `Bot::add_cog`; compiled cogs register commands, listeners and buttons as an all-or-nothing transaction |
| [x] SDK feature | Setup function/extensions | `Cog::register(&mut Bot)` is the explicit Rust setup entry point |
| [~] SDK adaptation | Load/unload/reload command groups | `Bot::add_cog` loads statically compiled modules before running; live unload/reload is not exposed because the registries are immutable while the poller runs |
| [ ] Verify | File-system discovery of `.py` cogs | Rust cannot import source files dynamically; defer or design plugin ABI separately |
| [~] SDK adaptation | Error attribution to cog/extension | Cog registration errors include the cog name; runtime handler errors include command/event/custom-ID context but do not yet attach cog ownership |
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

