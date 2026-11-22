# Yapcore architecture

**Status:** Locked for the v0.1 implementation baseline  
**Decision date:** 2026-11-21  
**License:** GPL-3.0-only  
**Crate:** `yapcore`  
**Protocol authority:** the current public Yapper developer guide and API contract in [api-contract.md](api-contract.md)

This document is the implementation contract for the initial crate. It intentionally separates the stable, documented platform surface from unverified features in the Python framework. "Feature parity" means reproducing supported behavior, not copying Python's syntax or internals.

## 1. Product and engineering goals

Yapcore is a native Rust framework for building bots on Yapper. It should feel like an idiomatic Rust library, not a mechanical translation of Python.

Priorities, in order:

1. Correct HTTP/API behavior and safe handling of bot credentials.
2. Reliable event polling, cursor handling and graceful shutdown.
3. Ergonomic async command and event handlers.
4. Broad coverage of documented bot endpoints.
5. Maintainable typed models/builders with forward-compatible raw JSON access.
6. A small default dependency graph and reproducible builds.

The framework must not claim support for API behavior that Yapper does not document or that has not been verified against a dedicated test bot.

## 2. Locked platform scope

### Stable v0.1 platform contract

- REST base URL: `https://www.yapper.website/api/bot/v1`.
- Authentication header: `Authorization: Bot <token>`.
- JSON is the default request/response representation.
- Event transport: long polling through `GET /events?after=<cursor>&wait=25`.
- Poll response cap: 100 events; maximum long-poll wait: 25 seconds.
- Commands are synchronized as a complete list through `PUT /commands`.
- Message, embed, button-interaction, server, channel, member, role, moderation, DM-opening and voice-state APIs follow the public contract in `api-contract.md`.
- Use a real bot token from the Developer settings. Normal user-account automation is out of scope.

### Current implementation slice

The first working runtime is intentionally narrower than the long-term architecture:

- One bot instance owns one poll loop.
- Poll responses are dispatched sequentially in server order.
- `Bot::run` fetches the profile, synchronizes the full command registry with `PUT /commands`, then begins polling.
- Registered commands can be invoked by a configured text prefix or through a slash-command `command` event.
- `CommandContext` exposes `command`, `args`, `argv`, `reply` and `say`; slash invocations also expose `ephemeral` and `ack`.
- The REST client and `Bot` facade support fetching one message, recent and forward message history, editing/deleting, bulk delete (1–100 IDs), pin/unpin, and reaction list/add/remove.
- Slash callbacks use secret-wrapped tokens and are auto-acknowledged after two seconds if a handler has not answered, leaving margin before the platform's three-second deadline.
- Interaction response attempts are capped at ten per invocation and are not automatically retried.
- `Bot::on_button` registers exact custom IDs; `Bot::button` registers custom-ID prefixes. Handlers receive an interaction context with reply, ephemeral, update and ack operations.
- Unanswered slash/button handlers are acknowledged after two seconds, leaving margin before Yapper's three-second deadline.
- `Bot::on_event` registers ordered asynchronous listeners for any event type; listeners receive the full raw-preserving event envelope.
- A synthetic `ready` event is dispatched after profile retrieval and command synchronization.
- `Bot::run` starts from the newest cursor and stops through a clonable watch-channel shutdown handle.
- Poll transport failures use bounded exponential backoff; 429/503 are retried only when Yapper supplies `Retry-After`.
- Command, button and generic event-handler failures are logged and isolated from later listeners/polling.

The current dispatcher is sequential. Select-menu and modal-submit handlers, persistent cursor storage, bounded-concurrency dispatch and typed wrappers for individual server events remain future milestones.

### Explicitly deferred

| Feature | Decision |
| --- | --- |
| Python package's custom WebSocket gateway | Not in v0.1. The current public guide does not document its protocol. |
| Bot joining calls / LiveKit audio / E2EE media | Not supported by the public bot contract; do not port. |
| Select menus and modal submit handlers | Wait for official payload-schema verification. |
| Components V2 layouts | Wait for official payload-schema verification. |
| Message attachments, sticker IDs and nonce-based send deduplication | Wait for the raw API contract or live test-bot confirmation. |
| Threads, scheduled events, custom emoji CRUD, detailed permission overrides, bot-managed webhook CRUD | Implement only after each route is confirmed. |
| Dynamic source-code hot reload | Not a Rust library baseline. Use statically linked cog registration. |
| Telemetry | Not in v0.1. If added later, it is opt-in, previewable and off by default. |
| The Python telemetry receiver/panel | Separate service; out of scope for this client library. |

