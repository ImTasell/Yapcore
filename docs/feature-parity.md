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

No feature is marked implemented yet. This is the initial porting checklist.

## 1. Configuration and lifecycle

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | Construct bot with explicit token | `Bot::builder().token(...)` |
| [ ] Documented | Read `YAPPER_BOT_TOKEN` | Builder helper with clear missing/invalid-token error |
| [ ] Documented | Configurable API base URL | `Config.api_base_url`, defaulting to official documented base |
| [ ] Documented | Prefix list, default `/` and `!` | `Config.prefixes` |
| [ ] Documented | Logging toggle | Integrate with optional `tracing`; no mandatory logger |
| [ ] Documented | Ignore bot/webhook messages by default | Configurable message filter |
| [ ] Documented | Start, stop and graceful shutdown | Async `start().await` plus cancellation/shutdown handle |
| [ ] Verify | Automatic version check | Optional, non-blocking and opt-out; decide whether it is desirable |
| [ ] Verify | Alternate API hostname fallback | Do not add until host equivalence and retry safety are confirmed |

## 2. HTTP client and errors

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | Auth header `Authorization: Bot <token>` | Central request middleware |
| [ ] Documented | JSON request/response handling | Typed Serde models plus raw JSON escape hatch |
| [ ] Documented | Error status and message | Structured `YapcoreError` |
| [ ] Verify | Error code and permission fields | Preserve if present in server response |
| [ ] Documented | 429 / 503 Retry-After handling | Shared retry policy; respect server-provided wait |
| [ ] Verify | Proxy retry/fallback behavior | Retry only when safe; never duplicate a message send |
| [ ] Documented | Configurable request timeout | Builder/config option with sane default |
| [ ] Documented | Multipart/file sending | Bounded streaming upload; verify current per-file/total limits |

Retry safety rules:
- GET can be retried for transient transport/server failures.
- Respect `Retry-After` for 429 and documented retryable 503 responses.
- Do not automatically replay non-idempotent POST requests unless the API supports a stable idempotency key/nonce and the same key is reused.
- Expose retry attempts and final status in diagnostics without leaking credentials.

## 3. Event transport and lifecycle

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | Long poll `GET /events?after=&wait=` | Primary event transport |
| [ ] Documented | Cursor-based ordered event delivery | Track monotonically increasing event ID |
| [ ] Documented | Up to 100 events per poll, up to 25-second wait | Honor documented server limits |
| [ ] Documented | One-day event retention | Explain restart/cursor implications |
| [ ] Documented | Optional `max_age` catch-up filter | Expose as polling configuration |
| [ ] Documented | Ready/error lifecycle callbacks | Typed lifecycle events |
| [ ] Documented | Event webhooks as a server-side delivery option | Not needed for normal outbound-only client; document as deployment mode |
| [ ] Verify | WebSocket gateway `wss://rt.yapper.website/bot` | Defer; current public guide does not document its protocol |
| [ ] Verify | Invite/channel/thread/category/message/server extra event kinds | Preserve unknown event payloads; add typed events only after endpoint confirmation |
| [ ] Verify | DM message event | Official docs say ordinary DMs are not delivered as bot events |
| [ ] Documented | Filter bot-authored messages | Optional dispatcher filter |
| [ ] Documented | Ignore duplicate/out-of-order event IDs | Cursor and deduplication in poll loop |

The official docs say event history is retained for one day. A persistent cursor store is useful for catch-up across restarts, but users must be able to choose a clean-start policy too.

