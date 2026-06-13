//! Expansion for `#[derive(Event)]`.
//!
//! A typed event channel: each enum variant is an event, emitted under
//! the wire name `"{group}:{variant}"` with the variant's fields as the
//! payload. `group` is the enum name with a trailing `Event` stripped,
//! lower-camel; `variant` is the variant lower-camel. The derive owns
//! the wire name on both ends -- `emit` on the Rust side, and an
//! [`EventSet`](tauri-typed-ipc) descriptor the bindings generator turns into a
//! `listen` client. Struct and unit variants today; tuple variants are
//! rejected (their fields have no names for the payload).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, Result, Variant};

use crate::lower_camel;

/// Path every generated reference to the tauri-typed-ipc crate goes through.
fn private() -> TokenStream {
    let rb = crate::krate::rb();
    quote!(#rb::__private)
}

/// One event variant, parsed once and shared by `emit` and the
/// [`EventSet`] descriptor.
struct EventVariant {
    ident: syn::Ident,
    /// Discriminant: the TS `type` tag and the wire-name suffix.
    discriminant: String,
    /// Wire name, `"{group}:{variant}"`.
    wire_name: String,
    /// Payload fields in declaration order (empty for unit and single
    /// tuple-payload variants).
    fields: Vec<(syn::Ident, syn::Type)>,
    /// A single unnamed tuple payload (`V(T)`): emitted as the value and
    /// rendered as `data: T`. `None` for unit and named-field variants.
    data: Option<syn::Type>,
}

pub fn expand(input: TokenStream) -> Result<TokenStream> {
    let rb = crate::krate::rb();
    let input: DeriveInput = syn::parse2(input)?;

    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &input.generics,
            "generic event enums are not supported",
        ));
    }

    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(
            &input,
            "Event can only be derived for enums",
        ));
    };

    let enum_ident = &input.ident;
    let name = enum_ident.to_string();
    let group = event_group(enum_ident);

    let mut variants = Vec::new();
    for variant in &data.variants {
        variants.push(parse_variant(&group, variant)?);
    }

    let emit = quote!(emit);
    let emit_to = quote!(emit_to);
    let no_target = quote!();
    let one_target = quote!(target,);
    let emit_arms = variants
        .iter()
        .map(|variant| emit_arm(enum_ident, variant, &emit, &no_target));
    let emit_to_arms = variants
        .iter()
        .map(|variant| emit_arm(enum_ident, variant, &emit_to, &one_target));
    let event_types = variants.iter().map(event_type);
    let private = private();

    Ok(quote! {
        impl #enum_ident {
            /// Emit this event to all targets through the given emitter
            /// (an `AppHandle`, window, or webview). The wire name is the
            /// derive's; the payload is the variant's fields.
            pub fn emit<R, E>(&self, emitter: &E) -> #private::tauri::Result<()>
            where
                R: #private::tauri::Runtime,
                E: #private::tauri::Emitter<R>,
            {
                match self {
                    #(#emit_arms)*
                }
            }

            /// Emit this event to a single `target` (a window or webview
            /// label, or an `EventTarget`) through the given emitter, so
            /// only listeners on that target receive it. Same wire name
            /// and payload as [`Self::emit`].
            pub fn emit_to<R, E, I>(&self, emitter: &E, target: I) -> #private::tauri::Result<()>
            where
                R: #private::tauri::Runtime,
                E: #private::tauri::Emitter<R>,
                I: ::core::convert::Into<#private::tauri::EventTarget>,
            {
                match self {
                    #(#emit_to_arms)*
                }
            }
        }

        impl #rb::EventSet for #enum_ident {
            const GROUP: &'static str = #group;
            const NAME: &'static str = #name;

            fn events(
                types: &mut #private::specta::Types,
            ) -> ::std::vec::Vec<#rb::EventType> {
                ::std::vec![#(#event_types,)*]
            }
        }
    })
}

fn parse_variant(group: &str, variant: &Variant) -> Result<EventVariant> {
    let discriminant = lower_camel(&variant.ident.to_string());
    let wire_name = format!("{group}:{discriminant}");
    let mut data = None;
    let fields = match &variant.fields {
        Fields::Unit => Vec::new(),
        Fields::Named(named) => named
            .named
            .iter()
            .map(|field| {
                (
                    field.ident.clone().expect("named field has an ident"),
                    field.ty.clone(),
                )
            })
            .collect(),
        // A single unnamed field is the payload type, nested under `data`.
        Fields::Unnamed(unnamed) if unnamed.unnamed.len() == 1 => {
            data = Some(unnamed.unnamed[0].ty.clone());
            Vec::new()
        }
        Fields::Unnamed(_) => {
            return Err(Error::new_spanned(
                variant,
                "tuple event variants need exactly one field (the payload type)",
            ));
        }
    };
    Ok(EventVariant {
        ident: variant.ident.clone(),
        discriminant,
        wire_name,
        fields,
        data,
    })
}