The optional signed event webhook is a server-side delivery configuration, not the same thing as a channel webhook used to post messages. Long polling remains the default v0.1 client transport.

## 3. Language, crate and compatibility policy

| Item | Decision |
| --- | --- |
| Package | One library crate named `yapcore` |
| Edition | Rust 2024 |
| MSRV | Rust 1.85 |
| Version | Start at 0.1.0 |
| License | GPL-3.0-only |
| Runtime | Tokio |
| HTTP | Reqwest with rustls |
| Serialization | Serde / serde_json |
| Error representation | Public structured error enum using thiserror |
| Logging | tracing facade; the application chooses its subscriber |
| Unsafe code | Forbidden in Yapcore's own source unless a documented, reviewed requirement appears (not expected) |
| Workspace | Not yet. Add one only when separately published crates or tools actually require it. |

Rust 1.85 is selected because it is the baseline for the 2024 edition and the chosen current Tokio/Reqwest release lines. The project must test both the declared MSRV and current stable Rust in CI.

Commit Cargo.lock for reproducible repository builds and CI. Dependency declarations use compatible version ranges; the lockfile records the exact tested graph. Keep the dependency graph lean and review new direct dependencies before adding them.

## 4. Crate structure

Use a single crate with clear module boundaries. This is the intended shape, not a requirement to create every file in the first commit.

~~~
src/
  lib.rs
  bot.rs                 # BotBuilder, Bot lifecycle, BotHandle
  config.rs              # validated configuration
  shutdown.rs            # watch-channel shutdown handle
  error.rs                # public errors and API error decoding
  ids.rs                  # typed string ID newtypes
  api/
    mod.rs
    client.rs             # reusable Reqwest client and auth
    request.rs            # request execution and bounded response reads
    retry.rs              # Retry-After and safe retry policy
    routes.rs             # route and query construction
    resources/
      mod.rs
      bot.rs
      commands.rs
      events.rs
      messages.rs
      users.rs
      servers.rs
      members.rs
      channels.rs
      roles.rs
      moderation.rs
      voice.rs
  models/
    mod.rs
    bot.rs
    user.rs
    server.rs
    member.rs
    channel.rs
    role.rs
    message.rs
    event.rs
    interaction.rs
  events/
    mod.rs
    poller.rs             # one cursor-based long-poll loop
    dispatcher.rs         # bounded handler dispatch
    registry.rs           # listeners and ownership metadata
    context.rs
  commands/
    mod.rs
    registry.rs           # command definitions and full-list sync
    parser.rs             # prefix and argument parsing
    context.rs
  interactions/
    mod.rs
    responder.rs          # callback response lifecycle/deadlines
    context.rs
  builders/
    mod.rs
    embed.rs
    button.rs
  cogs.rs                 # statically linked registration groups
~~~

Keep the layers independent:

- The API client knows HTTP, authentication, routes, serialization, response limits and retry policy. It knows nothing about commands or event callbacks.
- Models describe platform data and do not perform network requests.
- The poller fetches event envelopes and manages cursor state. It does not parse prefix commands.
- The dispatcher maps events to registered handlers. It does not construct URLs or know HTTP headers.
- Command and interaction contexts provide ergonomic helper methods over the API client.
- Builders produce JSON-compatible payload models. They do not send messages themselves.

## 5. Public API design

### v0.1 user-facing shape

The target API is a builder plus explicit registration methods. This is the intended ergonomic direction; exact signatures will be compile-tested during the foundation milestone.

~~~rust
use yapcore::{Bot, Result};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut bot = Bot::builder()
        .token_from_env("YAPPER_BOT_TOKEN")
        .prefixes(["/", "!"])
        .build()?;

    bot.command("ping", "Check whether the bot is alive", |ctx| async move {
        ctx.reply("pong").await?;
        Ok(())
    })?;

    bot.run().await
}
~~~

The library does not create or own the Tokio runtime and does not install process signal handlers. It runs inside the application's Tokio runtime. Applications choose current-thread or multi-thread execution and may clone the shutdown handle to cancel the bot from their own Ctrl+C or service-manager signal handling.

### Async handler representation

Do not add async-trait or a procedural macro for v0.1. Use generic registration functions and box futures only at the registry boundary:

~~~rust
type HandlerFuture =
    Pin<Box<dyn Future<Output = Result<(), HandlerError>> + Send + 'static>>;

type Handler =
    Arc<dyn Fn(CommandContext) -> HandlerFuture + Send + Sync + 'static>;
~~~

The public registration method is generic over a closure and its future, then stores a boxed future internally. This keeps call sites simple (`|ctx| async move { ... }`) without requiring users to write boxed futures.

