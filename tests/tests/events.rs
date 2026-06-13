//! The Event derive's emit half: a variant emits under its derived wire
//! name ("fader:changed") with the fields as the JSON payload, captured
//! here through a mock app's listener. `emit` broadcasts; `emit_to`
//! scopes delivery to one window or webview. Fixtures: ../src/lib.rs.

use std::sync::{Arc, Mutex};
use tauri::Listener;
use ttipc_tests::{FaderEvent, mock_webview};

#[test]
fn emit_delivers_named_payload() {
    let (app, _webview) = mock_webview(|_| false);
    let handle = app.handle();

    let captured = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    handle.listen("fader:changed", move |event| {
        *sink.lock().expect("lock") = Some(event.payload().to_string());
    });

    FaderEvent::Changed {
        channel: 3,
        value: 200,
    }
    .emit(handle)
    .expect("emit succeeds");

    assert_eq!(
        captured.lock().expect("lock").as_deref(),
        Some(r#"{"channel":3,"value":200}"#),
    );
}

#[test]
fn emit_to_delivers_only_to_the_targeted_label() {
    let (app, webview) = mock_webview(|_| false);
    let handle = app.handle();

    let captured = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    // The mock webview window is labeled "main"; this listener is scoped
    // to it, so a label-targeted emit reaches it only when the labels match.
    webview.listen("fader:changed", move |event| {
        *sink.lock().expect("lock") = Some(event.payload().to_string());
    });

    // A different label is filtered out -- not delivered.
    FaderEvent::Changed {
        channel: 1,
        value: 1,
    }
    .emit_to(handle, "other")
    .expect("emit_to succeeds");
    assert_eq!(captured.lock().expect("lock").as_deref(), None);

    // The matching label is delivered, with the same payload as `emit`.
    FaderEvent::Changed {
        channel: 3,
        value: 200,
    }
    .emit_to(handle, "main")
    .expect("emit_to succeeds");
    assert_eq!(
        captured.lock().expect("lock").as_deref(),
        Some(r#"{"channel":3,"value":200}"#),
    );
}
