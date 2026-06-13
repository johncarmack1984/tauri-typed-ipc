//! The generated dispatch mounted on tauri's invoke pipeline, driven
//! through the mock runtime. Wire parity with raw commands is the
//! contract under test. Fixtures: ../src/lib.rs.

use serde_json::json;
use tauri::ipc::InvokeBody;
use tauri::test::get_ipc_response;
use ttipc::{handler, handler_with_fallback};
use ttipc_tests::{
    App, BackupDispatch, CountedDispatch, CounterDispatch, Downloader, DownloadsDispatch,
    GreeterDispatch, Hits, PostStore, PostsDispatch, Service, TagStore, TagsDispatch, Tally, Vault,
    invoke_request, mock_webview, mock_webview_managed,
};

#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

#[test]
fn procedure_over_ipc() {
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
fn injected_app_handle_over_ipc() {
    let (_app, webview) = mock_webview(handler(App.into_procedures()));
    let response = get_ipc_response(
        &webview,
        invoke_request("greet_via", InvokeBody::Json(json!({ "name": "world" }))),
    )
    .expect("greet_via should resolve with the runtime's own handle");
    assert_eq!(
        response.deserialize::<String>().expect("a string response"),
        "Hello via app, world!"
    );
}

#[test]
fn unknown_command_gets_tauris_not_found() {
    let (_app, webview) = mock_webview(handler(App.into_procedures()));
    let err = get_ipc_response(
        &webview,
        invoke_request("nope", InvokeBody::Json(json!({}))),
    )
    .expect_err("unknown command should reject");
    assert_eq!(err, json!("Command nope not found"));
}

#[test]
fn invalid_args_reject_with_dispatch_error() {
    let (_app, webview) = mock_webview(handler(App.into_procedures()));
    let err = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({}))),
    )
    .expect_err("missing args should reject");
    let message = err.as_str().expect("error is a string");
    assert!(message.starts_with("invalid arguments:"), "got: {message}");
}

#[test]
fn raw_body_rejects() {
    let (_app, webview) = mock_webview(handler(App.into_procedures()));
    let err = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Raw(vec![1, 2, 3])),
    )
    .expect_err("raw body should reject");
    assert_eq!(err, json!("procedures take JSON arguments, not a raw body"));
}

#[test]
fn fallback_reaches_raw_commands() {
    let (_app, webview) = mock_webview(handler_with_fallback(
        App.into_procedures(),
        tauri::generate_handler![ping],
    ));

    let response = get_ipc_response(
        &webview,
        invoke_request("ping", InvokeBody::Json(json!({}))),
    )
    .expect("raw command via fallback");
    assert_eq!(response.deserialize::<String>().expect("a string"), "pong");

    let response = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({ "name": "wire" }))),
    )
    .expect("procedure still reachable");
    assert_eq!(
        response.deserialize::<String>().expect("a string"),
        "Hello, wire!"
    );
}

#[test]
fn merged_sets_share_one_handler() {
    let procedures = App.into_procedures().merge(Tally.into_procedures());
    let (_app, webview) = mock_webview(handler_with_fallback(
        procedures,
        tauri::generate_handler![ping],
    ));

    // A procedure from the first set.
    let greet = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({ "name": "world" }))),
    )
    .expect("the first set's procedure resolves");
    assert_eq!(
        greet.deserialize::<String>().expect("a string"),
        "Hello, world!"
    );

    // A procedure from the second set, through the same handler.
    let count = get_ipc_response(
        &webview,
        invoke_request("count", InvokeBody::Json(json!({ "n": 41 }))),
    )
    .expect("the second set's procedure resolves");
    assert_eq!(count.deserialize::<u32>().expect("a u32"), 42);

    // A command in neither set still reaches the raw fallback.
    let pong = get_ipc_response(
        &webview,
        invoke_request("ping", InvokeBody::Json(json!({}))),
    )
    .expect("raw command via fallback");
    assert_eq!(pong.deserialize::<String>().expect("a string"), "pong");
}

