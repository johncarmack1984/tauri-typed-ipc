//! Mock-IPC plumbing shared by the benchmark twins, kept identical by
//! construction so the only variable between the arms is the IPC layer
//! under test.

use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, mock_context, noop_assets};
use tauri::webview::InvokeRequest;

/// A ready-to-send invoke request, shaped exactly as the webview
/// shapes one.
pub fn invoke_request(cmd: &str, body: InvokeBody) -> InvokeRequest {
    InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: if cfg!(any(windows, target_os = "android")) {
            "http://tauri.localhost"
        } else {
            "tauri://localhost"
        }
        .parse()
        .expect("static url parses"),
        body,
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    }
}

/// A mock app and webview wired to the given invoke handler. Keep the
/// app alive for as long as the webview is used.
pub fn mock_webview<F>(handler: F) -> (tauri::App<MockRuntime>, tauri::WebviewWindow<MockRuntime>)
where
    F: Fn(tauri::ipc::Invoke<MockRuntime>) -> bool + Send + Sync + 'static,
{
    let app = tauri::test::mock_builder()
        .invoke_handler(handler)
        .build(mock_context(noop_assets()))
        .expect("mock app builds");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview builds");
    (app, webview)
}
