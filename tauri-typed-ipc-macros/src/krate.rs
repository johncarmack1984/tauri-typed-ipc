//! Resolves the path to the `tauri-typed-ipc` crate for generated code, honoring
//! however the consumer imported it (`ttipc`, `tauri_typed_ipc`, or a custom
//! alias) via `proc-macro-crate`.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::Ident;

/// The crate ident generated code refers to.
pub(crate) fn ident() -> Ident {
    let name = match proc_macro_crate::crate_name("tauri-typed-ipc") {
        Ok(proc_macro_crate::FoundCrate::Name(name)) => name,
        // `Itself` (the crate's own tests/doctests, resolved through the
        // `extern crate self as tauri_typed_ipc` alias) and `Err` (an isolated
        // macro unit test) both fall back to the canonical name.
        _ => "tauri_typed_ipc".to_string(),
    };
    Ident::new(&name, Span::call_site())
}

/// The leading-`::` path to the crate, e.g. `::ttipc`.
pub(crate) fn rb() -> TokenStream {
    let ident = ident();
    quote!(::#ident)
}

/// The crate's re-exported serde, as a string for `#[serde(crate = "...")]`
/// (which takes a path string, so the resolved `rb()` tokens cannot be used).
pub(crate) fn private_serde() -> String {
    format!("::{}::__private::serde", ident())
}
