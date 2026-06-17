//! Microbenchmark: parsing a TauRPC procedures trait, rewriting it, and
//! re-emitting ttipc source.

use divan::black_box;
use ttipc_migrate::{transform, transform_project};

fn main() {
    divan::main();
}

const TRAIT: &str = r#"
#[taurpc::procedures(path = "greeter")]
pub trait Greeter {
    async fn greet<R: Runtime>(app_handle: AppHandle<R>, name: String) -> Result<String>;
    async fn stream<R: Runtime>(app_handle: AppHandle<R>, sink: Channel<Tick>) -> Result<()>;
    async fn focus<R: Runtime>(window: Window<R>) -> Result<()>;
}
"#;

#[divan::bench]
fn transform_trait() -> String {
    transform(black_box(TRAIT)).expect("valid Rust")
}

const RESOLVER: &str = r#"
#[taurpc::procedures(path = "greeter")]
pub trait Greeter {
    async fn greet<R: Runtime>(app_handle: AppHandle<R>, name: String) -> Result<String>;
}

#[taurpc::ipc_type]
pub struct Payload {
    text: String,
}

#[taurpc::ipc_type]
pub struct GreeterImpl;

#[taurpc::resolvers]
impl Greeter for GreeterImpl {
    #[instrument(skip_all, err)]
    async fn greet<R: Runtime>(self, app_handle: AppHandle<R>, name: String) -> Result<String> {
        Ok(format!("hi {name}"))
    }
}
"#;

#[divan::bench]
fn transform_resolver() -> String {
    transform(black_box(RESOLVER)).expect("valid Rust")
}

// BigInt-style integers on the wire (a `usize` arg and a `usize` struct field):
// flagged, since ttipc's exporter rejects them with no `BigIntExportBehavior`.
const BIGINT: &str = r#"
#[derive(serde::Serialize, specta::Type, Clone)]
pub struct Channel {
    pub channel_number: usize,
    pub label: String,
}

#[taurpc::procedures(path = "cmd")]
pub trait CmdMethods {
    async fn update<R: Runtime>(app_handle: AppHandle<R>, channel_number: usize) -> Result<u8, String>;
}
"#;

#[divan::bench]
fn transform_bigint() -> String {
    transform(black_box(BIGINT)).expect("valid Rust")
}

const DEASYNC: &str = r#"
#[taurpc::procedures(path = "calc")]
pub trait Calc {
    async fn add<R: Runtime>(app_handle: AppHandle<R>, a: u8, b: u8) -> u8;
    async fn load<R: Runtime>(app_handle: AppHandle<R>) -> Result<u8>;
}

#[taurpc::ipc_type]
pub struct CalcImpl;

#[taurpc::resolvers]
impl Calc for CalcImpl {
    async fn add<R: Runtime>(self, app_handle: AppHandle<R>, a: u8, b: u8) -> u8 {
        a + b
    }
    async fn load<R: Runtime>(self, app_handle: AppHandle<R>) -> Result<u8> {
        fetch(&app_handle).await
    }
}
"#;

#[divan::bench]
fn transform_deasync() -> String {
    transform(black_box(DEASYNC)).expect("valid Rust")
}

const TRANSITIVE: &str = r#"
#[taurpc::procedures(path = "store")]
pub trait Store {
    async fn load() -> u8;
    async fn ping() -> u8;
    async fn refresh() -> u8;
}

#[taurpc::ipc_type]
pub struct StoreImpl;

#[taurpc::resolvers]
impl Store for StoreImpl {
    async fn load(self) -> u8 {
        7
    }
    async fn ping(self) -> u8 {
        1
    }
    async fn refresh(self) -> u8 {
        StoreImpl.load().await
    }
}
"#;

#[divan::bench]
fn transform_deasync_transitive() -> String {
    transform(black_box(TRANSITIVE)).expect("valid Rust")
}

const EVENTS: &str = r#"
#[taurpc::procedures(path = "app", event_trigger = AppTrigger)]
pub trait App {
    #[taurpc(alias = "renamed_ping")]
    async fn ping() -> u8;

    #[taurpc(event)]
    async fn ready();

    #[taurpc(event)]
    async fn progress(percent: u8);

    #[taurpc(event)]
    async fn moved(x: i32, y: i32);
}
"#;

#[divan::bench]
fn transform_events() -> String {
    transform(black_box(EVENTS)).expect("valid Rust")
}

