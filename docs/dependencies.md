# Yapcore dependency decisions

**Audit date:** 2026-11-21  
**Target:** Yapcore v0.1  
**MSRV:** Rust 1.85  
**Edition:** 2024  
**License:** GPL-3.0-only

The goal is a small, boring, well-supported dependency graph. The current released versions below were checked against docs.rs/crates.io records on the audit date. Cargo.toml should use compatible version requirements; Cargo.lock pins the exact graph tested in this repository.

## 1. Core dependencies

| Crate | Current release checked | Cargo requirement | Used for | Decision |
| --- | --- | --- | --- | --- |
| tokio | 1.54.1 | `1.54` | Runtime primitives, select macro, timers and synchronization | Required |
| reqwest | 0.13.5 | `0.13.5` | HTTPS REST client | Required |
| serde | 1.0.229 | `1.0` | Typed serialization/deserialization | Required |
| serde_json | 1.0.154 | `1.0` | JSON bodies and flexible raw event payloads | Required |
| thiserror | 2.0.18 | `2.0` | Public structured errors | Required |
| secrecy | 0.10.3 | `0.10` | Redacted secret wrapper and zeroization support | Required |
| tracing | 0.1.44 | `0.1` | Structured diagnostics/instrumentation | Required |
| httpdate | 1.0.3 | `1.0` | Parse the HTTP-date form of Retry-After | Required |
| fastrand | 2.5.0 | `2.5` | Non-cryptographic jitter for retry backoff only | Required |

### Tokio features

Enable only:

- `macros`: `tokio::select!` in the library and `#[tokio::test]` in tests.
- `rt`: task spawning and runtime-aware utilities used by the library.
- `time`: request/backoff/shutdown deadlines.
- `sync`: watch and bounded mpsc channels.

Do not enable `full`. The library uses Reqwest for network I/O and does not create a runtime. It does not need Tokio's raw TCP/UDP, filesystem, process, io-util, multi-thread runtime or signal features. An application that uses `#[tokio::main]` with the default multi-thread flavor should enable `rt-multi-thread` in its own Tokio dependency; Ctrl+C handling likewise remains the application's choice.

### Reqwest features

Use:

~~~toml
reqwest = {
    version = "0.13.5",
    default-features = false,
    features = ["rustls", "json"]
}
~~~

Rationale:

- rustls avoids a mandatory OpenSSL/native-TLS build dependency on Linux.
- json enables typed request and response serialization.
- Do not enable multipart until message-attachment support is confirmed.
- Do not enable compression, cookies, HTTP/3, SOCKS or blocking APIs for v0.1.
- Disable automatic redirects for authenticated bot API requests so the token is not forwarded to a different host.

Reqwest 0.13.5 declares Rust 1.85.0 as its minimum supported compiler. Tokio 1.54.0 raised its MSRV to 1.85; the current 1.54.1 patch is the selected runtime line. Therefore the crate's declared MSRV is Rust 1.85.

### Secret handling

Wrap the bot token using secrecy's SecretString/SecretBox API. Expose it only at the point where the Authorization header is built. Never derive a Debug representation that reveals the token. Do not serialize tokens, put them in URLs, or include them in tracing spans. The interaction callback token and event webhook secret receive the same treatment.

Secrecy's zeroization support reduces accidental retention but is not a promise that all copies inside HTTP libraries, operating-system buffers or allocator memory can be wiped.

### Retry utilities

- httpdate parses both Retry-After delta-seconds and HTTP-date values.
- fastrand provides non-cryptographic jitter for transport backoff only.
- Never use fastrand for bot tokens, interaction tokens, webhook secrets or security-sensitive identifiers.
- No retry framework is needed. The policy is short, method-aware and should stay in Yapcore's own code with explicit unit tests.

## 2. Development/test-only dependency

| Crate | Requirement | Used for | Normal user build? |
| --- | --- | --- | --- |
| wiremock | `0.6.5` | Isolated mock HTTP API server for contract tests | No; dev-dependency only |

Wiremock supports Tokio and isolated per-test mock servers. Its dependency graph is larger than the production graph, but it is compiled only for tests and gives deterministic HTTP contract coverage without a real Yapper token.

## 3. Cargo.toml baseline

This is the intended initial package manifest. The implementation commit may add metadata fields (description, repository, keywords, categories) but must not silently widen the runtime dependency set.

~~~toml
[package]
name = "yapcore"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
license = "GPL-3.0-only"
description = "An async Rust framework for building Yapper bots"
repository = "https://github.com/ImTasell/Yapcore"

[lib]
name = "yapcore"
path = "src/lib.rs"

