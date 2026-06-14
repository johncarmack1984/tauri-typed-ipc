//! Correctness gate for the taurpc arm: the bench is only meaningful
//! if both arms actually compute the same thing over their wires.

use bench_common::{invoke_request, mock_webview};
use serde_json::json;
use tauri::ipc::InvokeBody;
use tauri::test::get_ipc_response;
use taurpc_bench::{Greeter, GreeterImpl};

#[test]
fn taurpc_greet_over_ipc() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let _guard = runtime.enter();

    let (_app, webview) = mock_webview(taurpc::create_ipc_handler(GreeterImpl.into_handler()));
    let response = get_ipc_response(
        &webview,
        invoke_request(
            "TauRPC__greet",
            InvokeBody::Json(json!({ "name": "world" })),
        ),
    )
    .expect("greet should resolve");
    assert_eq!(
        response.deserialize::<String>().expect("a string response"),
        "Hello, world!"
    );
}
