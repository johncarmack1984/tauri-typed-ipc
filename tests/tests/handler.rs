//! The generated dispatch mounted on tauri's invoke pipeline, driven
//! through the mock runtime. Wire parity with raw commands is the
//! contract under test. Fixtures: ../src/lib.rs.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use serde_json::json;
use tauri::ipc::{Channel as TauriChannel, InvokeBody, InvokeResponseBody, JavaScriptChannelId};
use tauri::test::get_ipc_response;
use ttipc::{Context, Dispatch, handler, handler_with_fallback, procedures};
use ttipc_tests::{
    App, BackupDispatch, CountedDispatch, CounterDispatch, Downloader, DownloadsDispatch,
    GreeterDispatch, Hits, PostStore, PostsDispatch, Service, TagStore, TagsDispatch, Tally, Vault,
    invoke_request, mock_webview, mock_webview_managed,
};

#[tauri::command]
fn ping() -> &'static str {
    "pong"
}

// Raw commands that mirror ttipc procedures byte-for-byte, for the
// wire-parity assertions: identical return value and identical typed
// error, dispatched through the same MockRuntime.
#[tauri::command]
fn raw_greet(name: String) -> String {
    format!("Hello, {name}!")
}

#[tauri::command]
fn raw_locked() -> Result<(), ttipc_tests::DeskError> {
    Err(ttipc_tests::DeskError::Locked)
}

// A ttipc procedure set whose fallible method rejects the same
// `DeskError` as `raw_locked`, so the two reject shapes can be compared.
#[procedures]
trait Locker {
    fn lock(&self) -> Result<(), ttipc_tests::DeskError>;
}

struct Bolt;

impl Locker for Bolt {
    fn lock(&self) -> Result<(), ttipc_tests::DeskError> {
        Err(ttipc_tests::DeskError::Locked)
    }
}

/// The exact wire bytes of a JSON response body. Procedures and raw
/// commands both answer with `InvokeResponseBody::Json`, so comparing
/// these strings is a byte-level parity check, stronger than comparing
/// deserialized values.
fn json_bytes(body: InvokeResponseBody) -> String {
    match body {
        InvokeResponseBody::Json(s) => s,
        InvokeResponseBody::Raw(bytes) => {
            panic!("expected a JSON body, got {} raw bytes", bytes.len())
        }
    }
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

// A locally-defined set whose async procedure panics inside the spawned
// future. The fixtures crate is off-limits here, and the panic case is
// pathological, so it lives next to the test that asserts its behavior.
#[procedures]
trait Crasher {
    async fn boom(&self) -> u32;
}

struct Bomb;

impl Crasher for Bomb {
    async fn boom(&self) -> u32 {
        panic!("boom inside the spawned future")
    }
}

#[test]
fn async_panic_neither_resolves_nor_rejects() {
    // Observed behavior: when an `async fn` procedure's spawned future
    // panics, the panic unwinds the spawned task before `settle` runs, so
    // the future never yields an `Outcome` and the `InvokeResolver` is
    // dropped without resolving or rejecting -- neither IPC callback ever
    // fires. (Driving this through the real `get_ipc_response` does not
    // hang but *panics* in the helper: dropping the resolver drops the
    // responder that holds its result sender, so its blocking
    // `rx.recv().expect(..)` hits `RecvError`. Either way the invoke is
    // never settled.)
    //
    // We reproduce the handler's spawn exactly -- take the `Async` future
    // from dispatch, spawn it on tauri's runtime wrapped to report
    // completion -- and assert the report channel never delivers: the
    // future is unwound, so its `tx` is dropped rather than sent on, and a
    // bounded `recv_timeout` returns an error instead of `Ok`. Asserts the
    // non-settlement without hanging the test.
    let set = Bomb.into_procedures();
    let Dispatch::Async(future) = set.dispatch(&Context::empty(), "boom", json!({})) else {
        panic!("boom is async");
    };

    let (tx, rx) = mpsc::sync_channel::<()>(1);
    tauri::async_runtime::spawn(async move {
        let _outcome = future.await;
        let _ = tx.send(());
    });

    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "a panicking async procedure must not settle the invoke",
    );
}

