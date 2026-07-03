# tauri-typed-ipc

[![crates.io](https://img.shields.io/crates/v/tauri-typed-ipc.svg)](https://crates.io/crates/tauri-typed-ipc)
[![docs.rs](https://img.shields.io/docsrs/tauri-typed-ipc)](https://docs.rs/tauri-typed-ipc)
[![license](https://img.shields.io/crates/l/tauri-typed-ipc.svg)](#license)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/rust-secure-code/safety-dance/)

Type-safe IPC for Tauri, built on [specta](https://github.com/specta-rs/specta) v2.
Sync commands by default.

Define your Rust/TypeScript IPC surface once -- procedures and events as a single
Rust trait -- and both sides stay in agreement at compile time: the wire is
identical to a raw `#[tauri::command]`, and the matching TypeScript client is
generated and drift-checked from the same definition, so a change on one side
that the other hasn't accounted for fails the build rather than the app.

Compared to raw `invoke`, the call sites are typed end to end and the client can't silently drift from the Rust. Compared to [TauRPC](https://github.com/MatsDK/TauRPC) -- the closest existing tool -- procedures are sync by default (TauRPC's were async-only until [MatsDK/TauRPC#69](https://github.com/MatsDK/TauRPC/pull/69) added opt-in sync methods), the wire stays identical to a raw `#[tauri::command]` so a trait can be adopted one command at a time, and the TypeScript client is generated at build time with a drift `check` rather than at dev-server runtime.

The crate is `tauri-typed-ipc`; the examples here pull it in under the short
alias `ttipc` (`ttipc = { package = "tauri-typed-ipc", version = "0.1" }` in
`Cargo.toml`), though the full `tauri_typed_ipc` path works just as well.

**Status:** `0.1.3`, built on specta `2.0.0-rc.25` (pinned exact -- specta v2
is still a release candidate, so the dependency is pinned and bumped per
release). The surface below is exercised by the example app and the test
suite. See [ROADMAP.md](ROADMAP.md) for what's next.

## How it works

Define your IPC surface once as a Rust trait and implement it:

```rust
use ttipc::procedures;

#[procedures]
trait Greeter {
    fn greet(&self, name: String) -> String;
}

struct Backend;

impl Greeter for Backend {
    fn greet(&self, name: String) -> String {
        format!("Hello, {name}!")
    }
}
```

Mount every procedure on one handler. The wire is identical to a raw
`#[tauri::command]` -- same command name, same named-argument JSON, no
envelope -- so generated calls, hand-written `invoke`s, and raw commands are
interchangeable, and you can adopt it one command at a time:

```rust
tauri::Builder::default()
    .invoke_handler(ttipc::handler(Backend.into_procedures()))
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

Render the matching TypeScript client, and fail CI when it drifts:

```rust
ttipc::Bindings::new()
    .register::<GreeterProcedures>()
    .export_to("../src/bindings.ts")?;
```

```typescript
import { greeter } from "./bindings";
const hello = await greeter.greet("world"); // string
```

## What's here

- **Sync by default.** A procedure is a plain `fn`, dispatched inline on the
  main thread; mark one `async fn` and only that one is spawned on tauri's
  runtime. A sync handler puts no `Send` bound on its own logic (so it can hold
  `!Send` locals) and skips the executor hop -- though managed state still
  carries tauri's `Send + Sync` bound, and a slow sync handler blocks the UI
  thread, so long-running work opts into `async`.
- **Typed client.** A TypeScript client is generated from the trait, with a
  `check` mode that fails CI when the committed client drifts from the Rust.
- **Events**, both directions, from an enum.
- **Typed errors.** A `Result<_, E>` procedure rejects `E` on the wire, and
  the client types its `catch` against it.
- **Injection by type.** An `AppHandle` or `tauri::State` parameter is
  resolved by its type, never by its name.
- **Streaming.** A `Channel<T>` parameter streams values back to the caller.
- **Built on [specta](https://github.com/specta-rs/specta) v2** for the type
  machinery -- no vendored type system.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at
your option.