Handlers receive owned contexts and owned event data. No handler future may borrow from the poller's temporary response buffer. Do not use unsafe lifetime tricks.

### Contexts

Use separate context types for different operations:

- `MessageContext`: message metadata and reply/say helpers.
- `CommandContext`: command name, arguments, parsed options when supported, reply and ephemeral response helpers.
- `InteractionContext`: interaction ID, user/message metadata and callback response helpers.
- `EventContext`: typed event plus the API client for general listeners.

Context methods are async where they perform network requests. Raw platform payload access remains available through a `serde_json::Value` accessor for forward compatibility.

### Bot lifecycle

`Bot::run()` performs this sequence:

1. Validate already-built configuration.
2. Fetch the bot profile with `GET /me`.
3. Synchronize the full registered command list with `PUT /commands`.
4. Determine the initial cursor according to the configured start policy.
5. Run exactly one long-poll loop.
6. Dispatch events with bounded concurrency.
7. Exit when the shutdown handle is cancelled; stop starting new work and drain in-flight handlers for a bounded grace period.

A `BotHandle` contains only the shutdown sender and any intentionally public control operations. It must not expose mutable internals.

## 6. Runtime and event-delivery model

### Poller

- Exactly one poller per bot instance.
- Use the documented 25-second maximum wait.
- Use a 40-second HTTP timeout so the request timeout exceeds the long-poll wait with network overhead.
- Use a cancellable `tokio::select!` around the long-poll request.
- Skip duplicate/out-of-order event IDs at or below the committed cursor.
- Never advance the cursor after a failed poll.
- Do not poll again until the current response batch has been admitted to the dispatcher.
- Respect the page-level `last` cursor only after all events in that response have been admitted/settled according to the documented response semantics. Keep the cursor logic isolated and contract-tested because the raw OpenAPI response schema still needs final verification.

### Startup cursor policy

Provide two explicit start modes:

- `Latest` (default): call `GET /events?after=0&wait=0` and begin after the returned latest cursor. This matches the Python framework's clean-start behavior and avoids replaying the bot's historical backlog unexpectedly.
- `Resume(cursor)`: begin after a caller-supplied saved cursor to catch up within the platform's one-day event retention.

Expose the current cursor so applications can persist it. Do not require SQLite, Redis or any database. A built-in persistent cursor store can be designed later if demand justifies it.

The library cannot promise exactly-once side effects. If an application persists a cursor and crashes during a batch, a later resume may replay events that were already handled but not covered by the last saved cursor. Handlers that perform external side effects should be idempotent.

### Dispatch and backpressure

- Use a bounded dispatcher; never create an unbounded task per event.
- Default maximum concurrent ordinary event handlers: 8, configurable down to 1 for strict sequential behavior.
- Invoke event handlers in received order, but when concurrency is greater than 1 their completion order is not guaranteed.
- Time-sensitive interaction callbacks use a priority lane and must be started immediately; the dispatcher must not let a slow general message listener starve them.
- Wait for all work from the current page to settle before accepting an unlimited amount of more work.
- Handler errors and panics are isolated, sent to the error-reporting hook and do not crash the entire poller by default.
- Provide a way to choose strict sequential event processing for bots whose logic depends on global event order.

Do not hold a Tokio mutex or standard-library lock across an `.await`. Clone the handler `Arc`s while holding a registry lock, release the lock, then invoke handlers.

### Unknown events

Decode the common event envelope first. Keep the raw event type and raw JSON payload. An unknown event is not a deserialization failure and must not stop the bot. Users can register a raw event listener before a typed model is added in a later release.

### Interaction deadline

The public API requires the initial interaction answer within 3 seconds. The interaction context must carry its creation time and track whether an initial answer has been sent. Provide reply/ephemeral/update/ack methods and reject obviously expired local attempts before sending the request. The server remains authoritative.

Do not automatically retry an interaction callback after an ambiguous transport failure: it may already have been accepted, and replaying a response can consume the interaction's answer allowance.

## 7. HTTP/API client design

### Client ownership

- Build one reusable `reqwest::Client` per bot.
- Clone the client handle; do not create a new HTTP client per request.
- Use the official base URL by default and allow a custom base URL for tests/compatible deployments.
- Construct routes and query parameters centrally. Encode dynamic path segments and never concatenate untrusted user-supplied strings into route paths.
- Set the `Authorization: Bot ...` header centrally.
- Set a Yapcore User-Agent with package version.
- Disable automatic redirects for authenticated API requests. A redirect must not move a bot token to an unexpected host.
- Do not store cookies.
- Do not log request bodies or sensitive headers by default.

