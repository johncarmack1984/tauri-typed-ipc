//! Procedural macros for tauri-typed-ipc. Use through the `tauri-typed-ipc` crate.

mod error;
mod event;
mod krate;
mod procedures;

use proc_macro::TokenStream;

/// Lower-camel a name: first character lowercased, the rest kept.
/// `Faders` -> `faders`, `VolumeUp` -> `volumeUp`.
pub(crate) fn lower_camel(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => name.to_string(),
    }
}

/// Marks a trait as a tauri-typed-ipc procedure set.
///
/// Emits the trait (with a `Send` bound added to any `async fn`'s
/// returned future) plus a `{Trait}Dispatch` extension trait whose
/// `dispatch` method routes a `(name, args)` pair to the matching
/// procedure -- the seam every later layer (Tauri plugin glue, typed
/// bindings) builds on, and the seam unit tests call directly -- and a
/// `{Trait}Procedures` descriptor the TypeScript generator reads.
#[proc_macro_attribute]
pub fn procedures(attr: TokenStream, item: TokenStream) -> TokenStream {
    procedures::expand(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derives a typed event channel for an enum: each variant is an event,
/// emitted with `Variant { .. }.emit(&app)` under the wire name
/// `"{group}:{variant}"` with the variant's fields as the payload. The
/// derive owns the name on the Rust side; the bindings generator owns
/// the matching listener on the TS side (a later slice).
#[proc_macro_derive(Event)]
pub fn event(input: TokenStream) -> TokenStream {
    event::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derives a wire-serializable error for an enum: each variant serializes
/// to `{ type: "<variant>", message: "<Display>" }`, a discriminated union
/// the client can branch on. Display-based, so it works with non-Serialize
/// sources like `std::io::Error`.
#[proc_macro_derive(Error)]
pub fn error(input: TokenStream) -> TokenStream {
    error::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
