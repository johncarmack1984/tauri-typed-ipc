//! Fixtures for the taurpc arm: the same `greet` the tauri-typed-ipc twin
//! implements, in taurpc's native shape (async resolvers on a Clone impl).
//! The pinned 0.5.2 release is async-only -- the design difference under
//! measurement. Upstream merged opt-in sync methods in MatsDK/TauRPC#69;
//! once that ships in a release, this arm can grow a sync twin.

/// The procedure under test, identical across twins.
#[taurpc::procedures]
pub trait Greeter {
    async fn greet(name: String) -> String;
}

/// Unit state standing in for a real procedure set owner.
#[derive(Clone)]
pub struct GreeterImpl;

#[taurpc::resolvers]
impl Greeter for GreeterImpl {
    async fn greet(self, name: String) -> String {
        format!("Hello, {name}!")
    }
}
