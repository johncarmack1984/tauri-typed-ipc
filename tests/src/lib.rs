//! Shared fixtures for ttipc's behavioral tests and benches.

use tauri::AppHandle;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, MockRuntime, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use ttipc::procedures;

/// The walking-skeleton exemplar: one plain wire procedure, one with
/// an injected `AppHandle`. Injection matches by concrete type, so the
/// fixture names the mock runtime's handle type explicitly.
#[procedures]
pub trait Greeter {
    fn greet(&self, name: String) -> String;
    fn greet_via(&self, app: AppHandle<MockRuntime>, name: String) -> String;
}

/// A numeric procedure set for the binding-generation tests: a
/// unit-returning setter with an injected handle, a numeric getter, and
/// a string-returning labeler -- enough to cover `null`, `number`, and
/// `string` rendering plus injection-stripping. No impl needed; the
/// bindings are read off the generated `FadersProcedures` descriptor.
#[procedures]
pub trait Faders {
    fn set(&self, app: AppHandle<MockRuntime>, channel: u16, value: u8);
    fn level(&self, channel: u16) -> u8;
    fn label(&self, channel: u16, name: String) -> String;
}

// Multi-word method and argument names for the client-casing tests:
// the wire command and arg keys stay the snake_case Rust idents, only
// the TypeScript identifiers get cased. No impl needed -- the bindings
// read the generated descriptor.
#[procedures]
pub trait Mixer {
    fn set_channel(&self, channel_number: u16, new_value: u8);
}

// A custom wire type passed both ways by `Scenes`: proves named types
// reach the client as `export type` definitions and are referenced (not
// inlined) in command signatures, that Rust doc comments flow to TS,
// and -- living in a submodule -- that the Files layout gives it its
// own file.
pub mod scene {
    /// A saved lighting scene: a name and one level per channel.
    #[derive(serde::Serialize, serde::Deserialize, specta::Type)]
    pub struct Scene {
        pub name: String,
        pub levels: Vec<u8>,
    }
}

#[procedures]
pub trait Scenes {
    fn save(&self, scene: scene::Scene);
    fn load(&self, name: String) -> scene::Scene;
}

/// A typed event channel for the rig: `Changed { .. }.emit(&app)` emits
/// "fader:changed" with the fields as the payload. Drives the Event
/// derive's emit tests; the listener half becomes generated bindings.
#[derive(ttipc::Event)]
pub enum FaderEvent {
    Changed { channel: u16, value: u8 },
}

/// Single-payload events: a tuple variant `V(T)` carries an arbitrary `T`
/// (primitive, collection, ...) rendered as `{ type, data: T }` -- the
/// adjacently-tagged shape taurpc apps use. Mixed with unit and named
/// variants to prove all three coexist.
#[derive(ttipc::Event)]
pub enum SignalEvent {
    Ready,
    Progress { done: u32 },
    Code(u32),
    Batch(Vec<u32>),
}

/// An event group whose name matches the [`Posts`] namespace, so the
/// router factory merges them into one object: `posts.list()` plus
/// `posts.event.on(cb)` (the taurpc shape).
#[derive(ttipc::Event)]
pub enum PostsEvent {
    Updated { id: u32 },
}

/// A wire error for the Error derive tests: a tuple variant and a unit
/// variant, each serializing to `{ type, message }`.
#[derive(Debug, thiserror::Error, ttipc::Error)]
pub enum DeskError {
    #[error("channel {0} is out of range")]
    OutOfRange(u16),
    #[error("the desk is locked")]
    Locked,
}

/// A descriptor-only set for the typed-error binding tests: a fallible
/// procedure rejecting [`DeskError`], so the client gets a discriminated
/// union to type its catch (and an infallible one alongside, which gets
/// no `@throws`). No impl needed -- the bindings read the descriptor.
#[procedures]
pub trait Guarded {
    fn store(&self, channel: u16, value: u8) -> Result<(), DeskError>;
    fn peek(&self, channel: u16) -> u8;
}