/// The `match` arm for one variant: serialize its fields as the payload
/// and emit under the wire name. `method` is `emit` (broadcast) or
/// `emit_to` (targeted); `target` is the leading `target,` argument for
/// `emit_to` and empty for `emit`.
fn emit_arm(
    enum_ident: &syn::Ident,
    variant: &EventVariant,
    method: &TokenStream,
    target: &TokenStream,
) -> TokenStream {
    let ident = &variant.ident;
    let wire_name = &variant.wire_name;

    if variant.data.is_some() {
        let call = emit_call(
            method,
            target,
            wire_name,
            quote!(::core::clone::Clone::clone(payload)),
        );
        return quote! {
            #enum_ident::#ident(payload) => #call,
        };
    }

    if variant.fields.is_empty() {
        let call = emit_call(method, target, wire_name, quote!(()));
        return quote! {
            #enum_ident::#ident => #call,
        };
    }

    let private = private();
    let serde_crate = crate::krate::private_serde();
    let idents: Vec<_> = variant.fields.iter().map(|(ident, _)| ident).collect();
    let types: Vec<_> = variant.fields.iter().map(|(_, ty)| ty).collect();
    let call = emit_call(
        method,
        target,
        wire_name,
        quote! {
            Payload {
                #(#idents: ::core::clone::Clone::clone(#idents),)*
            }
        },
    );
    quote! {
        #enum_ident::#ident { #(#idents),* } => {
            #[derive(#private::serde::Serialize, ::core::clone::Clone)]
            #[serde(crate = #serde_crate)]
            struct Payload {
                #(#idents: #types,)*
            }
            #call
        }
    }
}

/// The emitter call shared by `emit` and `emit_to`:
/// `Emitter::{method}(emitter, {target}wire_name, payload)`. `target` is
/// empty for the broadcast `emit` and `target,` for the targeted
/// `emit_to`.
fn emit_call(
    method: &TokenStream,
    target: &TokenStream,
    wire_name: &str,
    payload: TokenStream,
) -> TokenStream {
    let private = private();
    quote! {
        #private::tauri::Emitter::#method(emitter, #target #wire_name, #payload)
    }
}

/// The `EventType` entry: each payload field lowered to a specta
/// `DataType`, plus the discriminant and wire name.
fn event_type(variant: &EventVariant) -> TokenStream {
    let rb = crate::krate::rb();
    let specta = quote!(#rb::__private::specta);
    let discriminant = &variant.discriminant;
    let wire_name = &variant.wire_name;
    let field_names = variant.fields.iter().map(|(ident, _)| ident.to_string());
    let field_types = variant.fields.iter().map(|(_, ty)| ty);
    let data = match &variant.data {
        Some(ty) => quote!(::core::option::Option::Some(
            <#ty as #specta::Type>::definition(types)
        )),
        None => quote!(::core::option::Option::None),
    };

    quote! {
        #rb::EventType {
            variant: #discriminant,
            name: #wire_name,
            fields: ::std::vec![
                #((#field_names, <#field_types as #specta::Type>::definition(types)),)*
            ],
            data: #data,
        }
    }
}

/// The event group: the enum name with a trailing `Event` stripped, then
/// lower-camel. `FaderEvent` -> `fader`, `Telemetry` -> `telemetry`.
fn event_group(ident: &syn::Ident) -> String {
    let name = ident.to_string();
    let base = name
        .strip_suffix("Event")
        .filter(|base| !base.is_empty())
        .unwrap_or(name.as_str());
    lower_camel(base)
}

#[cfg(test)]
mod tests {
    use quote::quote;

    #[test]
    fn fader_event_expansion() {
        let output = super::expand(quote! {
            enum FaderEvent {
                Changed { channel: u16, value: u8 },
            }
        })
        .expect("expansion failed");
        let file: syn::File = syn::parse2(output).expect("expansion is not valid Rust");
        insta::assert_snapshot!(prettyplease::unparse(&file));
    }

    #[test]
    fn rejects() {
        let cases = [
            (quote! { struct S; }, "Event can only be derived for enums"),
            (
                quote! { enum E { Tick(u8, u8) } },
                "tuple event variants need exactly one field (the payload type)",
            ),
            (
                quote! { enum E<T> { Tick { value: T } } },
                "generic event enums are not supported",
            ),
        ];
        for (input, message) in cases {
            let err = super::expand(input).expect_err("input should be rejected");
            assert_eq!(err.to_string(), message);
        }
    }
}
