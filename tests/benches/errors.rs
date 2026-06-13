//! Microbenchmark: serializing a derived error to its `{ type, message }`
//! wire form. Fixtures: ../src/lib.rs.

use ttipc_tests::DeskError;

fn main() {
    divan::main();
}

#[divan::bench]
fn serialize() -> String {
    serde_json::to_string(&DeskError::OutOfRange(99)).expect("serialize")
}
