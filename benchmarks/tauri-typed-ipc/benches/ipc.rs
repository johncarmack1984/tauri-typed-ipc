//! ttipc arm: full invoke pipeline through the mock runtime.
//!
//! Three arms in one binary: raw_command (the #[tauri::command] control),
//! ttipc_procedure (the sync-first path), and ttipc_async_procedure (the
//! async path). taurpc's resolvers are async-only, so ttipc_async_procedure
//! vs taurpc is the apples-to-apples pair; ttipc_procedure is the sync-first
//! arm with no taurpc equivalent.
//!
//! Compare against ../taurpc/benches/ipc.rs run on the same machine.
//! The raw_command arm is the shared control: report each layer as its
//! delta over raw in its own binary, which cancels machine and
//! tauri-version variance between the split graphs.

use bench_common::{invoke_request, mock_webview};
use ttipc::handler;
use tauri_typed_ipc_bench::{App, AsyncApp, GreeterAsyncDispatch, GreeterDispatch};
use serde_json::json;
use tauri::ipc::InvokeBody;
use tauri::test::get_ipc_response;

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
fn ttipc_async_procedure(bencher: divan::Bencher) {
    // Same greet workload as an async fn: handler -> spawn -> resolve,
    // through the identical pipeline. The gap over ttipc_procedure is the
    // spawn-and-resolve tax (tauri's runtime), which is the path taurpc's
    // async-only resolvers always take -- so this is the arm to set beside
    // taurpc.
    let (_app, webview) = mock_webview(handler(AsyncApp.into_procedures()));
    bencher
        .with_inputs(|| invoke_request("greet", InvokeBody::Json(json!({ "name": "world" }))))
        .bench_values(|request| get_ipc_response(&webview, request));
}

fn main() {
    divan::main();
}
