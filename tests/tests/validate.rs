//! Runtime payload validation mounted on tauri's invoke pipeline (the
//! `validate` feature). A validating handler rejects a bad payload at the
//! boundary -- before the procedure runs -- and routes a good one exactly as
//! the plain handler does. Gated on the tests crate's `validate` feature.
#![cfg(feature = "validate")]

use serde_json::json;
use tauri::ipc::InvokeBody;
use tauri::test::get_ipc_response;
use ttipc::{Bindings, Validator, handler, handler_validated, procedures};
use ttipc_tests::{invoke_request, mock_webview};

// A self-contained set, so the test does not lean on fixture internals: one
// bare-primitive argument, the case that would slip past an empty schema
// without the inline-leaf transform.
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

fn validator() -> Validator {
    Validator::new()
        .register::<GreeterProcedures>()
        .expect("the validator builds from the descriptor")
}

#[test]
fn valid_payload_resolves_unchanged() {
    let (_app, webview) = mock_webview(handler_validated(Backend.into_procedures(), validator()));
    let response = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({ "name": "world" }))),
    )
    .expect("a well-typed payload resolves as usual");
    assert_eq!(
        response.deserialize::<String>().expect("a string response"),
        "Hello, world!"
    );
}

#[test]
fn invalid_payload_rejects_at_the_boundary() {
    // A number where a string is required. The rejection is the validation
    // error -- proof it fired before dispatch, not the serde message the
    // unvalidated handler would produce (asserted below).
    let (_app, webview) = mock_webview(handler_validated(Backend.into_procedures(), validator()));
    let err = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({ "name": 5 }))),
    )
    .expect_err("a wrong-typed argument is rejected");
    let message = err.as_str().expect("error is a string");
    assert!(
        message.starts_with("invalid payload for"),
        "expected a validation rejection, got: {message}"
    );
}

#[test]
fn without_validation_the_same_payload_reaches_serde() {
    // The contrast: the plain handler lets the bad payload through to serde,
    // which rejects it with a different (deeper, less specific) message. This
    // is what the boundary check exists to pre-empt.
    let (_app, webview) = mock_webview(handler(Backend.into_procedures()));
    let err = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({ "name": 5 }))),
    )
    .expect_err("serde also rejects, but later");
    let message = err.as_str().expect("error is a string");
    assert!(
        message.starts_with("invalid arguments:"),
        "expected the serde-path message, got: {message}"
    );
}

// A set with a container argument, to exercise the check over the same nested
// shapes on both ends -- the emitted client schema and the Rust handler.
#[procedures]
trait Calc {
    fn add(&self, values: Vec<u32>, label: String) -> u32;
}

struct Adder;

impl Calc for Adder {
    fn add(&self, values: Vec<u32>, _label: String) -> u32 {
        values.iter().sum()
    }
}

#[test]
fn validated_handler_rejects_a_bad_container_element() {
    // The Rust twin of the client's nested check: Vec<u32> with a string
    // element is rejected at the boundary, before dispatch.
    let validator = Validator::new()
        .register::<CalcProcedures>()
        .expect("the validator builds");
    let (_app, webview) = mock_webview(handler_validated(Adder.into_procedures(), validator));
    let err = get_ipc_response(
        &webview,
        invoke_request(
            "add",
            InvokeBody::Json(json!({ "values": ["x"], "label": "y" })),
        ),
    )
    .expect_err("a non-integer element is rejected");
    let message = err.as_str().expect("error is a string");
    assert!(
        message.starts_with("invalid payload for"),
        "expected a validation rejection, got: {message}"
    );
}

#[test]
fn validated_client_emits_the_contract_and_a_guard() {
    let client = Bindings::new()
        .register::<GreeterProcedures>()
        .register::<CalcProcedures>()
        .validate(true)
        .export()
        .expect("the validated client renders");

    // The self-contained validator and the per-command schemas, emitted once.
    assert!(
        client.contains("function __ttipcValidate("),
        "expected the validator:\n{client}"
    );
    assert!(
        client.contains("const __ttipcSchemas"),
        "expected the schemas:\n{client}"
    );
    // The inline-leaf fix reaches the client too: greet's string arg carries a
    // real schema, and add's Vec<u32> emits an array-of-integer schema. Match
    // the shape, not exact bytes -- the numeric bounds/keys a specta-jsonschema
    // version stamps on an integer vary (`format` vs `maximum`), and `type`
    // sorts last, so `"type":"X"}` is a version-stable anchor.
    assert!(
        client.contains(r#""name":{"type":"string"}"#),
        "expected greet's string arg schema:\n{client}"
    );
    assert!(
        client.contains(r#""type":"array"}"#),
        "expected add's values to be an array:\n{client}"
    );
    assert!(
        client.contains(r#""type":"integer"}"#),
        "expected add's element to be an integer:\n{client}"
    );
    // Each method checks its arguments before the (unchanged) invoke.
    assert!(
        client.contains(r#"__ttipcValidate("greet", { name });"#),
        "expected greet's guard:\n{client}"
    );
    assert!(
        client.contains(r#"__ttipcValidate("add", { values, label });"#),
        "expected add's guard:\n{client}"
    );
}

#[test]
fn client_without_validate_emits_no_check() {
    // Default off: the plain client, with no validator and no schemas -- so an
    // app that does not opt in pays nothing and its output is unchanged.
    let plain = Bindings::new()
        .register::<GreeterProcedures>()
        .export()
        .expect("the plain client renders");
    assert!(!plain.contains("__ttipcValidate"), "got:\n{plain}");
    assert!(!plain.contains("__ttipcSchemas"), "got:\n{plain}");
}
