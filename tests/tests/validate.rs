//! Runtime payload validation mounted on tauri's invoke pipeline (the
//! `validate` feature). A validating handler rejects a bad payload at the
//! boundary -- before the procedure runs -- and routes a good one exactly as
//! the plain handler does. Gated on the tests crate's `validate` feature.
#![cfg(feature = "validate")]

use serde_json::json;
use tauri::ipc::InvokeBody;
use tauri::test::get_ipc_response;
use ttipc::{Validator, handler, handler_validated, procedures};
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
