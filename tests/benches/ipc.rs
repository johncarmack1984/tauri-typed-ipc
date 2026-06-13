//! End-to-end invoke cost through tauri's mock runtime: the same
//! function as a ttipc procedure vs a raw #[tauri::command], over
//! the identical pipeline (routing, payload deserialize, call,
//! response serialize, responder). The mock runtime skips the webview
//! and the process hop, so these numbers are the Rust-side cost only;
//! the delta between the two arms is what ttipc's routing adds.
//! `ttipc_state` is a procedure that takes injected managed state,
//! so it carries the one StateManager lookup the injection costs.
//! `ttipc_async` is an `async fn` procedure, so it carries the
//! spawn-and-resolve tax over the sync `ttipc_procedure`.
//! Fixtures: ../src/lib.rs.

use serde_json::json;
use tauri::ipc::InvokeBody;
use tauri::test::get_ipc_response;
use ttipc::handler;
use ttipc_tests::{
    App, BackupDispatch, CountedDispatch, GreeterDispatch, Hits, Service, Vault, invoke_request,
    mock_webview, mock_webview_managed,
};

#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

#[divan::bench]
fn raw_command(bencher: divan::Bencher) {
    let (_app, webview) = mock_webview(tauri::generate_handler![greet]);
    bencher
        .with_inputs(|| invoke_request("greet", InvokeBody::Json(json!({ "name": "world" }))))
        .bench_values(|request| get_ipc_response(&webview, request));
}

#[divan::bench]
fn ttipc_procedure(bencher: divan::Bencher) {
    let (_app, webview) = mock_webview(handler(App.into_procedures()));
    bencher
        .with_inputs(|| invoke_request("greet", InvokeBody::Json(json!({ "name": "world" }))))
        .bench_values(|request| get_ipc_response(&webview, request));
}

#[divan::bench]
fn ttipc_state(bencher: divan::Bencher) {
    let (_app, webview) = mock_webview_managed(Hits(7), handler(Service.into_procedures()));
    bencher
        .with_inputs(|| invoke_request("hits", InvokeBody::Json(json!({}))))
        .bench_values(|request| get_ipc_response(&webview, request));
}

#[divan::bench]
fn ttipc_async(bencher: divan::Bencher) {
    // The async path end-to-end: handler -> Dispatch::Async ->
    // async_runtime::spawn -> settle -> resolve, through the same mock
    // pipeline. The gap over `ttipc_procedure` is the spawn-and-resolve
    // tax, which is tauri's runtime, not ttipc's routing.
    let (_app, webview) = mock_webview(handler(Vault.into_procedures()));
    bencher
        .with_inputs(|| invoke_request("snapshot", InvokeBody::Json(json!({ "label": "x" }))))
        .bench_values(|request| get_ipc_response(&webview, request));
}

fn main() {
    divan::main();
}
