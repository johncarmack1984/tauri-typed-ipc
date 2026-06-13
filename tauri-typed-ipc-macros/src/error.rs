//! Expansion for `#[derive(Error)]`.
//!
//! A wire-serializable error: each variant becomes a discriminated-union
//! member `{ type: "<variant>", message: "<Display>" }`. Display-based,
//! so it works with non-Serialize sources (an `io::Error` field), and the
//! `type` tag lets the client branch on the variant. Enums only. The
//! `Serialize` impl is generated here; the matching specta type lands when
//! procedures return `Result` (R3).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, Result};

use crate::lower_camel;

/// Path every generated reference to the tauri-typed-ipc crate goes through.
fn private() -> TokenStream {
    let rb = crate::krate::rb();
    quote!(#rb::__private)
}

pub fn expand(input: TokenStream) -> Result<TokenStream> {
    let rb = crate::krate::rb();
    let input: DeriveInput = syn::parse2(input)?;

    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &input.generics,
            "generic error enums are not supported",
        ));
    }

    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(
            &input,
            "Error can only be derived for enums",
        ));
    };

    let ident = &input.ident;
    let name = ident.to_string();
    let private = private();

    // The per-variant `type` tag, shared by the Serialize arms (the wire
    // shape) and the ErrorSet descriptor (the binding's union).
    let tags: Vec<String> = data
        .variants
        .iter()
        .map(|variant| lower_camel(&variant.ident.to_string()))
        .collect();
    let arms = data.variants.iter().zip(&tags).map(|(variant, tag)| {
        let variant_ident = &variant.ident;
        let pattern = match &variant.fields {
            Fields::Named(_) => quote!(#ident::#variant_ident { .. }),
            Fields::Unnamed(_) => quote!(#ident::#variant_ident(..)),
            Fields::Unit => quote!(#ident::#variant_ident),
        };
        quote!(#pattern => #tag,)
    });

    Ok(quote! {
        impl #private::serde::Serialize for #ident {
            fn serialize<S>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error>
            where
                S: #private::serde::Serializer,
            {
                let tag: &'static str = match self {
                    #(#arms)*
                };
                let mut state = #private::serde::Serializer::serialize_struct(
                    serializer,
                    #name,
                    2,
                )?;
                #private::serde::ser::SerializeStruct::serialize_field(&mut state, "type", tag)?;
                #private::serde::ser::SerializeStruct::serialize_field(
                    &mut state,
                    "message",
                    &::std::string::ToString::to_string(self),
                )?;
                #private::serde::ser::SerializeStruct::end(state)
            }
        }

        impl #rb::ErrorSet for #ident {
            fn error_type() -> #rb::ErrorType {
                #rb::ErrorType {
                    name: #name,
                    variants: ::std::vec![#(#tags,)*],
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use quote::quote;

    #[test]
    fn save_error_expansion() {
        let output = super::expand(quote! {
            enum SaveError {
                Io(std::io::Error),
                Locked,
            }
        })
        .expect("expansion failed");
        let file: syn::File = syn::parse2(output).expect("expansion is not valid Rust");
        insta::assert_snapshot!(prettyplease::unparse(&file));
    }

    #[test]
    fn rejects() {
        let cases = [
            (quote! { struct S; }, "Error can only be derived for enums"),
            (
                quote! { enum E<T> { A(T) } },
                "generic error enums are not supported",
            ),
        ];
        for (input, message) in cases {
            let err = super::expand(input).expect_err("input should be rejected");
            assert_eq!(err.to_string(), message);
        }
    }
}