#[test]
fn wire_bytes_match_a_raw_command() {
    // The headline claim, asserted at the byte level: a ttipc procedure
    // and an equivalent raw `#[tauri::command]` produce identical wire
    // bytes through the same MockRuntime, for a resolve, a typed-error
    // reject, and a channel send.

    // Resolve: identical JSON body bytes, not just an equal value.
    let (_app, webview) = mock_webview(handler_with_fallback(
        App.into_procedures(),
        tauri::generate_handler![raw_greet],
    ));
    let proc_body = get_ipc_response(
        &webview,
        invoke_request("greet", InvokeBody::Json(json!({ "name": "world" }))),
    )
    .expect("the procedure resolves");
    let raw_body = get_ipc_response(
        &webview,
        invoke_request("raw_greet", InvokeBody::Json(json!({ "name": "world" }))),
    )
    .expect("the raw command resolves");
    assert_eq!(json_bytes(proc_body), json_bytes(raw_body));

    // Typed-error reject: the rejected value is byte-identical. (Reject
    // arrives as a `serde_json::Value`, not an `InvokeResponseBody`, so
    // this is the strongest shape the helper exposes for the error path.)
    let (_app, webview) = mock_webview(handler_with_fallback(
        Bolt.into_procedures(),
        tauri::generate_handler![raw_locked],
    ));
    let proc_err = get_ipc_response(
        &webview,
        invoke_request("lock", InvokeBody::Json(json!({}))),
    )
    .expect_err("the procedure rejects its typed error");
    let raw_err = get_ipc_response(
        &webview,
        invoke_request("raw_locked", InvokeBody::Json(json!({}))),
    )
    .expect_err("the raw command rejects its typed error");
    assert_eq!(proc_err, raw_err);

    // Channel send: a ttipc `Channel::send` and a raw `tauri::ipc::Channel`
    // produce the same `InvokeResponseBody`. The streamed body rides
    // tauri's eval path, not the invoke response, so it never returns
    // through `get_ipc_response`; both sides are captured with a recording
    // channel and compared by their JSON bytes.
    let ttipc_sent: Arc<std::sync::Mutex<Vec<InvokeResponseBody>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = ttipc_sent.clone();
    let factory = move |_id: JavaScriptChannelId| -> TauriChannel {
        let sink = sink.clone();
        TauriChannel::new(move |body| {
            sink.lock().expect("ttipc sink").push(body);
            Ok(())
        })
    };
    let ctx = Context::empty().with_channels(&factory);
    let set = Downloader.into_procedures();
    let outcome = set.dispatch(
        &ctx,
        "track",
        json!({ "total": 2, "progress": "__CHANNEL__:0" }),
    );
    let Dispatch::Sync(Ok(_)) = outcome else {
        panic!("track resolves synchronously");
    };

    // The same two values streamed through a raw tauri channel.
    let raw_sent: Arc<std::sync::Mutex<Vec<InvokeResponseBody>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let raw_sink = raw_sent.clone();
    let raw_channel: TauriChannel<ttipc_tests::Progress> = TauriChannel::new(move |body| {
        raw_sink.lock().expect("raw sink").push(body);
        Ok(())
    });
    raw_channel
        .send(ttipc_tests::Progress { done: 1, total: 2 })
        .expect("raw send 1");
    raw_channel
        .send(ttipc_tests::Progress { done: 2, total: 2 })
        .expect("raw send 2");

    let ttipc_bytes: Vec<String> = ttipc_sent
        .lock()
        .expect("ttipc sink")
        .drain(..)
        .map(json_bytes)
        .collect();
    let raw_bytes: Vec<String> = raw_sent
        .lock()
        .expect("raw sink")
        .drain(..)
        .map(json_bytes)
        .collect();
    assert_eq!(ttipc_bytes, raw_bytes);
}

#[test]
fn channel_streams_by_value_through_the_real_path() {
    // End-to-end over tauri's real channel path: the handler builds the
    // channel from the webview with `JavaScriptChannelId::channel_on`, and
    // each `send` runs that channel's own delivery. A `channel_interceptor`
    // -- tauri's supported seam, invoked inside `channel_on` before the
    // body would hit `eval` -- captures every streamed `InvokeResponseBody`
    // by value, so this asserts the streamed `Progress` values arrive, not
    // merely that the call returns Ok.
    // `Progress` is serialize-only (no `Deserialize`), so the captured
    // body is read back as JSON -- still by value, just untyped.
    let streamed: Arc<std::sync::Mutex<Vec<(usize, serde_json::Value)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = streamed.clone();
    let app = tauri::test::mock_builder()
        .channel_interceptor(move |_webview, _callback, index, body| {
            let value: serde_json::Value =
                body.clone().deserialize().expect("a streamed JSON body");
            sink.lock().expect("stream sink").push((index, value));
            true // consume: the test reads it here instead of eval'ing it
        })
        .invoke_handler(handler(Downloader.into_procedures()))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview builds");

    get_ipc_response(
        &webview,
        invoke_request(
            "track",
            InvokeBody::Json(json!({ "total": 3, "progress": "__CHANNEL__:7" })),
        ),
    )
    .expect("the streaming command resolves");

    let captured = streamed.lock().expect("stream sink");
    let indices: Vec<usize> = captured.iter().map(|(i, _)| *i).collect();
    let values: Vec<&serde_json::Value> = captured.iter().map(|(_, v)| v).collect();
    // The real channel orders messages, so indices count from zero.
    assert_eq!(indices, vec![0, 1, 2]);
    assert_eq!(
        values,
        vec![
            &json!({ "done": 1, "total": 3 }),
            &json!({ "done": 2, "total": 3 }),
            &json!({ "done": 3, "total": 3 }),
        ]
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