/// The taurpc drop-in: a `Result<_, String>` procedure. This trait compiling at
/// all proves `String: ErrorSet` (the macro requires it of every `Result` error);
/// `String` rejects as a bare string on the wire, and the client types the catch
/// as the built-in `string`. The impl drives the dispatch wire-parity test.
#[procedures]
pub trait Notes {
    fn save(&self, note: String) -> Result<u8, String>;
    fn count(&self) -> u8;
}

pub struct NotesImpl;

impl Notes for NotesImpl {
    fn save(&self, note: String) -> Result<u8, String> {
        if note.is_empty() {
            return Err("note is empty".to_string());
        }
        u8::try_from(note.len()).map_err(|_| "note too long".to_string())
    }

    fn count(&self) -> u8 {
        0
    }
}

/// Unit state standing in for a real procedure set owner.
pub struct App;

impl Greeter for App {
    fn greet(&self, name: String) -> String {
        format!("Hello, {name}!")
    }

    fn greet_via(&self, _app: AppHandle<MockRuntime>, name: String) -> String {
        format!("Hello via app, {name}!")
    }
}

/// A second procedure set for the composition tests and bench. Its
/// names are disjoint from [`Greeter`], so the two merge cleanly, and
/// it lives on its own state so each owner implements exactly one
/// trait (no `into_procedures` ambiguity).
#[procedures]
pub trait Counter {
    fn count(&self, n: u32) -> u32;
}

/// Owner of the [`Counter`] set.
pub struct Tally;

impl Counter for Tally {
    fn count(&self, n: u32) -> u32 {
        n + 1
    }
}

/// Two namespaced sets that share the method name `list`. Without a
/// namespace the bare wire commands collide; with
/// `#[procedures(namespace = ...)]` each becomes `posts.list` /
/// `tags.list`, so they coexist and route by namespace.
#[procedures(namespace = "posts")]
pub trait Posts {
    fn list(&self) -> u32;
}

/// Owner of the [`Posts`] set.
pub struct PostStore;

impl Posts for PostStore {
    fn list(&self) -> u32 {
        1
    }
}

// `path` is taurpc's spelling, an accepted alias for `namespace`.
#[procedures(path = "tags")]
pub trait Tags {
    fn list(&self) -> u32;
}

/// Owner of the [`Tags`] set.
pub struct TagStore;

impl Tags for TagStore {
    fn list(&self) -> u32 {
        2
    }
}

/// Managed state for the `State<T>` injection tests and bench: a value
/// the procedure reads through `tauri::State`, proving injection
/// resolves from the app's runtime-free StateManager, not from the
/// set's own `&self`.
pub struct Hits(pub u32);

/// A procedure set whose one procedure takes managed state by
/// injection. Its owner is distinct from the others so each state
/// implements exactly one trait.
#[procedures]
pub trait Counted {
    fn hits(&self, state: tauri::State<'_, Hits>) -> u32;
}

/// Owner of the [`Counted`] set.
pub struct Service;

impl Counted for Service {
    fn hits(&self, state: tauri::State<'_, Hits>) -> u32 {
        state.0
    }
}

/// An async procedure set for the R3 async-dispatch tests: a plain async
/// getter that resolves, and an async fallible one that rejects with a
/// typed error on `Err`. An async procedure forces the `Arc<Self>`
/// receiver and the spawn path; the `Result` arm drives `Outcome::Reject`.
#[procedures]
pub trait Backup {
    async fn snapshot(&self, label: String) -> String;
    async fn try_snapshot(&self, label: String) -> Result<String, BackupError>;
}

/// Owner of the [`Backup`] set.
pub struct Vault;

impl Backup for Vault {
    async fn snapshot(&self, label: String) -> String {
        format!("snapshot: {label}")
    }

