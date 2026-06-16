//! Trait-based, type-safe Tauri IPC, built on [specta](https://specta.dev)
//! v2. Sync by default.
//!
//! Define your IPC surface once as a Rust trait. [`macro@procedures`]
//! flattens it into a dispatch core and a descriptor the TypeScript client
//! is rendered from; [`handler`] mounts that on tauri's invoke pipeline.
//! The wire is identical to a raw `#[tauri::command]` -- same command name,
//! same named-argument JSON, no envelope -- so generated calls,
//! hand-written `invoke`s, and raw commands are interchangeable, and a
//! trait can be adopted one command at a time through
//! [`handler_with_fallback`].
//!
//! A procedure is a plain `fn` by default, dispatched inline on the main
//! thread; mark one `async fn` and only that one is spawned on tauri's
//! runtime. Sync-first is the founding choice: a sync handler carries no
//! `Send` bound on its own logic and skips the executor hop, where TauRPC
//! makes every procedure `async`. (A sync handler still runs on the UI
//! thread, so long-running work opts into `async`.)
//!
//! # Quickstart
//!
//! Define the surface and implement it:
//!
//! ```
//! use tauri_typed_ipc::procedures;
//!
//! #[procedures]
//! trait Greeter {
//!     fn greet(&self, name: String) -> String;
//! }
//!
//! struct Backend;
//!
//! impl Greeter for Backend {
//!     fn greet(&self, name: String) -> String {
//!         format!("Hello, {name}!")
//!     }
//! }
//!
//! // `#[procedures]` adds `into_procedures`, which type-erases the trait
//! // for the handler:
//! let procedures = Backend.into_procedures();
//! assert_eq!(procedures.names(), &["greet"]);
//! ```
//!
//! Mount every procedure on one handler:
//!
//! ```ignore
//! tauri::Builder::default()
//!     .invoke_handler(tauri_typed_ipc::handler(Backend.into_procedures()))
//!     .run(tauri::generate_context!())
//!     .expect("error while running tauri application");
//! ```
//!
//! With the `export` feature, render the matching client and guard it
//! against drift:
//!
//! ```ignore
//! tauri_typed_ipc::Bindings::new()
//!     .register::<GreeterProcedures>()
//!     .export_to("../src/bindings.ts")?;
//! ```
//!
//! ```typescript
//! import { greeter } from "./bindings";
//! const hello = await greeter.greet("world"); // Promise<string>
//! ```
//!
//! # What's here
//!
//! - **Procedures** -- [`macro@procedures`] on a trait: a sync `fn`
//!   dispatches inline, an `async fn` is spawned on tauri's runtime.
//! - **Typed client** -- with the `export` feature, `Bindings` renders a
//!   TypeScript client from the generated [`ProcedureSet`] descriptor, and
//!   its `check` fails CI when the committed client drifts.
//! - **Events**, both directions -- [`macro@Event`] on an enum emits typed
//!   payloads and generates the matching listeners.
//! - **Typed errors** -- [`macro@Error`]: a `Result<_, E>` procedure
//!   rejects `E` on the wire, and the client types its `catch` against it.
//! - **Injection by type** -- an `AppHandle` or [`tauri::State`] parameter
//!   is resolved from the [`Context`] by its type, never by its name.
//! - **Streaming** -- a [`Channel<T>`](Channel) parameter streams values
//!   back to the caller, on top of the value the procedure returns.
//! - **Composition** -- [`Procedures::merge`] mounts several traits on one
//!   handler.

mod bindings;
mod channel;
mod context;
mod handler;

#[cfg(feature = "export")]
pub use bindings::{Bindings, BindingsError, Layout, MethodCase};
pub use bindings::{ErrorSet, EventSet, ProcedureSet};
// Descriptor payloads carried from the derives to the bindings generator. Named
// in generated code (`ProcedureSet::procedures` etc.), never built by hand.
#[doc(hidden)]
pub use bindings::{ErrorType, EventType, ProcedureType};
pub use channel::Channel;
pub use context::Context;
pub use handler::{Procedures, handler, handler_with_fallback};
// The dispatch result types: returned by generated `dispatch` and consumed by
// `handler`. Plumbing, not part of the hand-written surface.
#[doc(hidden)]
pub use handler::{Dispatch, Outcome};
pub use tauri_typed_ipc_macros::{Error, Event, procedures};

/// Error returned by a generated `dispatch` when a call cannot reach
/// or leave the procedure. Wire-level failures only -- a procedure's
/// own failures belong in its return type.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// No procedure with the requested name exists in this set.
    #[error("unknown procedure: {0}")]
    UnknownProcedure(String),
    /// The arguments failed to deserialize into the procedure's
    /// parameter types.
    #[error("invalid arguments: {0}")]
    Deserialize(#[source] serde_json::Error),
    /// The procedure's return value failed to serialize.
    #[error("response serialization failed: {0}")]
    Serialize(#[source] serde_json::Error),
    /// The invoke arrived with a raw byte body; procedures take JSON
    /// named arguments.
    #[error("procedures take JSON arguments, not a raw body")]
    RawBody,
    /// A parameter is injected by type, but the call's [`Context`]
    /// offered no value of that type.
    #[error("missing injectable value: {0}")]
    MissingInjection(&'static str),
    /// A `State<T>` parameter was requested, but no value of that type
    /// is managed -- the app never called `.manage()` for it.
    #[error("state not managed: {0} -- did the app call `.manage()` for it?")]
    MissingState(&'static str),
    /// A `Channel<T>` parameter was requested, but the dispatch
    /// [`Context`] carried no channel factory. The handler always
    /// installs one from the webview, so this arises only when
    /// dispatching against a bare [`Context`] (no [`with_channels`]).
    ///
    /// [`with_channels`]: Context::with_channels
    #[error("no channel factory: {0}")]
    MissingChannel(&'static str),
}

// Referenced by macro-generated code. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use serde;
    pub use serde_json;
    pub use specta;
    pub use tauri;
}
