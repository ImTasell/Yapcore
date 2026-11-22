# Yapcore architecture proposal

**Status:** Proposed; no implementation has been committed.  
**Audit date:** 2026-11-21

## 1. Goals

Yapcore is a native Rust framework for building bots on Yapper. It aims to preserve the useful capabilities of the `our-websites/yapper.py` framework while using Rust-native APIs, async execution and a maintainable module structure.

Priorities:

1. Correctness against the official Yapper HTTP API.
2. Feature parity for documented, supported functionality.
3. Async I/O and predictable shutdown/reconnect behavior.
4. Strongly typed public models with a raw JSON escape hatch.
5. Useful errors and safe retry behavior.
6. A small, deliberate dependency set and modular optional features.

Non-goals for the first release:

- Reproducing Python decorators or Python's dynamic import behavior.
- Reproducing the upstream monolithic file structure.
- Porting the separate telemetry web server/panel.
- Implementing undocumented gateway or voice-media protocols before they are confirmed.

## 2. Protocol baseline

Use the current official documented base URL:

```text
https://www.yapper.website/api/bot/v1
```

Authenticate every bot API request with:

```http
Authorization: Bot <bot-token>
Content-Type: application/json
```

The bot token is secret. It must never appear in logs, debug output, panic messages, telemetry or examples committed to the repository.

The first transport is HTTPS REST plus event long polling:

```text
GET /events?after=<last_event_id>&wait=25
```

The current official guide documents up to 100 events per response, waits up to 25 seconds, and describes event history retained for one day. Persisting the event cursor should be an optional capability so applications can catch up after restarts; a clean-start mode should also be available.

The Python source includes a WebSocket gateway at `wss://rt.yapper.website/bot`, but the current public developer guide does not document its protocol. Keep it out of the stable baseline. If the platform confirms it, add it behind an optional feature using a maintained WebSocket crate.

## 3. Crate layout

Start as one library crate named `yapcore`, split into modules. A workspace is unnecessary until an independently versioned CLI, code generator or plugin host actually exists.

Suggested source layout:

```text
src/
  lib.rs
  bot.rs              # Bot builder, runtime and lifecycle
  config.rs           # token, API URL, prefixes, timeouts, feature options
  client/
    mod.rs
    http.rs            # Reqwest client, auth, request execution and retry policy
    routes.rs          # route construction and query encoding
  error.rs             # public error type and API error decoding
  ids.rs               # typed ID newtypes
  models/
    mod.rs
    user.rs
    server.rs
    member.rs
    channel.rs
    message.rs
    role.rs
    event.rs
  events/
    mod.rs
    poller.rs          # cursor-based long poll
    dispatch.rs        # event routing and handler registration
  commands/
    mod.rs
    registry.rs
    parser.rs
    options.rs
  interactions/
    mod.rs
    response.rs
    components.rs
    modal.rs           # add after schema verification
  builders/
    mod.rs
    embed.rs
    button.rs
    layout.rs          # Components V2, after schema verification
  cogs.rs              # runtime registration groups
  telemetry.rs         # optional, explicit opt-in only
  voice.rs             # voice presence management; no call media initially
```

Names can be adjusted when implementation begins, but boundaries should remain clear: HTTP should not know about commands, and command dispatch should not know how JSON is transmitted.

## 4. Public API direction

Use an async builder instead of trying to recreate Python decorators.

Illustrative shape (not a committed API contract):

```rust
use yapcore::{Bot, CommandContext, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let mut bot = Bot::builder()
        .token_from_env("YAPPER_BOT_TOKEN")
        .prefixes(["/", "!"])
        .build()?;

    bot.command("ping", "Check whether the bot is alive", |ctx: CommandContext| async move {
        ctx.reply("pong").await?;
        Ok(())
    })?;

    bot.on_message(|ctx| async move {
        if ctx.message().mentioned {
            ctx.reply(format!("hey {}", ctx.message().author_mention())).await?;
        }
        Ok(())
    });

    bot.start().await
}
```

Exact handler traits and lifetimes should be decided in a small compile-tested API spike before the public interface is frozen. Async closure ergonomics in Rust have trade-offs; do not hide them behind unsafe code or overengineered procedural macros.

A likely internal representation is an owned event/context and a boxed `Send + 'static` future returned by each registered handler. A simpler generic callback abstraction can be chosen if the prototype proves it ergonomic.

## 5. Core internal design

### Configuration

A validated `Config` should hold:

- Token (redacted in `Debug` output).
- Official API base URL, overridable for tests and compatible alternate deployments.
- Prefix list.
- Request/connect timeout.
- Retry budget and retry policy.
- Bot-message filtering option.
- Poll wait/cursor policy.
- Optional logger/tracing behavior.

Read environment variables only when the builder asks for them. Optional `.env` support can use `dotenvy` behind a feature or be left to the application; it must not silently load secrets from surprising parent directories.

### HTTP client

A cloneable API client should own one reusable `reqwest::Client` and shared immutable configuration.

Responsibilities:

- Add authentication and user-agent headers centrally.
- Serialize request models with Serde.
- Decode successful typed responses and error payloads.
- Preserve unknown fields where forward compatibility is valuable.
- Enforce bounded response/upload sizes.
- Encode path segments and query parameters safely.
- Apply method-aware retry rules.

Retry policy must not duplicate side effects. GET is normally safe to repeat. For POST, retry only when the API provides a documented idempotency mechanism and the same key is reused. Respect the server's `Retry-After` value for rate limits and retryable busy responses.

### Errors

Expose one public error type with structured categories, for example:

- Configuration/token error.
- Transport/timeout error.
- HTTP/API error: status, API message, optional API code, optional permission, optional retry-after.
- Serialization/deserialization error.
- Event/handler error.
- Shutdown/cancellation state.