#[test]
#[should_panic(expected = "registered by more than one merged set")]
fn merge_rejects_duplicate_names() {
    // Two sets that both answer to "greet" cannot coexist; the clash
    // is a startup panic naming the command, not a silent shadow.
    let _ = App.into_procedures().merge(App.into_procedures());
}

#[test]
fn namespaced_sets_coexist_and_route_by_namespace() {
    // Two sets sharing the method name `list` only coexist because the
    // namespace prefixes the wire command -- merge does not panic, and
    // each command routes to its own set.
    let procedures = PostStore
        .into_procedures()
        .merge(TagStore.into_procedures());
    let (_app, webview) = mock_webview(handler(procedures));

    let posts = get_ipc_response(
        &webview,
        invoke_request("posts.list", InvokeBody::Json(json!({}))),
    )
    .expect("posts.list routes to the posts set");
    assert_eq!(posts.deserialize::<u32>().expect("a u32"), 1);

    let tags = get_ipc_response(
        &webview,
        invoke_request("tags.list", InvokeBody::Json(json!({}))),
    )
    .expect("tags.list routes to the tags set");
    assert_eq!(tags.deserialize::<u32>().expect("a u32"), 2);
}

#[test]
fn injected_state_over_ipc() {
    let (_app, webview) = mock_webview_managed(Hits(7), handler(Service.into_procedures()));
    let response = get_ipc_response(
        &webview,
        invoke_request("hits", InvokeBody::Json(json!({}))),
    )
    .expect("hits resolves from managed state");
    assert_eq!(response.deserialize::<u32>().expect("a u32"), 7);
}

#[test]
fn unmanaged_state_rejects() {
    // The same procedure with nothing managed: injection finds no Hits
    // and fails loudly rather than silently substituting a default.
    let (_app, webview) = mock_webview(handler(Service.into_procedures()));
    let err = get_ipc_response(
        &webview,
        invoke_request("hits", InvokeBody::Json(json!({}))),
    )
    .expect_err("missing managed state should reject");
    let message = err.as_str().expect("error is a string");
    assert!(message.starts_with("state not managed:"), "got: {message}");
}

#[test]
fn async_procedure_over_ipc() {
    // An async procedure resolves through the spawn-on-the-runtime path,
    // wire-identical to a sync one from the client's view.
    let (_app, webview) = mock_webview(handler(Vault.into_procedures()));
    let response = get_ipc_response(
        &webview,
        invoke_request("snapshot", InvokeBody::Json(json!({ "label": "x" }))),
    )
    .expect("async snapshot should resolve");
    assert_eq!(
        response.deserialize::<String>().expect("a string response"),
        "snapshot: x"
    );
}

#[test]
fn channel_command_over_ipc() {
    // The handler builds a channel factory from the real webview, so a
    // streaming procedure resolves: its sends reach tauri's own delivery
    // path without error. (The streamed payload rides that eval path,
    // asserted by value in the dispatch test.)
    let (_app, webview) = mock_webview(handler(Downloader.into_procedures()));
    let response = get_ipc_response(
        &webview,
        invoke_request(
            "track",
            InvokeBody::Json(json!({ "total": 2, "progress": "__CHANNEL__:0" })),
        ),
    )
    .expect("a streaming command resolves");
    assert_eq!(
        response.deserialize::<serde_json::Value>().expect("a body"),
        json!(null)
    );
}

#[test]
fn async_typed_error_rejects_over_ipc() {
    // A procedure's `Err` rejects with the typed error's wire object
    // (`{ type, message }`), parity with a raw command returning Result
    // -- not a DispatchError string.
    let (_app, webview) = mock_webview(handler(Vault.into_procedures()));
    let err = get_ipc_response(
        &webview,
        invoke_request("try_snapshot", InvokeBody::Json(json!({ "label": "" }))),
    )
    .expect_err("an empty label should reject");
    assert_eq!(
        err,
        json!({ "type": "empty", "message": "the label is empty" })
    );
}
