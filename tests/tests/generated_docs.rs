//! The code ttipc's macros generate must satisfy a consumer that sets
//! `#![deny(missing_docs)]` -- every generated public item carries a doc.
//! This whole test binary opts into that lint, so a regression (an
//! undocumented generated item from `#[procedures]`, `#[derive(Event)]`,
//! or `#[derive(Error)]`) fails to compile here rather than silently
//! breaking a strict downstream crate.
#![deny(missing_docs)]

use ttipc::procedures;

/// A procedure trait: its generated dispatch trait, `dispatch` method,
/// `into_procedures`, and `{Trait}Procedures` descriptor must all be
/// documented.
#[procedures]
pub trait Probe {
    /// Returns a fixed byte.
    fn ping(&self) -> u8;
}

/// Owner of the [`Probe`] set.
pub struct Owner;

impl Probe for Owner {
    fn ping(&self) -> u8 {
        1
    }
}

/// An event enum: its generated emit and listener glue must be documented.
#[derive(ttipc::Event)]
pub enum Signal {
    /// A documented variant.
    Fired {
        /// A documented field.
        at: u32,
    },
}

/// A wire error enum: its generated serialization and `ErrorSet` glue must
/// be documented.
#[derive(Debug, thiserror::Error, ttipc::Error)]
pub enum Fault {
    /// A documented variant.
    #[error("a fault")]
    Bad,
}

#[test]
fn generated_items_are_documented() {
    // The coverage is this binary compiling under deny(missing_docs); the
    // assertion just exercises a generated path so it is not dead code.
    assert_eq!(Owner.into_procedures().names(), &["ping"]);
}
