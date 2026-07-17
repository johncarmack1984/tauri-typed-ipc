//! TypeScript client generation: the descriptor `#[procedures]` emits,
//! rendered to a single-file client. Injected parameters drop out, the
//! return shapes (`null`, `number`, `string`) come straight from specta,
//! and the command names match the wire. Fixtures: ../src/lib.rs.

use std::path::Path;
use ttipc::{Bindings, BindingsError, Layout, MethodCase};
use ttipc_tests::{
    DownloadsProcedures, FaderEvent, FadersProcedures, GuardedProcedures, LedgerProcedures,
    MixerProcedures, NotesProcedures, PatchesProcedures, PostsEvent, PostsProcedures,
    ScenesProcedures, SignalEvent, TagsProcedures,
};

#[test]
fn flat_file_client() {
    let client = Bindings::new()
        .register::<FadersProcedures>()
        .export()
        .expect("faders client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn named_types_client() {
    // A custom struct reaches the client as an `export type` and is
    // referenced by name in the command signatures, not inlined.
    let client = Bindings::new()
        .register::<ScenesProcedures>()
        .export()
        .expect("scenes client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn events_client() {
    // An event enum becomes a discriminated union plus an
    // `events.{group}.listen` that subscribes to each variant's wire name.
    let client = Bindings::new()
        .register_events::<FaderEvent>()
        .export()
        .expect("events client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn single_tuple_event_nests_payload_under_data() {
    let client = Bindings::new()
        .register_events::<SignalEvent>()
        .export()
        .expect("events client renders");

    // A single-field tuple variant V(T) nests its payload under `data` (so any
    // T works -- primitive, collection -- where spread named fields can't);
    // unit and named variants are unchanged.
    assert!(client.contains(r#"{ type: "ready" }"#), "got:\n{client}");
    assert!(
        client.contains(r#"{ type: "progress"; done: number }"#),
        "got:\n{client}"
    );
    assert!(
        client.contains(r#"{ type: "code"; data: number }"#),
        "got:\n{client}"
    );
    assert!(
        client.contains(r#"{ type: "batch"; data: number[] }"#),
        "got:\n{client}"
    );
    // The listener forwards the value as `data`, not spread.
    assert!(
        client.contains(r#"callback({ type: "code", data: event.payload })"#),
        "got:\n{client}"
    );
}

#[test]
fn multi_variant_events_client() {
    // A multi-variant group cannot return a single subscription, so its
    // listen fans out: an async method that awaits every subscription via
    // Promise.all, then returns one unlisten dropping them all. Locks that
    // shape plus each variant's forwarding (unit, named, tuple).
    let client = Bindings::new()
        .register_events::<SignalEvent>()
        .export()
        .expect("events client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn commands_and_events_coexist() {
    // The shape the real example wires: a command object and an event
    // listener object in one file, both imports present.
    let client = Bindings::new()
        .register::<FadersProcedures>()
        .register_events::<FaderEvent>()
        .export()
        .expect("client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn method_case_defaults_to_camel() {
    // Multi-word names: camelCase method and args in TypeScript, while
    // the wire command and arg keys stay the snake_case Rust idents.
    let client = Bindings::new()
        .register::<MixerProcedures>()
        .export()
        .expect("mixer client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn method_case_snake_is_the_taurpc_dropin() {
    // The taurpc drop-in: method names stay snake_case (verbatim), args
    // are still camelCase -- matching taurpc's generated proxy.
    let client = Bindings::new()
        .method_case(MethodCase::Snake)
        .register::<MixerProcedures>()
        .export()
        .expect("mixer client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn typed_error_client() {
    // A fallible procedure renders `Promise<T>` (it rejects -- wire
    // parity) with a `@throws` JSDoc, plus the error's discriminated
    // union as an `export type`. The infallible sibling has neither.
    let client = Bindings::new()
        .register::<GuardedProcedures>()
        .export()
        .expect("guarded client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn string_error_is_the_taurpc_dropin() {
    // `Result<_, String>` (the taurpc drop-in) renders `Promise<T>` with a
    // `@throws {string}` JSDoc -- the rejection is the built-in `string`, with no
    // `export type` alias. The infallible sibling gets neither.
    let client = Bindings::new()
        .register::<NotesProcedures>()
        .export()
        .expect("notes client renders");
    assert!(
        client.contains("@throws {string}") && !client.contains("export type string"),
        "the string error should render inline with no alias:\n{client}"
    );
    insta::assert_snapshot!(client);
}

#[test]
fn channels_client() {
    // A streaming procedure renders its `Channel<T>` parameter as
    // `Channel<T>` -- after the plain arguments, with `Channel` imported
    // from @tauri-apps/api/core -- and the payload type reaches the
    // client as an `export type`. The invoke passes the channel through
    // like any argument (it serializes to its id on the wire).
    let client = Bindings::new()
        .register::<DownloadsProcedures>()
        .export()
        .expect("downloads client renders");
    insta::assert_snapshot!(client);
}

#[test]
fn asymmetric_type_renders_unified() {
    // A `skip_serializing_if` field's serialize shape (may be omitted) and
    // deserialize shape (Option accepts omission via serde's implicit None)
    // meet in one sound TypeScript shape: `note?: string | null`. specta's
    // unified Format renders it directly as of 2.0.0-rc.26; under rc.25
    // ttipc surfaced a loud BindingsError here instead (the 0.1 boundary).
    let client = Bindings::new()
        .register::<PatchesProcedures>()
        .export()
        .expect("an asymmetric optional field renders under unified Format");
    assert!(
        client.contains("note?: string | null"),
        "the field should render optional-and-nullable:\n{client}"
    );
    insta::assert_snapshot!(client);
}

#[test]
fn bigint_type_errors_loudly() {
    // Specta forbids exporting BigInt-style integers (i64/u64/usize/...)
    // by default to avoid silent precision loss past 2^53. ttipc
    // surfaces that as a loud BindingsError rather than emitting a lossy
    // `number`. Lossless bigint transport is post-0.1: it needs an
    // upstream tauri invoke reviver (responses are parsed with
    // `response.json()`, with no hook to add one).
    let err = Bindings::new()
        .register::<LedgerProcedures>()
        .export()
        .expect_err("a BigInt-style integer cannot render losslessly");
    let message = err.to_string();
    assert!(message.contains("BigInt"), "got: {message}");
}

#[test]
fn check_guards_against_drift() {
    // The blessed consumer drift guard: matches the committed file ->
    // Ok; differs -> Drift; missing -> Read. Deterministic codegen makes
    // the comparison exact.
    let bindings = Bindings::new().register::<FadersProcedures>();
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bindings.ts");
    bindings
        .export_to(&path)
        .expect("seed the committed client");

    bindings
        .check(&path)
        .expect("a freshly written client is current");

    // A different set renders differently -- that is drift.
    let drifted = Bindings::new().register::<MixerProcedures>();
    assert!(matches!(
        drifted.check(&path),
        Err(BindingsError::Drift { .. })
    ));

    // A missing file is a read error, not silent success.
    assert!(matches!(
        bindings.check(dir.path().join("absent.ts")),
        Err(BindingsError::Read { .. })
    ));
}

#[test]
fn check_guards_files_layout() {
    // The Files (multi-file) layout gets the same guard: check renders to a
    // temp tree and compares it against the committed directory.
    let bindings = Bindings::new()
        .layout(Layout::Files)
        .register::<ScenesProcedures>();
    let dir = tempfile::tempdir().expect("tempdir");
    bindings
        .export_to(dir.path())
        .expect("seed the committed tree");

    bindings
        .check(dir.path())
        .expect("a freshly written Files client is current");

    // A different set writes a different tree -- drift.
    let drifted = Bindings::new()
        .layout(Layout::Files)
        .register::<FadersProcedures>();
    assert!(matches!(
        drifted.check(dir.path()),
        Err(BindingsError::Drift { .. })
    ));

    // A missing directory is a read error.
    assert!(matches!(
        bindings.check(dir.path().join("absent")),
        Err(BindingsError::Read { .. })
    ));
}

#[test]
fn router_factory_is_the_taurpc_dropin() {
    let client = Bindings::new()
        .register::<FadersProcedures>()
        .register::<MixerProcedures>()
        .router("createTauRPCProxy")
        .export()
        .expect("render");

    // The factory groups every set object into one nested router, so a
    // migrating frontend's `const taurpc = createTauRPCProxy();
    // taurpc.faders.x()` keeps its call sites against ttipc's output.
    assert!(
        client.contains("export const createTauRPCProxy = () => ({\n  faders,\n  mixer,\n});"),
        "router factory missing or wrong:\n{client}"
    );

    // Off by default -- no factory unless asked for.
    let without = Bindings::new()
        .register::<FadersProcedures>()
        .export()
        .expect("render");
    assert!(!without.contains("createTauRPCProxy"));
}

#[test]
fn router_factory_exposes_events() {
    let client = Bindings::new()
        .register::<PostsProcedures>() // namespace "posts" (methods)
        .register_events::<PostsEvent>() // group "posts" (events) -> merged
        .register_events::<SignalEvent>() // group "signal" (events, no set)
        .router("createTauRPCProxy")
        .export()
        .expect("render");

    // A namespace with both methods and events merges them, so
    // taurpc.posts.list() and taurpc.posts.event.on(cb) both work.
    assert!(
        client.contains("posts: { ...posts, event: { on: events.posts.listen } }"),
        "got:\n{client}"
    );
    // A group with no matching method set still gets an `event.on`.
    assert!(
        client.contains("signal: { event: { on: events.signal.listen } }"),
        "got:\n{client}"
    );
}

#[test]
fn namespaced_sets_prefix_the_wire() {
    let client = Bindings::new()
        .register::<PostsProcedures>()
        .register::<TagsProcedures>()
        .router("createTauRPCProxy")
        .export()
        .expect("render");

    // The namespace names the object and prefixes the wire command, so the
    // shared method name `list` does not collide across the two sets.
    assert!(client.contains("export const posts = {"), "got:\n{client}");
    assert!(
        client.contains("return invoke(\"posts.list\")"),
        "got:\n{client}"
    );
    assert!(client.contains("export const tags = {"), "got:\n{client}");
    assert!(
        client.contains("return invoke(\"tags.list\")"),
        "got:\n{client}"
    );
    // The drop-in factory groups both namespaces.
    assert!(
        client.contains("export const createTauRPCProxy = () => ({\n  posts,\n  tags,\n});"),
        "got:\n{client}"
    );
}

#[test]
fn export_is_deterministic() {
    let a = Bindings::new()
        .register::<FadersProcedures>()
        .export()
        .expect("render a");
    let b = Bindings::new()
        .register::<FadersProcedures>()
        .export()
        .expect("render b");
    assert_eq!(a, b);
}

#[test]
fn files_layout_splits_by_module() {
    // Files writes a directory: the command object lands in index.ts with
    // a specta-computed import, and the submodule's `Scene` gets its own
    // file. The whole tree is concatenated (relative paths, sorted) into
    // one snapshot.
    let dir = tempfile::tempdir().expect("tempdir");
    Bindings::new()
        .layout(Layout::Files)
        .register::<ScenesProcedures>()
        .export_to(dir.path())
        .expect("files export");
    insta::assert_snapshot!(dir_to_string(dir.path()));
}

#[test]
fn router_factory_resolves_in_file_under_files_layout() {
    // The factory names the set objects and the `events` object. Under
    // Layout::Files specta splits the type definitions into submodules, but
    // the runtime body -- objects, events, and factory -- all lands in
    // index.ts together, so those references resolve in-file with no import.
    // (dx and lux both export single-file, so this locks the combination
    // rather than a path either target hits.)
    let dir = tempfile::tempdir().expect("tempdir");
    Bindings::new()
        .layout(Layout::Files)
        .register::<PostsProcedures>() // namespace "posts" (methods)
        .register_events::<PostsEvent>() // group "posts" (events) -> merged
        .router("createTauRPCProxy")
        .export_to(dir.path())
        .expect("files export");

    let index = std::fs::read_to_string(dir.path().join("index.ts")).expect("index.ts");
    // The factory and every name it references sit in the one file.
    assert!(
        index.contains("export const createTauRPCProxy = () => ({"),
        "factory missing from index.ts:\n{index}"
    );
    assert!(
        index.contains("posts: { ...posts, event: { on: events.posts.listen } }"),
        "factory's merged entry missing:\n{index}"
    );
    assert!(
        index.contains("export const posts = {"),
        "referenced set object not in index.ts:\n{index}"
    );
    assert!(
        index.contains("export const events = {"),
        "referenced events object not in index.ts:\n{index}"
    );
}

fn dir_to_string(root: &Path) -> String {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|(path, body)| format!("// === {path} ===\n{body}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .expect("path under root")
                .to_string_lossy()
                .replace('\\', "/");
            let body = std::fs::read_to_string(&path).expect("read file");
            out.push((rel, body));
        }
    }
}
