//! Fixtures for the ttipc arm: the same `greet` the taurpc twin
//! implements, as a sync ttipc procedure.

use ttipc::procedures;

/// The procedure under test, identical across twins.
#[procedures]
pub trait Greeter {
    fn greet(&self, name: String) -> String;
}

/// Unit state standing in for a real procedure set owner.
pub struct App;

impl Greeter for App {
    fn greet(&self, name: String) -> String {
        format!("Hello, {name}!")
    }
}