// The central-event pattern: one event method carrying an in-file payload enum,
// which gains `#[derive(ttipc::Event)]` while the emit site becomes
// `payload.emit(&h)`.
const PAYLOAD_EVENT: &str = r#"
pub enum AppEvent {
    AuthChanged(bool),
    Tick,
}

#[taurpc::procedures(path = "app", event_trigger = AppEventTrigger)]
pub trait AppMethods {
    #[taurpc(event)]
    async fn event(event: AppEvent);
    async fn ping() -> u8;
}

pub fn fire(h: AppHandle, e: AppEvent) {
    AppEventTrigger::new(h).event(e).unwrap();
}
"#;

#[divan::bench]
fn transform_payload_event() -> String {
    transform(black_box(PAYLOAD_EVENT)).expect("valid Rust")
}

const MOUNT: &str = r#"
pub fn run() {
    let router = taurpc::Router::new()
        .export_config(bindings())
        .merge(AppImpl.into_handler())
        .merge(LogImpl.into_handler());

    tauri::Builder::default()
        .manage(state())
        .invoke_handler(router.into_handler())
        .run(tauri::generate_context!())
        .expect("error while running");
}
"#;

#[divan::bench]
fn transform_mount() -> String {
    transform(black_box(MOUNT)).expect("valid Rust")
}

// A `build() -> Router<R>` factory with extra builder methods: the methods drop,
// the chain collapses, and the return type becomes `ttipc::Procedures`.
const ROUTER_FACTORY: &str = r#"
pub fn build<R: Runtime>() -> Router<R> {
    let typescript = config();
    taurpc::Router::new()
        .export_config(typescript)
        .semantic_types(semantic())
        .dangerously_cast_bigints_to_number()
        .merge(AppImpl.into_handler())
        .merge(LogImpl.into_handler())
}
"#;

#[divan::bench]
fn transform_router_factory() -> String {
    transform(black_box(ROUTER_FACTORY)).expect("valid Rust")
}

// A split mount across two files: a `build() -> Router` factory and a separate
// `build().into_handler()` consumer resolved through the project registry.
const FACTORY_FILE: &str = r#"
pub fn build<R: Runtime>() -> Router<R> {
    taurpc::Router::new().merge(AppImpl.into_handler())
}
"#;
const CONSUMER_FILE: &str = r#"
pub fn run() {
    let handler = crate::router::build().into_handler();
    tauri::Builder::default().invoke_handler(handler);
}
"#;

#[divan::bench]
fn transform_cross_file_consumer() -> Vec<(String, String)> {
    let files = [
        ("router.rs".to_string(), FACTORY_FILE.to_string()),
        ("lib.rs".to_string(), CONSUMER_FILE.to_string()),
    ];
    transform_project(black_box(&files)).expect("valid Rust")
}

const EMITS: &str = r#"
#[taurpc::procedures(path = "app", event_trigger = AppBus)]
pub trait App {
    #[taurpc(event)]
    async fn ready();
    #[taurpc(event)]
    async fn at(x: i32, y: i32);
}

pub fn fire(h: AppHandle) {
    AppBus::new(h.clone()).ready().unwrap();
    AppBus::new(h.clone()).at(1, 2).unwrap();
    AppBus::new(h).send_to(EventTarget::Any).at(3, 4).unwrap();
}
"#;

#[divan::bench]
fn transform_emits() -> String {
    transform(black_box(EMITS)).expect("valid Rust")
}

// The surgical multi-file path: a trait file plus an emit file whose trigger is
// declared in the other file (resolved through the project-wide registry).
const TRAIT_FILE: &str = r#"
#[taurpc::procedures(path = "cmd", event_trigger = CmdBus)]
pub trait Cmd {
    #[taurpc(event)]
    async fn updated(value: u8);
}
"#;

const EMIT_FILE: &str = r#"// domain logic
impl Store {
    fn touch(&self, app: AppHandle, value: u8) -> Result<(), String> {
        CmdBus::new(app).updated(value).map_err(|e| e.to_string())?;
        Ok(())
    }
}
"#;

#[divan::bench]
fn transform_project_surgical() -> Vec<(String, String)> {
    transform_project(black_box(&[
        ("cmd.rs".to_string(), TRAIT_FILE.to_string()),
        ("store.rs".to_string(), EMIT_FILE.to_string()),
    ]))
    .expect("valid Rust")
}
