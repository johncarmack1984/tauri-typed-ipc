//! Dispatch cost against the direct-call floor, every outcome
//! measured.
//!
//! `dispatch` is the full generated happy path (name match +
//! serde_json argument/result roundtrip); `direct` is the same
//! procedure called as a plain method; `dispatch_injected` adds a
//! type-matched AppHandle extraction; `dispatch_channel` adds building
//! a `Channel<T>` from the context and streaming through it. The error
//! outcomes
//! (`dispatch_unknown`, `dispatch_invalid_args`) are measured so a
//! regression there is as visible as one on the happy path.
//! `procedures` runs that happy path through the type-erased
//! `Procedures` (one boxed-closure hop); `merged` mounts a second set
//! alongside, so its gap over `procedures` is the routing cost `merge`
//! adds. The end-to-end numbers live in benches/ipc.rs and
//! benchmarks/. Fixtures: ../src/lib.rs.

use std::any::Any;

use serde_json::json;
use ttipc::{Context, Dispatch};
use ttipc_tests::{
    App, BackupDispatch, CounterDispatch, Downloader, DownloadsDispatch, Greeter, GreeterDispatch,
    Tally, Vault,
};

#[divan::bench]
fn direct(bencher: divan::Bencher) {
    bencher
        .with_inputs(|| String::from("world"))
        .bench_values(|name| App.greet(name));
}

#[divan::bench]
fn dispatch(bencher: divan::Bencher) {
    let ctx = Context::empty();
    bencher
        .with_inputs(|| json!({ "name": "world" }))
        .bench_values(|args| App.dispatch(&ctx, "greet", args));
}

#[divan::bench]
fn dispatch_injected(bencher: divan::Bencher) {
    let tauri_app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app builds");
    let handle = tauri_app.handle().clone();
    let values: [&(dyn Any + Send + Sync); 1] = [&handle];
    let ctx = Context::new(&values);
    bencher
        .with_inputs(|| json!({ "name": "world" }))
        .bench_values(|args| App.dispatch(&ctx, "greet_via", args));
}

#[divan::bench]
fn dispatch_unknown(bencher: divan::Bencher) {
    let ctx = Context::empty();
    bencher
        .with_inputs(|| json!({}))
        .bench_values(|args| App.dispatch(&ctx, "nope", args));
}

#[divan::bench]
fn dispatch_invalid_args(bencher: divan::Bencher) {
    let ctx = Context::empty();
    bencher
        .with_inputs(|| json!({}))
        .bench_values(|args| App.dispatch(&ctx, "greet", args));
}

#[divan::bench]
fn dispatch_async(bencher: divan::Bencher) {
    // The synchronous cost of dispatching an async procedure: argument
    // deserialize plus boxing the future. This is ttipc's own async
    // tax over the sync `dispatch` floor; completion is measured below.
    let set = Vault.into_procedures();
    let ctx = Context::empty();
    bencher
        .with_inputs(|| json!({ "label": "x" }))
        .bench_values(|args| set.dispatch(&ctx, "snapshot", args));
}

#[divan::bench]
fn dispatch_async_resolved(bencher: divan::Bencher) {
    // The async path driven to completion: dispatch, then block on the
    // ready future. Sits above the sync floor by the runtime's
    // spawn-and-poll cost (which tauri owns, not ttipc).
    let set = Vault.into_procedures();
    let ctx = Context::empty();
    bencher
        .with_inputs(|| json!({ "label": "x" }))
        .bench_values(|args| {
            let Dispatch::Async(future) = set.dispatch(&ctx, "snapshot", args) else {
                unreachable!("snapshot is async");
            };
            tauri::async_runtime::block_on(future)
        });
}

#[divan::bench]
fn dispatch_channel(bencher: divan::Bencher) {
    // A streaming procedure: deserialize the channel id, build the typed
    // channel from the context factory, then stream. The factory's
    // channel discards sends, so this isolates ttipc's build-and-send
    // overhead from delivery (which tauri owns).
    let sink = |_id: tauri::ipc::JavaScriptChannelId| -> tauri::ipc::Channel {
        tauri::ipc::Channel::new(|_body| Ok(()))
    };
    let ctx = Context::empty().with_channels(&sink);
    let set = Downloader.into_procedures();
    bencher
        .with_inputs(|| json!({ "total": 2, "progress": "__CHANNEL__:0" }))
        .bench_values(|args| set.dispatch(&ctx, "track", args));
}

#[divan::bench]
fn procedures(bencher: divan::Bencher) {
    let ctx = Context::empty();
    let set = Tally.into_procedures();
    bencher
        .with_inputs(|| json!({ "n": 41 }))
        .bench_values(|args| set.dispatch(&ctx, "count", args));
}

#[divan::bench]
fn merged(bencher: divan::Bencher) {
    let ctx = Context::empty();
    let set = App.into_procedures().merge(Tally.into_procedures());
    bencher
        .with_inputs(|| json!({ "n": 41 }))
        .bench_values(|args| set.dispatch(&ctx, "count", args));
}

fn main() {
    divan::main();
}
