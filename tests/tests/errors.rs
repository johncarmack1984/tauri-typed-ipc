//! The Error derive: each variant serializes to a discriminated-union
//! member `{ type, message }` -- Display-based, so non-Serialize sources
//! work too. Fixtures: ../src/lib.rs.

use ttipc_tests::DeskError;

#[test]
fn serializes_as_discriminated_union() {
    assert_eq!(
        serde_json::to_string(&DeskError::OutOfRange(99)).expect("serialize"),
        r#"{"type":"outOfRange","message":"channel 99 is out of range"}"#,
    );
    assert_eq!(
        serde_json::to_string(&DeskError::Locked).expect("serialize"),
        r#"{"type":"locked","message":"the desk is locked"}"#,
    );
}
