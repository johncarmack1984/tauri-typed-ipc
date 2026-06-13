//! Behavioral coverage for the generated dispatch core: the macro
//! output is an ordinary trait, so it is tested like one -- no
//! expansion inspection needed here (that snapshot lives in
//! ttipc-macros). Fixtures: ../src/lib.rs.

use std::sync::{Arc, Mutex};

use serde_json::json;
use tauri::ipc::{Channel as TauriChannel, InvokeResponseBody, JavaScriptChannelId};
use ttipc::{Context, Dispatch, DispatchError, Outcome};
use ttipc_tests::{App, BackupDispatch, Downloader, DownloadsDispatch, GreeterDispatch, Vault};

/// The synchronous half of a [`Dispatch`], for the sync procedures under
/// test here; an `async fn` would arrive as [`Dispatch::Async`].
fn sync(dispatch: Dispatch) -> Result<Outcome, DispatchError> {
    match dispatch {
        Dispatch::Sync(result) => result,
        Dispatch::Async(_) => panic!("expected a synchronous dispatch"),
    }
}

#[test]
fn dispatches_by_name() {
    assert_eq!(
        sync(App.dispatch(&Context::empty(), "greet", json!({ "name": "world" }))).unwrap(),
        Outcome::Resolve(json!("Hello, world!"))
    );
}

#[test]
fn unknown_procedure_is_an_error() {
    let err = sync(App.dispatch(&Context::empty(), "nope", json!({}))).unwrap_err();
    assert!(matches!(&err, DispatchError::UnknownProcedure(p) if p == "nope"));
}

#[test]
fn bad_arguments_are_an_error() {
    let err = sync(App.dispatch(&Context::empty(), "greet", json!({}))).unwrap_err();
    assert!(matches!(err, DispatchError::Deserialize(_)));
}

#[test]
fn context_extracts_by_type() {
    let value = 7u16;
    let values: [&(dyn std::any::Any + Send + Sync); 1] = [&value];
    let ctx = Context::new(&values);
    assert_eq!(ctx.extract::<u16>(), Some(7));
    assert_eq!(ctx.extract::<u32>(), None);
}

#[test]
fn missing_injection_is_an_error() {
    let err =
        sync(App.dispatch(&Context::empty(), "greet_via", json!({ "name": "world" }))).unwrap_err();
    assert!(matches!(err, DispatchError::MissingInjection(ty) if ty.contains("AppHandle")));
}

#[test]
fn async_procedure_resolves() {
    let set = Vault.into_procedures();
    let Dispatch::Async(future) =
        set.dispatch(&Context::empty(), "snapshot", json!({ "label": "x" }))
    else {
        panic!("an async procedure dispatches to a future");
    };
    let outcome = tauri::async_runtime::block_on(future).unwrap();
    assert_eq!(outcome, Outcome::Resolve(json!("snapshot: x")));
}

#[test]
fn async_result_ok_resolves() {
    let set = Vault.into_procedures();
    let Dispatch::Async(future) =
        set.dispatch(&Context::empty(), "try_snapshot", json!({ "label": "x" }))
    else {
        panic!("an async procedure dispatches to a future");
    };
    let outcome = tauri::async_runtime::block_on(future).unwrap();
    assert_eq!(outcome, Outcome::Resolve(json!("ok: x")));
}

#[test]
fn async_result_err_rejects_typed_error() {
    // An empty label returns `Err(BackupError::Empty)`, which rejects
    // with the typed error's wire shape -- not a DispatchError string.
    let set = Vault.into_procedures();
    let Dispatch::Async(future) =
        set.dispatch(&Context::empty(), "try_snapshot", json!({ "label": "" }))
    else {
        panic!("an async procedure dispatches to a future");
    };
    let outcome = tauri::async_runtime::block_on(future).unwrap();
    assert_eq!(
        outcome,
        Outcome::Reject(json!({ "type": "empty", "message": "the label is empty" }))
    );
}

#[test]
fn async_bad_arguments_fail_without_a_spawn() {
    // Wire args deserialize up front, so malformed input settles
    // synchronously rather than spawning a doomed future.
    let set = Vault.into_procedures();
    let dispatch = set.dispatch(&Context::empty(), "snapshot", json!({}));
    assert!(matches!(
        dispatch,
        Dispatch::Sync(Err(DispatchError::Deserialize(_)))
    ));
}

/// A channel factory that records every sent body, standing in for the
/// webview-backed one the handler installs. The returned `Vec` collects
/// what a procedure streams; the closure builds a tauri channel whose
/// `send` pushes into it.
fn recording_channels() -> (
    Arc<Mutex<Vec<InvokeResponseBody>>>,
    impl Fn(JavaScriptChannelId) -> TauriChannel,
) {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let sink = recorded.clone();
    let factory = move |_id: JavaScriptChannelId| -> TauriChannel {
        let sink = sink.clone();
        TauriChannel::new(move |body| {
            sink.lock().expect("recording lock").push(body);
            Ok(())
        })
    };
    (recorded, factory)
}

/// Each recorded body parsed back to JSON, for comparing against what
/// the procedure streamed.
fn recorded_json(recorded: &Arc<Mutex<Vec<InvokeResponseBody>>>) -> Vec<serde_json::Value> {
    recorded
        .lock()
        .expect("recording lock")
        .iter()
        .map(|body| body.clone().deserialize().expect("a JSON body"))
        .collect()
}

#[test]
fn channel_dispatch_streams_sends() {
    // A sync streaming procedure: the channel id rides the wire next to
    // a normal argument, the typed channel is built from the context,
    // and each `send` reaches the factory's sink, serialized like a
    // command return.
    let (recorded, channels) = recording_channels();
    let ctx = Context::empty().with_channels(&channels);
    let set = Downloader.into_procedures();
    let outcome = sync(set.dispatch(
        &ctx,
        "track",
        json!({ "total": 2, "progress": "__CHANNEL__:0" }),
    ))
    .unwrap();
    assert_eq!(outcome, Outcome::Resolve(json!(null)));
    assert_eq!(
        recorded_json(&recorded),
        vec![
            json!({ "done": 1, "total": 2 }),
            json!({ "done": 2, "total": 2 }),
        ]
    );
}

#[test]
fn channel_dispatch_streams_from_async() {
    // The same over the async path: the channel is built in the
    // synchronous prelude and owned into the spawned future, which
    // streams once driven to completion.
    let (recorded, channels) = recording_channels();
    let ctx = Context::empty().with_channels(&channels);
    let set = Downloader.into_procedures();
    let Dispatch::Async(future) = set.dispatch(
        &ctx,
        "track_async",
        json!({ "total": 2, "progress": "__CHANNEL__:0" }),
    ) else {
        panic!("track_async is async");
    };
    let outcome = tauri::async_runtime::block_on(future).unwrap();
    assert_eq!(outcome, Outcome::Resolve(json!(null)));
    assert_eq!(
        recorded_json(&recorded),
        vec![
            json!({ "done": 1, "total": 2 }),
            json!({ "done": 2, "total": 2 }),
        ]
    );
}

#[test]
fn missing_channel_factory_is_an_error() {
    // Dispatching a streaming procedure against a bare context (no
    // channel factory) fails loudly rather than silently dropping the
    // stream.
    let set = Downloader.into_procedures();
    let err = sync(set.dispatch(
        &Context::empty(),
        "track",
        json!({ "total": 1, "progress": "__CHANNEL__:0" }),
    ))
    .unwrap_err();
    assert!(matches!(err, DispatchError::MissingChannel(ty) if ty.contains("Channel")));
}