    async fn try_snapshot(&self, label: String) -> Result<String, BackupError> {
        if label.is_empty() {
            Err(BackupError::Empty)
        } else {
            Ok(format!("ok: {label}"))
        }
    }
}

/// A wire error for the async `Result` dispatch tests, serializing to
/// `{ type, message }` like any [`ttipc::Error`].
#[derive(Debug, thiserror::Error, ttipc::Error)]
pub enum BackupError {
    #[error("the label is empty")]
    Empty,
}

/// A streamed value for the channel dispatch tests: the payload a
/// `Channel<Progress>` carries back to the caller.
#[derive(serde::Serialize, specta::Type)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

/// A streaming procedure set for the R3 channel tests: each procedure
/// takes a wire argument and a `Channel<Progress>`, proving the two
/// coexist in the generated `Args` (the channel id rides the wire, the
/// typed channel is built from the context). The sync arm builds it
/// inline; the async arm builds it in the prelude and owns it into the
/// spawned future.
#[procedures]
pub trait Downloads {
    fn track(&self, total: u32, progress: ttipc::Channel<Progress>);
    async fn track_async(&self, total: u32, progress: ttipc::Channel<Progress>);
}

/// A serde-asymmetric wire type for the phase-boundary test: a one-sided
/// rename gives the field a different *key* on each side, which specta's
/// unified `Format` cannot represent in one type. Exercises ttipc's 0.1
/// boundary -- such a type errors loudly at export rather than rendering
/// a wrong shape; phase-aware rendering is post-0.1.
///
/// `skip_serializing_if` used to sit here, but specta represents that one
/// soundly now (`note?: string | null` covers both phases: serialize omits
/// the key, deserialize accepts it absent, null, or set). Only a shape with
/// no honest union left -- like this key disagreement -- still errors.
#[derive(serde::Serialize, serde::Deserialize, specta::Type)]
pub struct Patch {
    #[serde(rename(serialize = "patchNote"))]
    pub note: String,
}

/// A descriptor-only set using the asymmetric [`Patch`] both ways, so
/// exporting its bindings hits the unified-`Format` limitation. No impl
/// needed -- the binding test only reads the descriptor.
#[procedures]
pub trait Patches {
    fn apply(&self, patch: Patch) -> Patch;
}

/// A descriptor-only set using BigInt-style integers (`u64`, `i64`),
/// which specta forbids exporting by default to avoid precision loss past
/// 2^53. Exercises ttipc's 0.1 boundary -- such a procedure errors
/// loudly at export. Lossless transport needs an upstream tauri invoke
/// reviver hook (responses are parsed with `response.json()`, no hook), so
/// it is post-0.1.
#[procedures]
pub trait Ledger {
    fn balance(&self, account: u64) -> i64;
}

/// Owner of the [`Downloads`] set.
pub struct Downloader;

impl Downloads for Downloader {
    fn track(&self, total: u32, progress: ttipc::Channel<Progress>) {
        for done in 1..=total {
            progress
                .send(Progress { done, total })
                .expect("channel send");
        }
    }

    async fn track_async(&self, total: u32, progress: ttipc::Channel<Progress>) {
        for done in 1..=total {
            progress
                .send(Progress { done, total })
                .expect("channel send");
        }
    }
}

/// A ready-to-send invoke request, shaped exactly as the webview
/// shapes one for a raw command (the wire-parity contract).
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

/// Like [`mock_webview`], but with one value managed in app state, so
/// `State<T>` injection has something to resolve.
pub fn mock_webview_managed<S, F>(
    state: S,
    handler: F,
) -> (tauri::App<MockRuntime>, tauri::WebviewWindow<MockRuntime>)
where
    S: Send + Sync + 'static,
    F: Fn(tauri::ipc::Invoke<MockRuntime>) -> bool + Send + Sync + 'static,
{
    let app = tauri::test::mock_builder()
        .manage(state)
        .invoke_handler(handler)
        .build(mock_context(noop_assets()))
        .expect("mock app builds");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview builds");
    (app, webview)
}
