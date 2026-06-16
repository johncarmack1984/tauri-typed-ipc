# tauri-typed-ipc

Type-safe IPC for Tauri, built on [specta](https://github.com/specta-rs/specta) v2.
Sync commands by default.

Define your Rust/TypeScript IPC surface once -- procedures and events as a single
Rust trait -- and both sides stay in agreement at compile time: the wire is
identical to a raw `#[tauri::command]`, and the matching TypeScript client is
generated and drift-checked from the same definition, so a change on one side
that the other has not accounted for fails the build rather than the app.

The crate is `tauri-typed-ipc`; the examples here pull it in under the short
alias `ttipc` (`ttipc = { package = "tauri-typed-ipc" }` in `Cargo.toml`), though
the full `tauri_typed_ipc` path works just as well.

**Status:** `0.1.0`, built on specta `2.0.0-rc.25` (pinned exact -- specta v2
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
  runtime. Most IPC handlers are short, and a sync handler avoids the
  `Send`/`!Send` and main-thread constraints the async path carries.
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