### Response and request limits

- Enforce a configurable response-body size cap (initial default: 16 MiB) while reading the body, not only by trusting Content-Length.
- Enforce endpoint-specific upload limits before sending: avatar 8 MiB and banner 10 MiB per the current public guide.
- Do not implement undocumented message attachment upload limits until that feature is verified.
- Preserve unknown JSON fields when practical for platform forward compatibility.

### Error model

Expose a structured `Error` with distinct cases for:

- Invalid/missing configuration or token.
- Transport failure and timeout.
- API response failure, carrying status, message, optional API code, optional permission and parsed Retry-After.
- JSON encode/decode failure.
- Handler failure (reported separately from transport failure).
- Shutdown/cancellation.

Do not reduce API errors to strings. Keep the original HTTP status and a bounded representation of the response body so applications can react to 401, 403, 404, 409, 429 and 503.

### Retry rules

Retries are centralized and method-aware. Reqwest's own retry behavior is not relied on.

1. Retry transient transport failures and selected gateway/proxy 5xx responses only for safe/idempotent operations.
2. Retry 429 only when the response includes a valid Retry-After value.
3. Retry 503 only when the response includes Retry-After and the server contract says the operation was not performed.
4. Never automatically replay arbitrary POST requests.
5. PUT/DELETE retries are permitted only for operations whose API semantics are idempotent and whose request body is unchanged.
6. Parse both delta-seconds and HTTP-date Retry-After formats.
7. Use a bounded exponential backoff with jitter for retryable transport failures; never sleep longer than the configured retry budget.
8. Do not retry malformed JSON, local validation errors, 400, 401, 403, 404 or ambiguous interaction callbacks.

A client-generated nonce must not be introduced as an idempotency mechanism unless Yapper documents and confirms server-side support.

### Initial runtime defaults

These are starting defaults for v0.1, not hard-coded limits. Every operational value that affects reliability should be configurable:

| Setting | Default |
| --- | --- |
| Long-poll wait | 25 seconds (platform maximum) |
| HTTP request timeout | 40 seconds |
| Maximum response body | 16 MiB |
| Maximum concurrent ordinary event handlers | 8 |
| Shutdown drain grace period | 5 seconds |
| Transport retry attempts | 3 retries after the initial attempt |
| Transport backoff | 500 ms base, exponential, capped at 30 seconds, with jitter |
| Total automatic retry wait budget | 60 seconds |

Retry-After is a server instruction, not a hint to retry sooner. If the requested wait exceeds the remaining automatic retry budget, return the structured API error with its Retry-After value rather than retrying early or blocking forever. Cancellation must interrupt backoff and long-poll waits.

## 8. Security defaults

1. Require HTTPS for the API base URL. Permit plain HTTP only for loopback mock servers when an explicit test-only override is enabled.
2. Never place a bot token, interaction token or event-webhook secret in a URL or query string.
3. Mark the Authorization header value as sensitive and disable automatic redirects for authenticated API calls.
4. Store secrets in a redacting secret wrapper. Do not derive a Debug representation that reveals them.
5. Do not log request/response bodies by default. Avoid logging full Reqwest errors without sanitizing their URL, because an error can retain the request URL.
6. Treat interaction and event-webhook secrets with the same care as the bot token.
7. Keep telemetry absent from v0.1; never send usage data unless a future explicit opt-in feature is enabled.
8. Keep body-size limits and upload limits enforced before data is accepted or transmitted.

## 9. Models, IDs and serialization

- Use Serde structs for stable documented response shapes.
- Use enums for known discriminants, with an `Unknown(String)`/raw-value path where forward compatibility is required.
- Represent IDs as distinct transparent newtypes around `String` (for example `UserId`, `ServerId`, `ChannelId`, `MessageId`). This prevents mixing IDs without introducing a UUID parsing dependency.
- Keep timestamps as the platform's serialized ISO-8601 string in the first release. Add a typed time crate only if the public API needs date arithmetic or conversion.
- Use `Option<T>` for fields that are legitimately absent or nullable.
- Use `serde_json::Value` only at flexible extension boundaries and raw event payloads; prefer typed public fields elsewhere.
- Keep serialization builders separate from resource methods.

Do not eagerly deserialize fields that are not stable in the current official contract. Unknown data should be retained or ignored safely rather than breaking the event loop.

## 10. Commands and framework registration

### Commands

