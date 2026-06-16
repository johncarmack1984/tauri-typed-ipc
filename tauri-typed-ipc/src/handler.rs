//! Mounting procedure sets on tauri's invoke pipeline.
//!
//! Wire parity is the design constraint here: a tauri_typed_ipc procedure is
//! invoked exactly like a raw `#[tauri::command]` -- same command
//! name, same named-args JSON body, no envelope -- so hand-written
//! `invoke()` calls, generated clients, and raw commands are
//! interchangeable on the wire, and benchmarks of the two compare the
//! same protocol.
//!
//! Note on stability: tauri documents [`Invoke`] as an unstable,
//! macro-facing API, but [`tauri::Builder::invoke_handler`] -- the
//! only public mount point for custom routing -- takes a closure over
//! it, so every IPC layer (TauRPC included) stands on this seam. tauri is
//! a caret dependency (`tauri = "2"`), so a 2.x release that reshapes
//! `Invoke` would surface here; the crate is tested against the current
//! tauri 2.x and bumps deliberately if the seam moves.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;
use tauri::Manager;
use tauri::ipc::{Invoke, InvokeBody, InvokeResolver, InvokeResponseBody, JavaScriptChannelId};

use crate::{Context, DispatchError};

/// A dispatched call's wire response: the value to resolve the invoke
/// with, or -- when the procedure returned `Err` -- the serialized error
/// to reject it with. This is distinct from [`DispatchError`]: that is
/// tauri_typed_ipc's own wire-level failure, rejected as a string, whereas an
/// [`Outcome::Reject`] carries the procedure's typed error, rejected
/// as-is so the client branches on it exactly as it would for a raw
/// `#[tauri::command]` returning `Result`.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Resolve the invoke with this value (the procedure returned `Ok`,
    /// or a plain value).
    Resolve(Value),
    /// Reject the invoke with this serialized error (the procedure
    /// returned `Err`).
    Reject(Value),
}

/// How a dispatched call is settled. A plain `fn` procedure settles
/// inline on the calling (main) thread as [`Sync`](Self::Sync); an
/// `async fn` hands back a [`Send`] future as [`Async`](Self::Async)
/// for [`handler`] to spawn on tauri's async runtime. The runtime type
/// `R` never appears here -- the future is `R`-free -- so dispatch
/// stays type-erased while `handler<R>` keeps `R` to itself.
pub enum Dispatch {
    /// Settled synchronously: the procedure's wire response, or a
    /// wire-level failure.
    Sync(Result<Outcome, DispatchError>),
    /// An `async fn`'s pending response, awaited after a spawn.
    Async(Pin<Box<dyn Future<Output = Result<Outcome, DispatchError>> + Send>>),
}

