//! Microbenchmark: rendering a single-file TypeScript client from the
//! generated descriptor. Fixtures: ../src/lib.rs.

use ttipc::{Bindings, Layout};
use ttipc_tests::{
    DownloadsProcedures, FaderEvent, FadersProcedures, GuardedProcedures, MixerProcedures,
    PostsProcedures, ScenesProcedures, SignalEvent, TagsProcedures,
};

fn main() {
    divan::main();
}

#[divan::bench]
fn export_flat_file() -> String {
    Bindings::new()
        .register::<FadersProcedures>()
        .export()
        .expect("client renders")
}

#[divan::bench]
fn export_router() -> String {
    Bindings::new()
        .register::<FadersProcedures>()
        .register::<MixerProcedures>()
        .router("createTauRPCProxy")
        .export()
        .expect("client renders")
}

#[divan::bench]
fn export_namespaced() -> String {
    Bindings::new()
        .register::<PostsProcedures>()
        .register::<TagsProcedures>()
        .export()
        .expect("client renders")
}

#[divan::bench]
fn export_events() -> String {
    Bindings::new()
        .register_events::<FaderEvent>()
        .export()
        .expect("client renders")
}

#[divan::bench]
fn export_multi_variant_events() -> String {
    // A multi-variant group exercises the fan-out listener -- the async
    // Promise.all path -- distinct from the single-variant passthrough in
    // export_events.
    Bindings::new()
        .register_events::<SignalEvent>()
        .export()
        .expect("client renders")
}

#[divan::bench]
fn export_named_types() -> String {
    Bindings::new()
        .register::<ScenesProcedures>()
        .export()
        .expect("client renders")
}

#[divan::bench]
fn export_cased_names() -> String {
    // Multi-word names exercise the camelCase transform and the wire-key
    // mapping in the invoke object.
    Bindings::new()
        .register::<MixerProcedures>()
        .export()
        .expect("client renders")
}

#[divan::bench]
fn export_typed_error() -> String {
    // A fallible procedure exercises the `@throws` JSDoc and the error
    // discriminated-union rendering.
    Bindings::new()
        .register::<GuardedProcedures>()
        .export()
        .expect("client renders")
}

#[divan::bench]
fn export_channels() -> String {
    // A streaming procedure exercises the `Channel<T>` rendering and the
    // combined invoke/Channel import.
    Bindings::new()
        .register::<DownloadsProcedures>()
        .export()
        .expect("client renders")
}

#[divan::bench]
fn check_flat_file(bencher: divan::Bencher) {
    // The consumer drift guard: render, read the committed file, compare.
    // Seeded once outside the timed loop so this measures the check path,
    // not the temp-file setup.
    let bindings = Bindings::new().register::<FadersProcedures>();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bindings.ts");
    bindings
        .export_to(&path)
        .expect("seed the committed client");
    bencher.bench(|| bindings.check(&path).expect("current"));
}

#[divan::bench]
fn check_files(bencher: divan::Bencher) {
    // The Files-layout guard: render to a temp tree, read it back, and
    // compare against the committed directory. Seeded once outside the loop.
    let bindings = Bindings::new()
        .layout(Layout::Files)
        .register::<ScenesProcedures>();
    let dir = tempfile::tempdir().expect("tempdir");
    bindings
        .export_to(dir.path())
        .expect("seed the committed tree");
    bencher.bench(|| bindings.check(dir.path()).expect("current"));
}

#[divan::bench]
fn export_router_files(bencher: divan::Bencher) {
    // The drop-in factory under the multi-file layout: the runtime body
    // (objects, events, factory) renders into index.ts while the type defs
    // split out. Writes a tree, so it is set up like check_files.
    let bindings = Bindings::new()
        .layout(Layout::Files)
        .register::<PostsProcedures>()
        .register_events::<FaderEvent>()
        .router("createTauRPCProxy");
    let dir = tempfile::tempdir().expect("tempdir");
    bencher.bench(|| bindings.export_to(dir.path()).expect("client renders"));
}