Do not convert every error into a string. Applications need to branch on 401, 403, 404, 409, 429 and 503.

### Event poller

The poller should:

1. Start from either a configured cursor or a clean-start cursor.
2. Request events after the last successfully processed ID.
3. Process events in server order.
4. Advance the cursor only after the event has been handed to the dispatcher according to the documented semantics.
5. Keep the prior cursor when a request fails, so a retry does not silently skip data.
6. Back off on network/server errors while respecting Retry-After.
7. Stop promptly when cancellation is requested.

For stronger delivery guarantees, expose a cursor-store trait. The library should not require a database. An in-memory cursor is the default; applications can provide file/database persistence.

Unknown event types should not crash the runtime. Preserve the event name, common envelope and raw JSON payload so new server events can be handled before a new typed release.

### Command registry and dispatcher

The command registry should validate names/descriptions/options before syncing to `PUT /commands`.

The dispatcher should separate:
- Raw event decoding.
- Filtering (for example, ignore bot-authored messages).
- Prefix and command detection.
- Slash-command option parsing.
- Command checks.
- Handler execution.
- Interaction response tracking and acknowledgement.
- Generic event listeners.
- Error reporting.

Interactions must be treated as deadline-bound operations. The official guide requires a response within 3 seconds. The framework should prevent accidental double responses and offer an automatic acknowledgement policy, without hiding handler failures.

### Models and builders

Use typed structs/enums for common models and Serde. For API fields that are optional or may change, use `Option<T>` and preserve unknown data where needed.

Builders should produce the same JSON shapes expected by the API. They should not perform network operations. Keep validation close to builder finalization, but do not pretend client-side validation can replace server-side validation.

### Cogs

A Rust `Cog` should be an owned group of registered commands, event listeners and interaction handlers. The bot should track which registrations belong to each cog and remove them as a unit.

The Python package can load/reload arbitrary source files at runtime. Rust source code cannot be recompiled and safely imported as a normal runtime feature. First release should support explicit statically linked cog registration and unload/reload of registrations; a dynamic plugin ABI/WASM design is a separate opt-in project.

## 6. Dependency strategy

Keep dependencies small. Proposed baseline:

| Crate | Purpose | Notes |
| --- | --- | --- |
| `tokio` | Async runtime, timers, signals and synchronization | Enable only required features |
| `reqwest` | HTTPS client | Prefer rustls to avoid system OpenSSL requirements |
| `serde` + `serde_json` | Models and JSON | Core serialization |
| `thiserror` | Public typed errors | Keep error definitions ergonomic |
| `url` | URL and query construction | Avoid manual query concatenation |
| `bytes` | File/body buffers | Use bounded streaming where possible |
| `tracing` | Optional structured diagnostics | Do not log credentials or interaction tokens |

Possible later additions:

- `futures-util` and `tokio-tungstenite`: only if the gateway is officially confirmed.
- `regex`: only if regular-expression interaction matching is kept as a public feature.
- `dotenvy`: optional environment-file loading.
- `time` or `chrono`: only if timestamp ergonomics require it.
- `mime_guess`: optional file content-type inference.

Avoid adding `livekit`, crypto stacks, database frameworks or a plugin runtime to the default dependency graph. Voice media is not in the documented bot capability set.

## 7. Feature flags

Possible features once the baseline compiles:

- `dotenv`: optional .env file support.
- `gateway`: experimental WebSocket transport, only after protocol confirmation.
- `telemetry`: optional opt-in report client; disabled by default.
- `regex`: regex-based custom ID routing if this is wanted in the stable API.

Do not create feature flags speculatively for every module. A feature flag should correspond to a meaningful optional dependency or capability.

## 8. Testing strategy

### Unit tests

- Token and configuration validation.
- Command names, descriptions and option schema.
- Prefix parsing, longest-prefix matching and edge cases.
- Positional and named option parsing.
- Embed/button/component JSON serialization.
- Event decoding, unknown event preservation and typed ID wrappers.
- Error payload decoding and Retry-After parsing.
- Retry policy decisions (especially ensuring POST is not repeated unsafely).
- Cog registration/unregistration ownership.
- Interaction response state and deadline handling.

### Mock API contract tests

Use a local mock HTTP server; no live token in CI. Test:

- Correct API base and auth header.
- `GET /me` and `PUT /commands`.
- Message send/edit/delete and interaction callbacks.
- Server/channel/member/role/moderation routes.
- 401/403/404/409/429/503 responses.
- Malformed JSON and empty responses.
- Connection timeout and retry behavior.
- Multipart uploads within configured limits.
- Polling cursor behavior, duplicate IDs and event order.

### Live smoke tests

Run manually against a dedicated test bot after each major milestone. Keep tokens in local environment variables or repository secrets. Never commit tokens or screenshots/logs containing them.

### CI

At minimum:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Use a pinned stable Rust toolchain in CI. Cache Cargo registry/build artifacts. Do not require a live Yapper token for ordinary CI.

## 9. Implementation sequence

1. Resolve licensing and scope.
2. Create crate skeleton, public error type, configuration and API client.
3. Implement typed IDs, core models and `GET /me`.
4. Implement event polling, cursor handling and graceful shutdown.
5. Implement event dispatch and async handlers.
6. Implement command registry, sync and parser.
7. Implement message operations and interaction responses.
8. Implement embeds/buttons/components.
9. Implement documented server/member/channel/role/moderation routes.
10. Add cogs and optional verified endpoint families.
11. Consider telemetry, gateway and voice only after the support questions are answered.

The first release should be useful with a minimal dependency set: start a bot, register a command, receive events, reply to messages and shut down cleanly.