type DispatchFn = dyn Fn(&Context<'_>, &str, Value) -> Dispatch + Send + Sync;

/// A type-erased procedure set: the `(names, dispatch)` pair that
/// `#[procedures]` flattens a trait into, so routing needs no
/// knowledge of the trait itself. [`merge`](Self::merge) combines
/// several sets into one, so a whole app mounts on a single
/// [`handler`].
#[must_use]
pub struct Procedures {
    names: Box<[&'static str]>,
    dispatch: Box<DispatchFn>,
}

impl Procedures {
    /// Bundles a dispatch function with the procedure names it answers
    /// to. Generated `into_procedures` implementations call this; user
    /// code normally never does.
    ///
    /// The `Send + Sync` bound on `dispatch` is required by
    /// [`tauri::Builder::invoke_handler`] and is not negotiable: the
    /// procedure-set impl is captured here, so it must be `Sync`. That
    /// rules out `!Sync` state (`RefCell`, `Rc`) -- shared state interior-
    /// mutates behind a `Mutex` (an uncontended lock is ~ns). tauri itself
    /// escapes this with `unsafe impl Send/Sync`, which tauri-typed-ipc forbids
    /// (`unsafe_code = "forbid"`). See the [threading model][tm].
    ///
    /// [tm]: https://github.com/johncarmack1984/tauri-typed-ipc/blob/main/docs/tauri-threading.md
    #[doc(hidden)]
    pub fn new(
        names: &'static [&'static str],
        dispatch: impl Fn(&Context<'_>, &str, Value) -> Dispatch + Send + Sync + 'static,
    ) -> Self {
        Self {
            names: names.into(),
            dispatch: Box::new(dispatch),
        }
    }

    /// The procedure names this set answers to.
    #[must_use]
    pub fn names(&self) -> &[&'static str] {
        &self.names
    }

    /// Combines two procedure sets into one that answers to the names
    /// of both. This is tauri_typed_ipc's composition story: each trait keeps
    /// its own `#[procedures]` impl, and an app with several of them
    /// mounts a single
    /// `a.into_procedures().merge(b.into_procedures())` on one
    /// [`handler`] -- no router type to build and no `into_handler`
    /// step, unlike the merge-into-a-router pattern this replaces.
    ///
    /// A name defined by both sets is a registration mistake with no
    /// sensible resolution, so it panics here -- at startup, naming the
    /// command -- rather than silently shadowing one with the other.
    pub fn merge(self, other: Procedures) -> Procedures {
        let Procedures {
            names: a_names,
            dispatch: a,
        } = self;
        let Procedures {
            names: b_names,
            dispatch: b,
        } = other;

        for name in b_names.iter() {
            assert!(
                !a_names.contains(name),
                "tauri_typed_ipc: procedure {name:?} is registered by more than one merged set",
            );
        }

        let names = a_names.iter().chain(b_names.iter()).copied().collect();
        let dispatch = move |ctx: &Context<'_>, procedure: &str, args: Value| {
            if a_names.contains(&procedure) {
                a(ctx, procedure, args)
            } else {
                b(ctx, procedure, args)
            }
        };
        Procedures {
            names,
            dispatch: Box::new(dispatch),
        }
    }

    /// Routes one call to the named procedure.
    pub fn dispatch(&self, ctx: &Context<'_>, procedure: &str, args: Value) -> Dispatch {
        (self.dispatch)(ctx, procedure, args)
    }
}

/// Builds a tauri invoke handler from a procedure set.
///
/// Commands outside the set are reported unhandled, so tauri answers
/// `Command {name} not found` -- byte-identical to its raw-command
/// behavior.
pub fn handler<R: tauri::Runtime>(
    procedures: Procedures,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    handler_with_fallback(procedures, |_| false)
}

/// Like [`handler`], but commands outside the set fall through to
/// `fallback` -- typically `tauri::generate_handler![...]` -- so raw
/// commands and procedures coexist while an app migrates.
pub fn handler_with_fallback<R, F>(
    procedures: Procedures,
    fallback: F,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static
where
    R: tauri::Runtime,
    F: Fn(Invoke<R>) -> bool + Send + Sync + 'static,
{
    move |invoke| {
        if !procedures.names().contains(&invoke.message.command()) {
            return fallback(invoke);
        }

        let Invoke {
            message, resolver, ..
        } = invoke;
        let args = match message.payload() {
            InvokeBody::Json(value) => value.clone(),
            InvokeBody::Raw(_) => {
                resolver.reject(DispatchError::RawBody.to_string());
                return true;
            }
        };

        // What this call may have injected, matched by concrete type
        // (so the AppHandle is the runtime's own, mock included).
        // `state_ref` is the runtime-free StateManager, so carrying it
        // inward keeps the dispatch path off the `R` parameter.
        let webview = message.webview();
        let injectable: [&(dyn Any + Send + Sync); 1] = [webview.app_handle()];
        // Build `Channel<T>` parameters from the webview -- the one
        // `R`-typed step. `channel_on` returns an `R`-free channel (the
        // runtime is captured inside its `Arc`'d closure), so dispatch
        // stays off `R`. This factory is only borrowed for the dispatch
        // call below; async procedures build their channels in the
        // synchronous prelude and own them into the spawned future.
        let make_channel = {
            let webview = webview.clone();
            move |id: JavaScriptChannelId| id.channel_on::<R, InvokeResponseBody>(webview.clone())
        };
        let ctx = Context::new(&injectable)
            .with_state(message.state_ref())
            .with_channels(&make_channel);

        match procedures.dispatch(&ctx, message.command(), args) {
            Dispatch::Sync(result) => settle(resolver, result),
            Dispatch::Async(future) => {
                // The future is `R`-free and `Send`, so it spawns on
                // tauri's runtime while the `InvokeResolver<R>` rides
                // along to settle the response off the main thread. If the
                // future panics, the task unwinds before settling, so the
                // invoke neither resolves nor rejects (the JS promise stays
                // pending) -- the same outcome as a panicking raw async
                // command.
                tauri::async_runtime::spawn(async move {
                    settle(resolver, future.await);
                });
            }
        }
        true
    }
}

/// Resolves or rejects an invoke from a dispatched call's outcome. A
/// procedure's typed `Err` rejects with the serialized error; a
/// wire-level [`DispatchError`] rejects with its string form.
fn settle<R: tauri::Runtime>(resolver: InvokeResolver<R>, result: Result<Outcome, DispatchError>) {
    match result {
        Ok(Outcome::Resolve(value)) => resolver.resolve(value),
        Ok(Outcome::Reject(value)) => resolver.reject(value),
        Err(err) => resolver.reject(err.to_string()),
    }
}
