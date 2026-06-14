//! taurpc arm: full invoke pipeline through the mock runtime.
//!
//! Compare against ../tauri-typed-ipc/benches/ipc.rs run on the same machine.
//! The raw_command arm is the shared control: report each layer as its
//! delta over raw in its own binary, which cancels machine and
//! tauri-version variance between the split graphs. The taurpc path is
//! async by design (command parse + handler lookup + Arc + channel
//! send + task resolution), so its delta includes the executor
//! round-trip -- that is the cost difference being measured, not an
//! artifact.

use bench_common::{invoke_request, mock_webview};
use serde_json::json;
use tauri::ipc::InvokeBody;
use tauri::test::get_ipc_response;
use taurpc_bench::{Greeter, GreeterImpl};

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
fn taurpc_procedure(bencher: divan::Bencher) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _guard = runtime.enter();

    let (_app, webview) = mock_webview(taurpc::create_ipc_handler(GreeterImpl.into_handler()));
    bencher
        .with_inputs(|| {
            invoke_request(
                "TauRPC__greet",
                InvokeBody::Json(json!({ "name": "world" })),
            )
        })
        .bench_values(|request| get_ipc_response(&webview, request));
}

fn main() {
    divan::main();
}
