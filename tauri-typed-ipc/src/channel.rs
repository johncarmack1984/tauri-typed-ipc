//! Streaming responses over a typed channel.
//!
//! A procedure that takes a [`Channel<T>`] streams zero or more `T`
//! values back to the caller, on top of (and independent of) the single
//! value it eventually resolves with. This mirrors
//! [`tauri::ipc::Channel<T>`] but is built through tauri_typed_ipc's
//! runtime-erased dispatch: the handler constructs the underlying tauri
//! channel from the webview -- the one `R`-typed step -- and hands a
//! procedure this wrapper, which carries no `R`. Because the wrapper
//! owns its channel (rather than borrowing the dispatch [`Context`]), it
//! can also cross into an `async fn`'s spawned future, unlike the
//! borrowed `AppHandle`/`State` injections.
//!
//! [`Context`]: crate::Context

use std::marker::PhantomData;

use tauri::ipc::{Channel as TauriChannel, InvokeResponseBody};

/// A typed, one-way channel a procedure streams values through.
///
/// Obtained by declaring a `Channel<T>` parameter on a `#[procedures]`
/// method; the client passes the matching JavaScript `Channel` and
/// receives every [`send`](Self::send).
pub struct Channel<T> {
    inner: TauriChannel<InvokeResponseBody>,
    // `fn(T)` rather than `T`: the channel only ever sends `T`, never
    // holds one, so this stays `Send + Sync` whatever `T` is.
    marker: PhantomData<fn(T)>,
}

impl<T> Channel<T> {
    /// Wraps the tauri channel the handler built from the webview.
    /// Crate-internal: a procedure receives a `Channel<T>`, it never
    /// constructs one.
    pub(crate) fn from_tauri(inner: TauriChannel<InvokeResponseBody>) -> Self {
        Self {
            inner,
            marker: PhantomData,
        }
    }

    /// The channel identifier, matching the JavaScript `Channel` that
    /// receives the messages.
    pub fn id(&self) -> u32 {
        self.inner.id()
    }
}

impl<T: serde::Serialize> Channel<T> {
    /// Streams one value to the caller. Non-blocking: it hands the
    /// serialized value to tauri for delivery and returns. The value is
    /// serialized exactly as a command return would be -- a JSON body --
    /// so this is byte-identical to `tauri::ipc::Channel::<T>::send`.
    pub fn send(&self, value: T) -> tauri::Result<()> {
        let json = serde_json::to_string(&value)?;
        self.inner.send(InvokeResponseBody::Json(json))
    }
}