## 4. Commands and argument parsing

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | Register command names/descriptions | Command registry |
| [ ] Documented | Slash command synchronization | Sync full command list before dispatch |
| [ ] Documented | Prefix-command parsing | Longest matching prefix, case-insensitive name handling |
| [ ] Documented | `args` and `argv` | Typed `CommandContext` argument access |
| [ ] Verify | 100 command maximum in current framework | Use current documented API limit; validate on registration |
| [ ] Documented | Per-command permission/check callback | Async check hook |
| [ ] Verify | Typed options: string/integer/number/boolean/user/channel/role | Typed option schema and parser |
| [ ] Verify | Choices and named `key=value` input | Parser compatibility tests |
| [ ] Verify | Usage/error replies on invalid arguments | Optional built-in validation response |
| [ ] Documented | Command event distinct from regular message | Separate event/interaction handling |
| [ ] Documented | Auto-acknowledge unanswered slash interactions | Track whether the response has already been sent; respect deadline |

Rust API should favor explicit typed command registration and async handlers. It does not need to imitate Python decorators.

## 5. Messages, models and interactions

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | Send message | `send_message` |
| [ ] Documented | Reply to message | `Message::reply` |
| [ ] Documented | Send embed(s) | Embed builder and Serde serialization |
| [ ] Documented | Edit/delete own message | Typed API methods |
| [ ] Documented | Fetch one/recent/forward messages | Pagination and time cursor |
| [ ] Documented | Bulk delete | Up to documented API limit |
| [ ] Documented | Pin/unpin | Typed API methods |
| [ ] Documented | Add/remove/list reactions | URL-encode emoji in route segment |
| [ ] Verify | Four-file multipart send and sticker IDs | Confirm current upload contract and limits |
| [ ] Documented | Button interactions: reply/ephemeral/update/ack | One-shot/deadline-aware interaction response |
| [ ] Documented | Interaction expiry and answer count limits | Track deadline/expiry locally, still treat server as authority |
| [ ] Verify | Select menu handlers | Python package supports selects; official page's interaction examples focus on buttons |
| [ ] Verify | Modal and text-input components | Present in Python source; confirm current server payload schema |
| [ ] Verify | Components V2 layouts | Present in Python source; confirm official schema before finalizing |
| [ ] Documented | Unknown message/event JSON fields | Keep forward-compatible raw fields where practical |

## 6. Embeds, buttons and component builders

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | Embed title/description/url/color/author/footer/fields/image/thumbnail/timestamp | `Embed` builder |
| [ ] Documented | Up to 5 embeds and 6000 aggregate characters | Validation helpers; server remains final authority |
| [ ] Documented | Button row and button styles | `Button` and `ActionRow` builders |
| [ ] Documented | Link buttons | Validate HTTPS URL |
| [ ] Documented | Disabled buttons and emoji | Builder fields |
| [ ] Verify | Select menu option builders | Include after schema confirmation |
| [ ] Verify | Modal/TextInput builders | Include after schema confirmation |
| [ ] Verify | Components V2: text, section, gallery, separator, file, container | Dedicated typed component enum once protocol is confirmed |
| [ ] Documented | Plain JSON interoperability | Accept raw `serde_json::Value` as escape hatch |

Do not expose both a huge builder API and dozens of raw JSON methods for the same behavior. The builder layer should serialize to stable API payload types.

## 7. Server and member management

| Status | Python feature | Rust target |
| --- | --- | --- |
| [ ] Documented | List/get servers | Server endpoints |
| [ ] Documented | List server/global channels and channel details | Channel endpoints and filters |
| [ ] Documented | List members with pagination / fetch one member | Member endpoints |
| [ ] Documented | Roles list/get/create/edit/delete/reorder | Role endpoints |
| [ ] Documented | Add/remove role | Role assignment endpoints |
| [ ] Documented | Get and set permissions/overrides | Documented permission endpoints first |
| [ ] Documented | Kick/ban/unban/list bans/timeout | Moderation endpoints |
| [ ] Documented | Edit nickname | Member endpoint |
| [ ] Documented | Create/edit/delete channel | Channel endpoints |
| [ ] Documented | Create/rename/delete category | Category endpoints |
| [ ] Documented | Open DM/send DM | User DM endpoints |
| [ ] Documented | Voice state lookup and move/disconnect members | Voice management endpoints |
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

