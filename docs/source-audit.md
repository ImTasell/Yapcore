# Upstream source audit

**Audit date:** 2026-11-21  
**Target:** [our-websites/yapper.py](https://github.com/our-websites/yapper.py), default branch `main`  
**Purpose:** establish what Yapcore is porting before implementation.

This is a behavioral and protocol audit, not a line-by-line translation plan. The official Yapper developer documentation is the authority for supported API behavior. The Python repository is the authority for the behavior of the Python package itself. Where they disagree, the discrepancy is recorded rather than silently treated as supported API.

## 1. There are two relevant SDKs

Yapper's developer site distributes a small, first-party `yapper.py` starter SDK as a standalone file. Separately, the PyPI project and GitHub repository `our-websites/yapper.py` provide a larger, unofficial framework.

Yapcore's stated target is the latter: the full framework, not only the official starter example. The official developer documentation remains the protocol reference.

- Official bot documentation: <https://www.yapper.website/developers>
- Official API reference: <https://www.yapper.website/developers/api>
- Official OpenAPI 3.1 link: <https://www.yapper.website/openapi.json>
- Python framework repository: <https://github.com/our-websites/yapper.py>
- Python framework package metadata: <https://github.com/our-websites/yapper.py/blob/main/pyproject.toml>

## 2. Upstream repository layout

The current upstream repository contains:

| Path | Purpose |
| --- | --- |
| `src/yapper/main.py` | Main client, models, builders, command/event framework, HTTP, gateway and API methods |
| `src/yapper/telemetry.py` | Separate opt-in client telemetry extension |
| `server/` | A separate telemetry collection/panel service; not part of the published Python package |
| `tests/` | API surface, error, metadata, package, telemetry and telemetry-server tests |
| `README.md` | Installation, examples, feature inventory and support information |
| `pyproject.toml` | Package metadata and optional dependencies |

At the audited revision, `main.py` is approximately 2,816 lines / 135 KB. Most framework behavior lives in that one module. This is a useful behavior reference, but not a structure we should reproduce in Rust.

The package metadata declares Python 3.10+, `python-dotenv` as a required package dependency, and optional `voice` extras for `livekit` and `cryptography`. The main module catches a missing dotenv import, and the single-file SDK is documented as standard-library-only. These distribution modes should not be conflated.

## 3. Main framework behavior

### Client lifecycle and configuration

The main public entry point is `yapper.Bot`.

- Token comes from the constructor or `YAPPER_BOT_TOKEN`; it must begin with `yb_`.
- API base can be overridden through the constructor or `YAPPER_API_URL`.
- Prefixes default to `/` and `!`.
- Messages from bots/webhooks are ignored by default.
- Logging is enabled by default.
- The Python implementation is synchronous and blocking: `bot.run()` owns its event loop, and normal handlers are ordinary synchronous functions.

### HTTP layer

The implementation uses the Python standard-library `urllib` stack and JSON serialization. It adds the bot token with the `Authorization: Bot <token>` header.

The retry logic is more involved than a basic HTTP wrapper:

- Parses API error payloads into `YapperError` with HTTP status, retry-after, error code and permission fields.
- Waits and retries 429 responses and certain 503 busy responses when `Retry-After` is supplied and the configured wait budget allows it.
- Retries selected transient proxy failures, with method-aware safeguards.
- Avoids blindly repeating message POST requests unless the implementation knows a nonce can make the send safe.
- Switches to a fallback API hostname for a limited period after some connectivity failures.

The upstream default hosts do not exactly match the current official developer page. Python source currently names `https://bots.yapper.website/api/bot/v1` and `https://www.yapin.gg/api/bot/v1` as its primary/fallback pair. The current official developer page documents `https://www.yapper.website/api/bot/v1`. Yapcore should default to the documented host and make any alternative base URL explicit/configurable; do not silently inherit an old fallback.

### Event transport and dispatch

The Python framework has two event transports:

1. A custom WebSocket implementation for `wss://rt.yapper.website/bot`.
2. Long polling through `GET /events?after=<cursor>&wait=25`, used when the gateway is unavailable or disabled.

The WebSocket code implements the required RFC 6455 subset itself using sockets, TLS, frame masks, ping/pong, fragmentation and close codes. Rust should not port this hand-written protocol implementation; use a maintained WebSocket crate only if the gateway is confirmed as supported.

The current official developer page documents long polling and event webhooks, but does not document this WebSocket gateway protocol. Treat the gateway as an undocumented upstream feature until Yapper confirms its support and stability. Polling is the safe baseline.

The dispatcher converts raw JSON into message/command/interaction wrappers, checks bot-message filtering, resolves prefix commands, runs command checks and option parsing, invokes matching button/select/modal callbacks, emits general event listeners and routes failures to error listeners or logs.

### Commands and argument parsing

The command system supports:

- Prefix commands and server-registered slash commands.
- Up to 100 commands in the Python framework (`COMMAND_LIMIT = 100`); the current official platform guide documents a 50-command server cap. Yapcore validates against the stricter documented platform cap of 50.
- Names containing lowercase letters, digits, hyphens and underscores, 1–32 characters.
- Optional descriptions and up to 10 typed options.
- Option types: string, integer, number, boolean, user, channel and role.
- Choices, required/optional argument ordering, positional values and named `name=value` values.
- A final string option that can consume the remainder of the line.
- Per-command checks and automatic usage/error responses.

Command registration is synchronized to Yapper using `PUT /commands`. A slash command arrives as a distinct command event; it must be answered through the interaction response mechanism.

### Events, messages and interactions

Core wrappers include:

- `Data`: dictionary-like event/API data with attribute access.
- `Message`: `reply`, `say` and author mention helpers.
- `CommandMessage`: command response, ephemeral response, acknowledgement and modal helpers.
- `Interaction`: button/select/modal response methods, including reply, ephemeral, update, acknowledgement and modal opening.
- `Webhook`: send and delete helpers.

The package's event list is broader than the public event list in the official guide. The Python source includes DM message events, invite events, channel/thread/category lifecycle events, message updates/deletes, server add/remove and a test event. The official page says ordinary DMs are not delivered to bots as events. Treat those extra event kinds as undocumented until verified.

Button interactions have a hard response deadline: the official guide says answer within 3 seconds. The interaction token is temporary and sensitive. The framework includes auto-acknowledgement logic for command interactions that were not otherwise answered.

### Components and serialization

The Python API includes both plain-dictionary helpers and builder classes:

- Embeds and buttons: `Embed`, `Button`, `Row`, `row()`, `btn()`, `link_button()`.
- Select menus and modals: `Select`, `TextInput`, `Modal`.
- Components V2: `TextDisplay`, `Thumbnail`, `Section`, `MediaGallery`, `Separator`, `FileComponent`, `Container`, plus lowercase helpers.
- Recursive conversion of builder objects to plain JSON-compatible values.

These are mostly payload-building conveniences; they should remain separate from HTTP transport and event dispatch in Rust.

### Cogs and extensions

The Python framework supports:

- `Cog` subclasses with decorated commands, listeners and button handlers.
- `setup(bot)` functions in extension modules.
- Loading/unloading/reloading files and folders at runtime.
- Ownership tracking so a cog/extension can unregister only its own handlers.
- Command-list synchronization after cog changes.
- Optional checks and lifecycle hooks.

Rust cannot safely recreate Python's import-and-reload behavior as a direct equivalent. The idiomatic first version should provide registered cog instances/traits and deterministic load/unload of registrations. Dynamic code hot-reload should be considered a separate plugin-system project, not a baseline parity requirement.

### REST surface exposed by the Python Bot

The public methods cover:

- Messages: send, send embed/layout, edit, edit layout, fetch one/list/forward messages, delete, bulk delete, pin/unpin, react/unreact, list/remove reactions.
- Servers and members: list/get servers, member counts, channels, channel lookup, members, member lookup, roles, user/profile lookup.
- Roles and permissions: add/remove/create/edit/delete/reorder roles, permission lookup, channel/category overrides and everyone permissions.
- Moderation: kick, ban, unban, list bans, timeout, nickname.
- Channels and categories: list/create/edit/delete channels, list/create/delete categories.
- Threads: list/create/edit/delete.
- DMs: open/find a DM and send a DM.
- Voice management: inspect voice state, move/disconnect members, join/leave voice channels.
- Voice media connection: optional async LiveKit connection with E2EE key exchange.
- Webhooks: create/list own and channel webhooks, send and delete.
- Server scheduled events: list/create.
- Profile: set status, profile, avatar and banner.
- Raw escape hatch: `bot.api(method, path, body)`.

Some methods above use endpoints not enumerated in the currently published official developer page. See `feature-parity.md` for their status.

## 4. Telemetry is a separate optional feature

The framework's telemetry module is not imported by default. It can be enabled explicitly, and it honors opt-out environment variables. It sends a report on startup and then periodically (30 minutes by default), with bot identity/profile data, runtime/SDK versions, uptime, server/member counts, command metadata, cog/extension names, listener counts, button count and optional user-supplied values.

The telemetry receiver and panel live under upstream `server/`; `pyproject.toml` packages from `src/`, so the server is not part of the client library distribution.

Yapcore should not send telemetry by default. If ported, it must be an explicitly enabled optional module with an exact preview of the report and clear opt-out behavior.

## 5. Upstream tests and test gaps

The upstream test suite includes:

- Public API export checks.
- Error and test-event behavior.
- Package exports, version metadata and dotenv behavior.
- Telemetry payload/client behavior.
- Telemetry server behavior.

The visible tests do not amount to an end-to-end contract suite for every REST endpoint, the custom WebSocket implementation, every command-option edge case or voice media connection. Yapcore should introduce mock-server contract tests and focused unit tests instead of assuming upstream behavior is fully covered.

## 6. Audit discrepancies to resolve

| Finding | Why it matters | Action |
| --- | --- | --- |
| Current official API base differs from the Python module's configured primary host | A copied hostname may be stale or internal | Use official documented base by default; test any alias before supporting it |
| Official docs describe polling/webhooks, not the Python module's WebSocket gateway | Gateway behavior may be private, experimental or version-specific | Make polling the baseline; verify gateway support before implementation |
| Official docs state bots do not join voice calls; Python package has LiveKit voice connection code | This is a direct support/contract conflict | Defer `connect_voice` and `VoiceCall` pending confirmation |
| Official docs do not list several package event types and API method families | The Python package may contain undocumented endpoints/features | Label these as experimental/undocumented and verify against the live API |
| Python framework allows 100 local commands, while the current official developer guide documents a 50-command server cap | A full Python registry may be rejected by the server; the SDK-side cap is not the server contract | Yapcore enforces 50 before PUT /commands |
| `pyproject.toml` says package version 1.8.5, while `main.py` currently declares `__version__ = "1.8.0"` | Version/user-agent/update checks may report inconsistent values | Read version from one generated source in Yapcore |
| Main module header says Python 3.9+, package metadata says Python 3.10+ | Distribution metadata is stricter than the source header | Do not infer runtime compatibility from the header alone |
| README describes the single-file version as standard-library-only, package metadata depends on python-dotenv | Package and single-file distribution requirements differ | Make .env loading optional in Rust or document its dependency clearly |

## 7. Licensing alignment

The upstream Python repository declares **GPL-3.0-only** in its package metadata and includes a GPL-3.0 license. Yapcore's license was changed from MIT to GPL-3.0-only on 2026-11-21 to align with the upstream framework's declared license.

A direct translation/derivative port should retain GPL-3.0-only notices and comply with the license's distribution/source obligations. This is an engineering record, not legal advice. A clean-room implementation based only on independently documented API behavior is a different approach, but should be deliberately separated from copied code and reviewed appropriately.

This note is an engineering record, not legal advice. Preserve required copyright and license notices for any reused or translated upstream material.

## 8. Conclusions

1. Port the framework's behavior, not its monolithic Python structure.
2. Build the documented HTTPS REST API and long-poll event loop first.
3. Treat gateway, voice-media connection, extra events and undocumented endpoint families as verification items.
4. Preserve the broad command, event, component and server-management surface through idiomatic Rust abstractions.
5. Keep telemetry disabled unless a user explicitly enables it.
6. Keep the GPL-3.0-only license and preserve required upstream copyright/license notices for reused or translated material.

