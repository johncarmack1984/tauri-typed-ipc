//! Correctness gate for the ttipc arm: the bench is only meaningful
//! if both arms actually compute the same thing over the same wire.

use bench_common::{invoke_request, mock_webview};
use ttipc::handler;
use tauri_typed_ipc_bench::{App, AsyncApp, GreeterAsyncDispatch, GreeterDispatch};
use serde_json::json;
use tauri::ipc::InvokeBody;
use tauri::test::get_ipc_response;

#[test]
fn ttipc_greet_over_ipc() {
    let (_app, webview) = mock_webview(handler(App.into_procedures()));
    let response = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({ "name": "world" }))),
    )
    .expect("greet should resolve");
    assert_eq!(
        response.deserialize::<String>().expect("a string response"),
        "Hello, world!"
    );
}

#[test]
fn ttipc_async_greet_over_ipc() {
    let (_app, webview) = mock_webview(handler(AsyncApp.into_procedures()));
    let response = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({ "name": "world" }))),
    )
    .expect("async greet should resolve");
    assert_eq!(
        response.deserialize::<String>().expect("a string response"),
        "Hello, world!"
    );
}
