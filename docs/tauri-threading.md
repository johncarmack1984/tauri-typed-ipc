# Where Tauri commands actually run

The receipts behind tauri-typed-ipc's sync-by-default design. Every claim below
is cited into pinned source: tauri `tauri-v2.11.2`, wry `wry-v0.55.0`
(the wry version tauri-runtime-wry 2.11.2 depends on). File paths are
crate-relative; line numbers are from those tags.

## Summary

| You write                       | Macro context | Body executes on                            | Bounds imposed        |
|---------------------------------|---------------|---------------------------------------------|-----------------------|
| `fn` + `#[tauri::command]`      | Blocking      | the event-loop (main) thread, inline in IPC delivery | none beyond `'static` |
| `async fn` + `#[tauri::command]`| Async         | tauri's tokio runtime worker                | future `Send + 'static` |
| `fn` + `#[tauri::command(async)]` | Async ("sync_threadpool") | tokio runtime worker, sync body inside an async block | args/return `Send`; blocks a worker |

Two consequences define tauri-typed-ipc:

1. Blocking commands pay no `Send` tax and run on the same thread that
   ran `tauri::Builder::run` setup. So *accessing* `!Send` state (`Rc`,
   `RefCell`) from a blocking command is sound -- though tauri-typed-ipc cannot
   register such state anyway (see "What this licenses").
2. The price is honest: a slow blocking command stalls the UI *and* all
   other IPC, because delivery is serialized on that same thread.

## The macro split

`#[tauri::command]` chooses between two codegen bodies
(tauri-macros/src/command/wrapper.rs:248-253). An `async fn` forces the
Async context (wrapper.rs:158-160); `#[tauri::command(async)]` on a
non-async fn also selects Async, which the macro itself labels
`"sync_threadpool"` in its tracing kind (wrapper.rs:263-267).

- Blocking body: the user fn is called inline -- `let result =
  $path(...)` -- then resolved synchronously (wrapper.rs:429-435).
  No spawn, no hop: it runs wherever the invoke pipeline runs.
- Async body: the call is wrapped in an `async move` block handed to
  `resolver.respond_async_serialized(...)` (wrapper.rs:388-395).

`respond_async_serialized` requires `F: Future + Send + 'static` and
runs it via `crate::async_runtime::spawn` (tauri/src/ipc/mod.rs:343-388,
spawn at :375). That signature is the origin of the `Send + 'static`
tax on everything an async command touches.

Note the `(async)` attribute footgun: the sync body runs *inside the
async block on a runtime worker* -- there is no `spawn_blocking` -- so
long CPU work in that mode starves the async runtime. tauri-typed-ipc should
not reproduce this mode silently.

## The delivery chain has no thread hops

From IPC arrival to user code, every step is a plain call:

1. The `ipc` custom-protocol closure (tauri/src/ipc/protocol.rs:35)
   calls `handle_ipc_message` (protocol.rs:185).
2. Which calls `Webview::on_message` (tauri/src/webview/mod.rs:1742) --
   ACL checks inline -- then `manager.run_invoke_handler(invoke)`
   (webview/mod.rs:1909, manager/mod.rs:471).
3. Which calls the generated wrapper, i.e. the Blocking body above.

So a blocking command executes on whatever thread the platform webview
delivers IPC on.

## Which thread delivers IPC: a structural guarantee on desktop

Tauri registers its handlers with wry
(tauri-runtime-wry/src/lib.rs:5163 for the postMessage `ipc_handler`,
:5197 for the custom protocol). wry stores both as boxes WITHOUT `Send`:

- `custom_protocols: HashMap<String, Box<dyn Fn(...)>>`
  (wry/src/lib.rs:646-648)
- `ipc_handler: Option<Box<dyn Fn(Request<String>)>>`
  (wry/src/lib.rs:651)

A `Box<dyn Fn>` that is not `Send` cannot legally leave the thread that
owns it. The webview is built on the event-loop thread, so on the
desktop backends these closures can only ever be invoked there. This is
stronger than observed behavior -- it is enforced by the type system.

Platform call sites, for completeness:

- macOS/iOS: the handler is called inline from the
  `WKURLSchemeHandler` start method
  (wry/src/wkwebview/class/url_scheme_handler.rs:57, call at :322);
  WebKit delivers scheme-task callbacks on the main thread.
- Windows: `attach_custom_protocol_handler` registers a
  `WebResourceRequested` COM event (wry/src/webview2/mod.rs:921, :955);
  WebView2 fires events on the thread that created the environment --
  the event-loop thread.
- Linux: the handler is registered as a webkit2gtk URI scheme callback
  (wry/src/webkitgtk/web_context.rs:144), dispatched by the glib main
  context on the main thread.

**Android is the exception**: wry wraps handlers in `unsafe impl Send`
shims (`UnsafeIpc`, wry/src/android/mod.rs:66-72) and routes through a
pipe to JNI, so the desktop guarantee does not transfer. tauri-typed-ipc's
`!Send`-state story is desktop-first; Android needs its own audit
before any claim is made.

## What this licenses, and what it does not

- *Accessing* `RefCell`/`Rc` state from a blocking command is sound on
  desktop: setup (`tauri::Builder::run`), command execution, and event
  emission all share the event-loop thread.
- But tauri-typed-ipc cannot REGISTER `!Sync` state. `tauri::Builder::
  invoke_handler` requires `Fn(Invoke<R>) -> bool + Send + Sync +
  'static` (app.rs:1652), so the captured procedure-set impl must be
  `Sync` -- which `RefCell` is not. tauri escapes this for its own
  window map with `unsafe impl Send/Sync`; tauri-typed-ipc forbids `unsafe`
  (`unsafe_code = "deny"`), so shared state interior-mutates behind a
  `Mutex` (uncontended, ~ns). The faders example ships `Mutex<[u8;512]>`.
  Sync-first's payoff is the *inline dispatch* above (no spawn, no
  executor, no `Send`-coloring of command logic), NOT `RefCell` vs
  `Mutex`. A compile-fail test pins this: tests/compile_fail/
  refcell_state.rs.
- For an `async` procedure, the future is `Send + 'static` and `&self`
  capture forces the impl `Send + Sync` -- the same `Mutex` answer, so
  there is no extra tension. tauri-typed-ipc does NOT add a `ctx.sync(|| ...)`
  main-thread hop over tauri's `run_on_main_thread`: it would not lift
  the registration bound above, and it is exactly the re-entrant
  main-thread pattern behind tauri's `RefCell`-window-map panic (the
  upstream scar, fixed only on tauri dev as of 2026-06; tauri-typed-ipc targets
  stock tauri). Sync stays the safe path; async opt-in matches a raw
  tauri async command, no worse.

## Sources

- https://github.com/tauri-apps/tauri/tree/tauri-v2.11.2
- https://github.com/tauri-apps/wry/tree/wry-v0.55.0
