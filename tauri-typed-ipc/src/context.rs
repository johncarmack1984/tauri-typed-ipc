//! Type-matched parameter injection.
//!
//! A [`Context`] is the small set of values the handler offers to one
//! dispatch call; generated code pulls injected parameters out of it
//! by concrete type (`TypeId`), never by parameter name -- a renamed
//! parameter cannot silently change behavior, and a type that is not
//! on offer fails loudly as [`DispatchError::MissingInjection`].
//!
//! Managed state ([`tauri::State`]) is offered the same way but
//! resolved through tauri's [`StateManager`] -- a runtime-free
//! `TypeId` map -- so the whole dispatch path stays free of the
//! `R: Runtime` parameter even while injecting `State<T>`. The handler
//! is the one place that touches `R`; what it hands inward does not.
//!
//! [`DispatchError::MissingInjection`]: crate::DispatchError::MissingInjection

use std::any::Any;

use tauri::ipc::{Channel as TauriChannel, JavaScriptChannelId};
use tauri::{State, StateManager};

/// Builds the runtime-erased tauri channel for a JavaScript channel id.
/// The handler installs one closed over the webview (the one `R`-typed
/// step); the resulting channel carries no `R`, so dispatch stays off
/// the runtime parameter. `Send + Sync` keeps [`Context`] shareable
/// across threads, as it is without channels.
type ChannelFactory<'a> = dyn Fn(JavaScriptChannelId) -> TauriChannel + Send + Sync + 'a;

/// Injectable values for one dispatch call, borrowed from the caller.
/// Procedures without injected parameters never look at it.
///
/// The handler builds and fills this; generated `dispatch` reads it. It is
/// not part of the hand-written surface, so it is hidden from the docs.
#[doc(hidden)]
pub struct Context<'a> {
    values: &'a [&'a (dyn Any + Send + Sync)],
    state: Option<&'a StateManager>,
    channels: Option<&'a ChannelFactory<'a>>,
}

impl<'a> Context<'a> {
    /// A context offering the given values, matched by concrete type.
    pub fn new(values: &'a [&'a (dyn Any + Send + Sync)]) -> Self {
        Self {
            values,
            state: None,
            channels: None,
        }
    }

    /// A context offering nothing.
    pub fn empty() -> Context<'static> {
        Context {
            values: &[],
            state: None,
            channels: None,
        }
    }

    /// Adds the app's managed state to what this context can resolve,
    /// so `State<T>` parameters reach it. The handler supplies this
    /// from the invoke message.
    pub fn with_state(mut self, state: &'a StateManager) -> Self {
        self.state = Some(state);
        self
    }

    /// Adds the channel factory the handler builds from the webview, so
    /// `Channel<T>` parameters can be constructed. Each call deserializes
    /// a channel id off the wire and asks this factory for the channel.
    pub fn with_channels(mut self, channels: &'a ChannelFactory<'a>) -> Self {
        self.channels = Some(channels);
        self
    }

    /// The first offered value of type `T`, cloned out.
    pub fn extract<T: Clone + 'static>(&self) -> Option<T> {
        self.values
            .iter()
            .find_map(|value| value.downcast_ref::<T>())
            .cloned()
    }

    /// The managed value of type `T`, if one is managed and this
    /// context carries the state to look in.
    pub fn state<T: Send + Sync + 'static>(&self) -> Option<State<'a, T>> {
        self.state.and_then(|state| state.try_get::<T>())
    }

    /// The typed channel for the given id, if this context carries a
    /// channel factory. `T` is the value type the channel sends; it is
    /// phantom on the underlying channel, so the factory builds one
    /// concrete channel and this only tags it.
    pub fn channel<T>(&self, id: JavaScriptChannelId) -> Option<crate::Channel<T>> {
        self.channels
            .map(|make| crate::Channel::from_tauri(make(id)))
    }
}
