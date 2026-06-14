//! Fixtures for the taurpc arm: the same `greet` the tauri-typed-ipc twin
//! implements, in taurpc's native shape (async-only resolvers on a
//! Clone impl -- there is no sync option, which is the design
//! difference under measurement).

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