- Register commands locally before `run()`.
- Validate command name and description lengths locally using the published limits.
- Synchronize the entire command registry in one `PUT /commands` request.
- Enforce the published 100-command maximum.
- Implement prefix parsing with longest-prefix matching.
- Keep the slash-command dispatch path distinct from normal message-prefix parsing.
- Implement the documented option schema only after its exact OpenAPI payload is verified. The Python implementation's option parser is a useful compatibility target, not proof that every option type is currently supported by the server.
- No procedural macros are required.

### Cogs

A Rust cog is a statically linked group of command and event registrations with an ownership token. It can register/unregister its handlers as a unit. Runtime source loading and arbitrary code hot-reload are not part of v0.1. A WASM plugin ABI or dynamic library mechanism would be a separate design with its own safety and versioning constraints.

### Components

Implement the stable documented Embed and Button payload surface first. Keep builders as plain serializable data. Do not freeze Select, Modal or Components V2 payload types until the official schema/live test-bot verification is complete.

## 11. Dependency and feature policy

The exact dependency decisions and Cargo manifest are recorded in [dependencies.md](dependencies.md).

The default graph stays intentionally small:

- Tokio runtime primitives, macros, time and synchronization; the application owns runtime construction and process signals.
- Reqwest using rustls and JSON only; no native OpenSSL dependency.
- Serde and serde_json.
- thiserror for structured errors.
- secrecy for token redaction/zeroization support.
- tracing for library diagnostics.
- httpdate for parsing Retry-After headers.

Do not add async-trait, futures-util, bytes, url, uuid, chrono, a database, a CLI parser, WebSocket client, LiveKit, or a plugin runtime to the baseline. The standard library and existing dependencies cover the initial design.

No optional features are declared until there is a real, verified capability or optional dependency to gate. .env loading, telemetry, gateway support and experimental platform extensions can be evaluated separately.

## 12. Testing and quality gates

### Unit tests

- Config validation and redacted Debug output for secrets.
- Typed ID wrappers and Serde round-tripping.
- Command name/description validation.
- Longest-prefix matching and parser boundary cases.
- Embed/button serialization.
- Unknown event deserialization.
- API error-body decoding.
- Retry-After seconds/date parsing.
- Retry matrix proving unsafe POSTs are not replayed.
- Cursor behavior for empty, duplicate, ordered and full 100-event pages.
- Interaction response deadline/state behavior.
- Dispatcher bounded concurrency, priority lane and cancellation.
- Cog registration ownership/unregistration.

### HTTP contract tests

Use an isolated local mock HTTP server (Wiremock in dev-dependencies is the current choice). No live bot token is required in CI.

Test:
- Exact default base URL and `Authorization` header.
- Redirects do not forward credentials.
- `GET /me`, `PUT /commands`, `GET /events`.
- Message send/edit/delete and interaction callbacks.
- Documented server/channel/member/role/moderation routes.
- Empty success bodies and malformed JSON.
- 400/401/403/404/409/429/503 behavior.
- Retry-After integer/date values.
- Request timeout, response-size limits and cancellation.
- Multipart/raw avatar/banner upload constraints once upload code is added.

### Live smoke tests

Use a dedicated test bot manually. Store credentials in environment variables or GitHub Actions secrets. Live tests are opt-in and never run on ordinary pull requests. Never put a token in code, issue comments, logs or artifacts.

### CI commands

~~~
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
~~~

Also test the declared MSRV (1.85) separately from current stable. Cache the Cargo registry and build outputs. Keep CI free of live platform credentials.

## 13. Implementation sequence

1. **M0 — Decisions (complete):** GPL-3.0-only, source audit, feature inventory, API contract and architecture decisions.
2. **M1 — Crate foundation:** Cargo manifest, lint policy, public Error, Config, secret token wrapper, typed IDs and API client.
3. **M2 — First live API path:** GET /me, command sync and a mock-tested request layer.
4. **M3 — Polling runtime:** startup cursor policy, long polling, shutdown, response/error handling and bounded dispatcher.
5. **M4 — First usable bot:** prefix command parser, command context, send/reply and live /ping smoke test.
6. **M5 — Interactions:** button event model and callback responder with deadline tracking.
7. **M6 — Main documented resources:** servers, channels, members, roles, moderation, DMs and voice-state management.
8. **M7 — Builders and cogs:** Embed/Button builders, registration ownership and documented command options.
9. **M8 — Verified extended surface:** only add each undocumented endpoint family after schema/live contract confirmation.
10. **M9 — Release quality:** public docs, examples, compatibility tests, CI, crate metadata and versioning.

A useful v0.1 is not "thousands of lines". It is the smallest complete path that can authenticate, register a command, receive an event, reply, handle shutdown and pass deterministic mock-server tests. Expand from that working spine without pre-emptively implementing speculative features.
