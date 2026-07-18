//! Fixtures for the ttipc arm: the same `greet` the taurpc twin
//! implements, as a sync ttipc procedure and as an async one. The async
//! twin matches the taurpc arm's async resolvers for the apples-to-apples
//! pair; the sync twin is the sync-first arm.

use ttipc::procedures;

/// The procedure under test, identical across twins.
#[procedures]
pub trait Greeter {
    fn greet(&self, name: String) -> String;
}

/// The async twin of `greet`: same signature shape, but an `async fn`,
/// so it takes the spawn-and-resolve path taurpc's resolvers always do.
#[procedures]
pub trait GreeterAsync {
    async fn greet(&self, name: String) -> String;
}

/// Unit state standing in for a real procedure set owner.
pub struct App;

impl Greeter for App {
    fn greet(&self, name: String) -> String {
        format!("Hello, {name}!")
    }
}

/// Owner of the async set, kept separate from `App` so each owner
/// implements a single `Dispatch` trait and `into_procedures` stays
/// unambiguous.
pub struct AsyncApp;

impl GreeterAsync for AsyncApp {
    async fn greet(&self, name: String) -> String {
        format!("Hello, {name}!")
    }
}

/// Managed state for the async-state arm: the greeting prefix the
/// procedure reads through `tauri::State` inside the spawned future.
pub struct Prefix(pub &'static str);

/// The async twin that also takes managed state: the same workload plus
/// a `State<T>` parameter, so the arm measures the injection path's
/// spawn-side cost (the prelude clones the `Arc<StateManager>`, the
/// future resolves the state inside the spawn).
#[procedures]
pub trait GreeterAsyncState {
    async fn greet(&self, prefix: tauri::State<'_, Prefix>, name: String) -> String;
}

/// Owner of the async-state set (one owner per `Dispatch` trait).
pub struct AsyncStateApp;

impl GreeterAsyncState for AsyncStateApp {
    async fn greet(&self, prefix: tauri::State<'_, Prefix>, name: String) -> String {
        format!("{}, {name}!", prefix.0)
    }
}