[features]
default = []

[dependencies]
fastrand = "2.5"
httpdate = "1.0"
reqwest = { version = "0.13.5", default-features = false, features = ["rustls", "json"] }
secrecy = "0.10"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
tokio = { version = "1.54", features = ["macros", "rt", "sync", "time"] }
tracing = "0.1"

[dev-dependencies]
wiremock = "0.6.5"
~~~

Do not commit an artificially exact Cargo.lock generated from a different Rust target. Generate and commit the lockfile when the first Cargo foundation is created and run the lockfile tests in CI.

## 4. Deliberately not included

| Crate | Why excluded now | Reconsider when |
| --- | --- | --- |
| async-trait | Generic registration methods plus boxed std futures solve our handler storage needs; native async traits are enough for static-dispatch internals | A genuinely object-safe async extension trait is needed |
| futures / futures-util | Standard Future, Pin/Box, Tokio JoinSet and channels cover the initial code | Stream combinators prove materially simpler or more correct |
| tokio-util | A Tokio watch channel handles the single bot shutdown signal | A standard CancellationToken hierarchy becomes necessary |
| bytes | Vec<u8> is enough for small documented avatar/banner uploads | Streaming/zero-copy request bodies are introduced |
| url | Reqwest re-exports URL handling; use its Url type and query builder | A public URL abstraction needs an independent direct dependency |
| uuid | IDs are transparent string newtypes; strict UUID parsing is not needed for type separation | Protocol validation proves useful and accepted IDs are all canonical UUIDs |
| chrono / time | Keep ISO timestamps as strings; avoid pulling in a date/time ecosystem prematurely | API consumers need arithmetic, parsing or timezone conversions |
| rand | A full general-purpose RNG is unnecessary for retry jitter | A non-security random feature becomes a first-class product capability |
| dotenvy / dotenv-ng | Avoid implicit filesystem/environment mutation in the core library; use process environment variables | A documented, opt-in dotenv feature is requested |
| tracing-subscriber | Libraries should not choose a global subscriber or formatting policy | An example application needs a logger |
| clap | Yapcore is a library, not a CLI | A separate CLI is designed |
| sqlx / rusqlite / redis clients | Persistence belongs to the application; initial cursor can be passed in/out | Built-in storage is explicitly requested |
| tokio-tungstenite | The Python gateway is not in the public contract | Yapper documents/supports the gateway protocol |
| livekit / cryptography | Public bot docs state bots do not join voice calls | Yapper adds official bot audio support |
| wasm plugin runtimes / libloading | Dynamic plugin execution is outside v0.1 and has security/versioning implications | A separate plugin architecture is designed |

## 5. Dependency policy

1. Prefer stable, widely used crates with permissive licenses compatible with GPL-3.0-only.
2. Keep runtime dependencies separate from dev-dependencies.
3. Do not add a crate just to avoid writing a short, well-tested helper.
4. Do not implement cryptographic primitives ourselves.
5. Use Rust's standard library for simple IDs, timestamp strings, ownership and future boxing.
6. Re-check crate MSRV before adopting a dependency that raises Yapcore's declared MSRV.
7. Enable Cargo features explicitly; avoid broad `full` or `default` features when they pull in capabilities we do not use.
8. Review the dependency tree with `cargo tree -e features` before releases.
9. Use `cargo deny` or an equivalent CI check for license/security policy after the initial crate compiles.
10. Update the dependency audit whenever a new direct runtime dependency is added.

## 6. Sources

- Tokio 1.54.1 release and MSRV change: <https://github.com/tokio-rs/tokio/releases>
- Tokio 1.54.1 documentation: <https://docs.rs/tokio/1.54.1/tokio/>
- Reqwest 0.13.5 crate metadata and feature list: <https://docs.rs/crate/reqwest/0.13.5>
- Reqwest 0.13.5 source manifest: <https://docs.rs/crate/reqwest/0.13.5/source/Cargo.toml.orig>
- Serde JSON 1.0.154: <https://docs.rs/crate/serde_json/1.0.154>
- thiserror 2.0.18: <https://docs.rs/crate/thiserror/2.0.18>
- secrecy 0.10.3: <https://docs.rs/crate/secrecy/0.10.3>
- tracing 0.1.44: <https://docs.rs/crate/tracing/0.1.44>
- httpdate 1.0.3: <https://docs.rs/crate/httpdate/1.0.3>
- fastrand 2.5.0: <https://docs.rs/crate/fastrand/2.5.0>
- wiremock 0.6.5: <https://docs.rs/crate/wiremock/0.6.5>
