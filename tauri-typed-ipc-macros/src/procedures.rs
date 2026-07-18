//! Expansion for `#[procedures]`.
//!
//! Walking-skeleton scope: sync procedures taking `&self`, owned named
//! wire arguments, and type-matched injected parameters (`AppHandle`
//! today), plus a `{Trait}Procedures` descriptor that carries each wire
//! signature to tauri-typed-ipc's TypeScript generator. Everything else is
//! rejected with a pointed error rather than half-supported -- async and
//! events arrive in their own slices.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Error, FnArg, ItemTrait, Pat, Result, ReturnType, TraitItem, Type, parse_quote};

/// Path every generated reference to the tauri-typed-ipc crate goes through.
fn private() -> TokenStream {
    let rb = crate::krate::rb();
    quote!(#rb::__private)
}

/// Injected parameters are recognized by TYPE, never by name (a name
/// can drift silently; a type cannot). Today the set is `AppHandle`,
/// matched on the path's last segment so `tauri::AppHandle`,
/// `AppHandle<MockRuntime>`, and aliases that keep the name all
/// qualify. A type alias that hides the name falls back to being a
/// wire argument and fails to compile loudly (it has no Deserialize),
/// never silently.
fn is_injected(ty: &Type) -> bool {
    last_segment_is(ty, "AppHandle")
}

/// Managed state is injected too, but resolved through tauri's
/// runtime-free `StateManager` rather than the `dyn Any` slice -- so
/// it is recognized separately. Matched on the last path segment, like
/// [`is_injected`], so `tauri::State<'_, T>` and a plain `State<'_, T>`
/// both qualify.
fn state_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "State" {
        return None;
    }
    // The first type argument, skipping the `'_` lifetime: the `T` in
    // `State<'_, T>`, which `ctx.state::<T>()` resolves.
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

/// The `T` in a `Channel<T>` parameter -- the value the procedure
/// streams back. Recognized by the last path segment, like
/// [`state_inner`], so `ttipc::Channel<T>` and a plain `Channel<T>`
/// both qualify. A channel is the one parameter that is both a wire
/// argument (its id crosses the wire) and server-built (the handler
/// constructs it from the webview), so it is classified separately.
fn channel_inner(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Channel" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

fn last_segment_is(ty: &Type, ident: &str) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == ident)
}

/// The `T` in a `Result<T, E>` return, recognized by the last path
/// segment so `Result<..>` and `std::result::Result<..>` both qualify;
/// `None` for any other return. Drives the dispatch branch (a procedure
/// returning `Result` resolves on `Ok` and rejects on `Err`) and the
/// binding's success type (specta has no `Type` for `Result`, so the
/// descriptor renders this `Ok` type).
fn result_ok_type(ty: &Type) -> Option<&Type> {
    result_type_arg(ty, 0)
}

/// The `E` in a `Result<T, E>` return -- the error a procedure rejects
/// with, which the client binding types its catch against. `None` for
/// any non-`Result` return.
fn result_err_type(ty: &Type) -> Option<&Type> {
    result_type_arg(ty, 1)
}

/// The `n`th type argument of a `Result<..>` return (0 = `T`, 1 = `E`),
/// recognized by the last path segment. `None` for any other return.
fn result_type_arg(ty: &Type, n: usize) -> Option<&Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args
        .iter()
        .filter_map(|arg| match arg {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .nth(n)
}

/// One procedure, parsed and validated once. Both the dispatch arm and
/// the binding descriptor are built from this, so the wire/injected
/// split lives in a single place.
struct Procedure {
    /// The method's identifier, for the call into the user's trait.
    ident: syn::Ident,
    /// The wire command name -- the identifier, stringified.
    name: String,
    /// Named wire arguments, in declaration order.
    wire_idents: Vec<syn::Ident>,
    wire_types: Vec<Type>,
    /// Type-matched injected parameters (`AppHandle`).
    injected_idents: Vec<syn::Ident>,
    injected_types: Vec<Type>,
    /// Managed-state parameters (`State<T>`), paired with the inner `T`
    /// each one resolves by.
    state_idents: Vec<syn::Ident>,
    state_types: Vec<Type>,
    /// Streaming parameters (`Channel<T>`), paired with the inner `T`
    /// each one sends. The id rides the wire; the channel is built from
    /// the dispatch context.
    channel_idents: Vec<syn::Ident>,
    channel_types: Vec<Type>,
    /// Every argument ident in declaration order, naming the call.
    call_args: Vec<syn::Ident>,
    /// Return type (`()` when the procedure has no return).
    output: Type,
    /// `async fn`: dispatched through a spawned future rather than
    /// inline. Forces the `Arc<Self>` receiver on the dispatch trait.
    is_async: bool,
}

pub fn expand(attr: TokenStream, item: TokenStream) -> Result<TokenStream> {
    let rb = crate::krate::rb();
    let namespace = parse_namespace(attr)?;

    let mut item: ItemTrait = syn::parse2(item)?;

    if !item.generics.params.is_empty() || item.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &item.generics,
            "generic procedure traits are not supported",
        ));
    }

    let mut procs = Vec::new();
    for trait_item in &item.items {
        let TraitItem::Fn(method) = trait_item else {
            return Err(Error::new_spanned(
                trait_item,
                "only methods are allowed in a procedure trait",
            ));
        };
        procs.push(parse_procedure(method)?);
    }

    // A bare `async fn` in a trait returns a non-`Send` `impl Future`,
    // but the dispatch path spawns that future on tauri's runtime, which
    // needs `Send`. Rewrite each async method to spell the bound out --
    // `fn(..) -> impl Future<Output = T> + Send`. Implementors still
    // write `async fn`; the bound is just checked at the impl.
    for trait_item in &mut item.items {
        if let TraitItem::Fn(method) = trait_item
            && method.sig.asyncness.is_some()
        {
            method.sig.asyncness = None;
            let output = match &method.sig.output {
                ReturnType::Default => quote!(()),
                ReturnType::Type(_, ty) => quote!(#ty),
            };
            method.sig.output = parse_quote! {
                -> impl ::core::future::Future<Output = #output> + ::core::marker::Send
            };
        }
    }

    let private = private();
    let vis = &item.vis;
    let trait_ident = &item.ident;
    let dispatch_ident = format_ident!("{trait_ident}Dispatch");
    let procedures_ident = format_ident!("{trait_ident}Procedures");
    // An explicit `namespace` names the object and prefixes every wire
    // command (`ns.method`), matching taurpc's `path`; otherwise the
    // object is the lower-camel trait and wire names stay bare.
    let object = match &namespace {
        Some(ns) => ns.clone(),
        None => crate::lower_camel(&trait_ident.to_string()),
    };
    let namespace_const = match &namespace {
        Some(ns) => quote! {
            const NAMESPACE: ::core::option::Option<&'static str> =
                ::core::option::Option::Some(#ns);
        },
        None => quote!(),
    };
    let wire_name = |proc: &Procedure| -> String {
        match &namespace {
            Some(ns) => format!("{ns}.{}", proc.name),
            None => proc.name.clone(),
        }
    };

    let dispatch_doc = format!(
        "Generated by `#[ttipc::procedures]`: routes a `(name, args)` \
         pair to the matching [`{trait_ident}`] procedure."
    );
    let procedures_doc = format!(
        "Generated by `#[ttipc::procedures]`: the binding descriptor \
         for [`{trait_ident}`]. Hand to `ttipc::Bindings::register` to \
         emit the TypeScript client."
    );

    let names: Vec<String> = procs.iter().map(wire_name).collect();
    let arms = procs
        .iter()
        .zip(&names)
        .map(|(proc, wire)| dispatch_arm(trait_ident, proc, wire));
    let proc_types = procs.iter().map(procedure_type);

    // An `async fn` is dispatched through a spawned `'static` future, so
    // its receiver must be owned: a set with any async procedure takes
    // `self: Arc<Self>`, and `into_procedures` hands the closure a fresh
    // clone per call. `Arc<Self>: Send + Sync` is exactly the bound
    // `into_procedures` already requires, so async adds no new bound. A
    // fully sync set keeps the borrowed `&self` receiver unchanged.
    let has_async = procs.iter().any(|proc| proc.is_async);
    let dispatch_self = if has_async {
        quote!(self: ::std::sync::Arc<Self>)
    } else {
        quote!(&self)
    };
    // The async arm captures `Arc<Self>` into a `Send` future, which
    // needs `Self: Send + Sync + 'static` in scope. `into_procedures`
    // already requires it, so this only restates it where the dispatch
    // body needs it; a fully sync set keeps the unbounded receiver.
    let dispatch_where = if has_async {
        quote!(where Self: ::core::marker::Send + ::core::marker::Sync + 'static)
    } else {
        quote!()
    };
    let into_procedures_body = if has_async {
        quote! {
            let this = ::std::sync::Arc::new(self);
            #rb::Procedures::new(
                &[#(#names,)*],
                move |ctx, procedure, args| {
                    #dispatch_ident::dispatch(::std::sync::Arc::clone(&this), ctx, procedure, args)
                },
            )
        }
    } else {
        quote! {
            #rb::Procedures::new(
                &[#(#names,)*],
                move |ctx, procedure, args| {
                    #dispatch_ident::dispatch(&self, ctx, procedure, args)
                },
            )
        }
    };

    Ok(quote! {
        #item

        #[doc = #dispatch_doc]
        #vis trait #dispatch_ident: #trait_ident {
            /// Routes one `(procedure, args)` call to the matching method:
            /// deserializes the wire arguments, resolves injected
            /// parameters from `ctx`, and returns the dispatch outcome.
            fn dispatch(
                #dispatch_self,
                _ctx: &#rb::Context<'_>,
                procedure: &str,
                args: #private::serde_json::Value,
            ) -> #rb::Dispatch
            #dispatch_where
            {
                match procedure {
                    #(#arms)*
                    _ => #rb::Dispatch::Sync(::core::result::Result::Err(
                        #rb::DispatchError::UnknownProcedure(
                            ::std::borrow::ToOwned::to_owned(procedure),
                        ),
                    )),
                }
            }

            /// Type-erases this set for registration with
            /// `ttipc::handler`. The bounds are tauri's
            /// `invoke_handler` bounds, inherited through
            /// `ttipc::Procedures::new`.
            fn into_procedures(self) -> #rb::Procedures
            where
                Self: ::core::marker::Sized
                    + ::core::marker::Send
                    + ::core::marker::Sync
                    + 'static,
            {
                #into_procedures_body
            }
        }

        impl<T: #trait_ident> #dispatch_ident for T {}

        #[doc = #procedures_doc]
        #vis struct #procedures_ident;

        impl #rb::ProcedureSet for #procedures_ident {
            const OBJECT: &'static str = #object;
            #namespace_const

            fn procedures(
                types: &mut #private::specta::Types,
            ) -> ::std::vec::Vec<#rb::ProcedureType> {
                ::std::vec![#(#proc_types,)*]
            }
        }
    })
}

/// Parse the optional `#[procedures(namespace = "...")]` argument (or its
/// taurpc spelling `path = "..."`): the explicit knob that prefixes every
/// wire command (`ns.method`) and names the object. Absent -> bare names.
fn parse_namespace(attr: TokenStream) -> Result<Option<String>> {
    if attr.is_empty() {
        return Ok(None);
    }
    let meta: syn::MetaNameValue = syn::parse2(attr)?;
    // `path` is taurpc's spelling, `namespace` is tauri-typed-ipc's -- accept both.
    if !meta.path.is_ident("namespace") && !meta.path.is_ident("path") {
        return Err(Error::new_spanned(
            &meta.path,
            "#[procedures] only accepts `namespace = \"...\"` (alias `path`)",
        ));
    }
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(lit),
        ..
    }) = &meta.value
    else {
        return Err(Error::new_spanned(
            &meta.value,
            "namespace must be a string literal",
        ));
    };
    let value = lit.value();
    if value.is_empty() || value.contains('.') {
        return Err(Error::new_spanned(
            lit,
            "namespace must be non-empty and contain no `.`",
        ));
    }
    Ok(Some(value))
}

fn parse_procedure(method: &syn::TraitItemFn) -> Result<Procedure> {
    let sig = &method.sig;

    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return Err(Error::new_spanned(
            &sig.generics,
            "generic procedures are not supported",
        ));
    }
    match sig.receiver() {
        Some(receiver) if receiver.reference.is_some() && receiver.mutability.is_none() => {}
        _ => {
            return Err(Error::new_spanned(
                sig,
                "procedures take &self (shared access to the procedure set's state)",
            ));
        }
    }

    let mut wire_idents = Vec::new();
    let mut wire_types = Vec::new();
    let mut injected_idents = Vec::new();
    let mut injected_types = Vec::new();
    let mut state_idents = Vec::new();
    let mut state_types = Vec::new();
    let mut channel_idents = Vec::new();
    let mut channel_types = Vec::new();
    let mut call_args = Vec::new();
    for arg in &sig.inputs {
        let FnArg::Typed(arg) = arg else {
            // The receiver, already validated above.
            continue;
        };
        let Pat::Ident(pat) = &*arg.pat else {
            return Err(Error::new_spanned(
                &arg.pat,
                "procedure arguments must be plain identifiers (they name the wire fields)",
            ));
        };
        if matches!(&*arg.ty, Type::Reference(_)) {
            return Err(Error::new_spanned(
                &arg.ty,
                "procedure arguments must be owned types (they are deserialized off the wire)",
            ));
        }
        if last_segment_is(&arg.ty, "State") {
            let inner = state_inner(&arg.ty).ok_or_else(|| {
                Error::new_spanned(
                    &arg.ty,
                    "State parameters need a type argument, e.g. State<'_, T>",
                )
            })?;
            state_idents.push(pat.ident.clone());
            state_types.push(inner.clone());
        } else if last_segment_is(&arg.ty, "Channel") {
            let inner = channel_inner(&arg.ty).ok_or_else(|| {
                Error::new_spanned(
                    &arg.ty,
                    "Channel parameters need a type argument, e.g. Channel<T>",
                )
            })?;
            channel_idents.push(pat.ident.clone());
            channel_types.push(inner.clone());
        } else if is_injected(&arg.ty) {
            if !injected_idents.is_empty() {
                return Err(Error::new_spanned(
                    &arg.ty,
                    "duplicate injected parameter: only one AppHandle per procedure",
                ));
            }
            injected_idents.push(pat.ident.clone());
            injected_types.push((*arg.ty).clone());
        } else {
            wire_idents.push(pat.ident.clone());
            wire_types.push((*arg.ty).clone());
        }
        call_args.push(pat.ident.clone());
    }

    // An async body runs off the main thread as a `'static` future, so
    // its inputs must be owned. Wire arguments are (deserialized owned),
    // an injected `AppHandle` is cloned out of the `Context` in the
    // synchronous prelude and moved in, a `Channel<T>` is likewise built
    // in the prelude and owned, and `State<T>` is resolved inside the
    // future from the owned `Arc<StateManager>` the prelude clones out
    // (a borrow from the context could not cross the spawn).
    let is_async = sig.asyncness.is_some();

    let output = match &sig.output {
        ReturnType::Default => parse_quote!(()),
        ReturnType::Type(_, ty) => (**ty).clone(),
    };

    Ok(Procedure {
        ident: sig.ident.clone(),
        name: sig.ident.to_string(),
        wire_idents,
        wire_types,
        injected_idents,
        injected_types,
        state_idents,
        state_types,
        channel_idents,
        channel_types,
        call_args,
        output,
        is_async,
    })
}

/// The `match` arm that deserializes the wire args, resolves injected
/// parameters from the [`Context`], calls the procedure, and turns its
/// return into a wire [`Outcome`]. A sync procedure settles inline; an
/// `async fn` deserializes up front and hands back a spawnable future.
///
/// [`Outcome`]: ttipc::Outcome
fn dispatch_arm(trait_ident: &syn::Ident, proc: &Procedure, wire: &str) -> TokenStream {
    let rb = crate::krate::rb();
    let private = private();
    let Procedure {
        ident,
        wire_idents,
        wire_types,
        injected_idents,
        injected_types,
        state_idents,
        state_types,
        channel_idents,
        channel_types,
        call_args,
        output,
        is_async,
        ..
    } = proc;

    // `&*self` is the receiver either way: a borrowed `&Self`, or `&Self`
    // through the `Arc<Self>` an async set carries.
    let call = if *is_async {
        quote!(#trait_ident::#ident(&*self, #(#call_args,)*).await)
    } else {
        quote!(#trait_ident::#ident(&*self, #(#call_args,)*))
    };

    // A `Result` return resolves on `Ok` and rejects with the serialized
    // typed error on `Err` (raw-command parity); any other return
    // resolves.
    let outcome = if result_ok_type(output).is_some() {
        quote! {
            match #call {
                ::core::result::Result::Ok(value) => #private::serde_json::to_value(value)
                    .map(#rb::Outcome::Resolve)
                    .map_err(#rb::DispatchError::Serialize),
                ::core::result::Result::Err(error) => #private::serde_json::to_value(error)
                    .map(#rb::Outcome::Reject)
                    .map_err(#rb::DispatchError::Serialize),
            }
        }
    } else {
        quote! {
            #private::serde_json::to_value(#call)
                .map(#rb::Outcome::Resolve)
                .map_err(#rb::DispatchError::Serialize)
        }
    };

    // A `Channel<T>` parameter is both a wire argument (its id is a
    // string in the JSON body) and server-built (the channel is made
    // from the dispatch context's factory), so the id deserializes as a
    // field here and the typed channel is built from it below.
    let serde_crate = crate::krate::private_serde();
    let args_struct = quote! {
        #[derive(#private::serde::Deserialize)]
        #[serde(crate = #serde_crate)]
        struct Args {
            #(#wire_idents: #wire_types,)*
            #(#channel_idents: #private::tauri::ipc::JavaScriptChannelId,)*
        }
    };

    if *is_async {
        // Deserialize wire args up front so bad input fails without a
        // spawn, then resolve everything the future must own in the
        // synchronous prelude: any `Channel<T>` (built here, owned), any
        // injected `AppHandle` (cloned out of the context), and -- when
        // the procedure takes `State<T>` -- the `Arc<StateManager>`
        // itself, because `State` borrows and is therefore resolved
        // inside the future from that owned manager.
        let state_prelude = if state_idents.is_empty() {
            quote!()
        } else {
            let first_state = &state_types[0];
            quote! {
                let __ttipc_state_manager = match _ctx.state_manager() {
                    ::core::option::Option::Some(manager) => manager,
                    ::core::option::Option::None => {
                        return #rb::Dispatch::Sync(::core::result::Result::Err(
                            #rb::DispatchError::MissingState(
                                ::core::any::type_name::<#first_state>(),
                            ),
                        ));
                    }
                };
            }
        };
        quote! {
            #wire => {
                #args_struct
                match #private::serde_json::from_value::<Args>(args) {
                    ::core::result::Result::Ok(Args {
                        #(#wire_idents,)*
                        #(#channel_idents,)*
                    }) => {
                        #(
                            let #channel_idents = match _ctx.channel::<#channel_types>(#channel_idents) {
                                ::core::option::Option::Some(channel) => channel,
                                ::core::option::Option::None => {
                                    return #rb::Dispatch::Sync(::core::result::Result::Err(
                                        #rb::DispatchError::MissingChannel(
                                            ::core::any::type_name::<#rb::Channel<#channel_types>>(),
                                        ),
                                    ));
                                }
                            };
                        )*
                        #(
                            let #injected_idents = match _ctx.extract::<#injected_types>() {
                                ::core::option::Option::Some(value) => value,
                                ::core::option::Option::None => {
                                    return #rb::Dispatch::Sync(::core::result::Result::Err(
                                        #rb::DispatchError::MissingInjection(
                                            ::core::any::type_name::<#injected_types>(),
                                        ),
                                    ));
                                }
                            };
                        )*
                        #state_prelude
                        #rb::Dispatch::Async(::std::boxed::Box::pin(async move {
                            #(
                                let #state_idents = __ttipc_state_manager
                                    .try_get::<#state_types>()
                                    .ok_or(#rb::DispatchError::MissingState(
                                        ::core::any::type_name::<#state_types>(),
                                    ))?;
                            )*
                            #outcome
                        }))
                    }
                    ::core::result::Result::Err(error) => #rb::Dispatch::Sync(
                        ::core::result::Result::Err(#rb::DispatchError::Deserialize(error)),
                    ),
                }
            }
        }
    } else {
        // Settle inline. The closure gives `?` a `Result`-returning scope
        // without early-returning the `Dispatch` this arm yields.
        quote! {
            #wire => #rb::Dispatch::Sync(
                (|| -> ::core::result::Result<#rb::Outcome, #rb::DispatchError> {
                    #args_struct
                    let Args { #(#wire_idents,)* #(#channel_idents,)* } =
                        #private::serde_json::from_value(args)
                            .map_err(#rb::DispatchError::Deserialize)?;
                    #(
                        let #injected_idents = _ctx
                            .extract::<#injected_types>()
                            .ok_or(#rb::DispatchError::MissingInjection(
                                ::core::any::type_name::<#injected_types>(),
                            ))?;
                    )*
                    #(
                        let #state_idents = _ctx
                            .state::<#state_types>()
                            .ok_or(#rb::DispatchError::MissingState(
                                ::core::any::type_name::<#state_types>(),
                            ))?;
                    )*
                    #(
                        let #channel_idents = _ctx
                            .channel::<#channel_types>(#channel_idents)
                            .ok_or(#rb::DispatchError::MissingChannel(
                                ::core::any::type_name::<#rb::Channel<#channel_types>>(),
                            ))?;
                    )*
                    #outcome
                })(),
            ),
        }
    }
}

/// The `ProcedureType` entry: each wire argument, each `Channel<T>`'s
/// inner `T`, and the return lowered to a specta `DataType`. Injected
/// parameters (`AppHandle`, `State`) never appear -- they are
/// server-side, not part of the client surface -- but channels do: the
/// client passes them, so the binding renders them as `Channel<T>`. A
/// `Result<T, E>` return contributes its success type `T`: specta has no
/// `Type` for `Result`, and the wire resolves with `T` (an `Err` rejects
/// instead).
fn procedure_type(proc: &Procedure) -> TokenStream {
    let rb = crate::krate::rb();
    let specta = quote!(#rb::__private::specta);
    let name = &proc.name;
    let arg_names = proc.wire_idents.iter().map(syn::Ident::to_string);
    let arg_types = &proc.wire_types;
    let channel_names = proc.channel_idents.iter().map(syn::Ident::to_string);
    let channel_types = &proc.channel_types;
    let output = result_ok_type(&proc.output).unwrap_or(&proc.output);
    // A `Result<_, E>` return contributes its error type so the client can
    // type the rejection (`E: ttipc::Error`); other returns reject
    // nothing.
    let error = match result_err_type(&proc.output) {
        Some(err) => quote! {
            ::core::option::Option::Some(<#err as #rb::ErrorSet>::error_type())
        },
        None => quote!(::core::option::Option::None),
    };

    quote! {
        #rb::ProcedureType {
            name: #name,
            args: ::std::vec![
                #((#arg_names, <#arg_types as #specta::Type>::definition(types)),)*
            ],
            channels: ::std::vec![
                #((#channel_names, <#channel_types as #specta::Type>::definition(types)),)*
            ],
            output: <#output as #specta::Type>::definition(types),
            error: #error,
        }
    }
}

#[cfg(test)]
mod tests {
    use quote::quote;

    #[test]
    fn greet_expansion() {
        let output = super::expand(
            quote!(),
            quote! {
                trait Greeter {
                    fn greet(&self, name: String) -> String;
                }
            },
        )
        .expect("expansion failed");
        let file: syn::File = syn::parse2(output).expect("expansion is not valid Rust");
        insta::assert_snapshot!(prettyplease::unparse(&file));
    }

    #[test]
    fn injected_expansion() {
        // The injected AppHandle splits off the wire Args struct and
        // becomes a type-matched ctx.extract instead -- snapshotted so
        // that split is visible and stays under test.
        let output = super::expand(
            quote!(),
            quote! {
                trait Faders {
                    fn set(&self, app: tauri::AppHandle, channel: u16, value: u8);
                }
            },
        )
        .expect("expansion failed");
        let file: syn::File = syn::parse2(output).expect("expansion is not valid Rust");
        insta::assert_snapshot!(prettyplease::unparse(&file));
    }

    #[test]
    fn state_expansion() {
        // A State<T> param splits off the wire Args like an AppHandle,
        // but resolves through ctx.state (tauri's runtime-free
        // StateManager) instead of ctx.extract -- snapshotted so that
        // distinction stays visible and under test.
        let output = super::expand(
            quote!(),
            quote! {
                trait Counted {
                    fn hits(&self, state: tauri::State<'_, Hits>) -> u32;
                }
            },
        )
        .expect("expansion failed");
        let file: syn::File = syn::parse2(output).expect("expansion is not valid Rust");
        insta::assert_snapshot!(prettyplease::unparse(&file));
    }

    #[test]
    fn async_expansion() {
        // A set mixing a sync procedure with an async one: the async
        // procedure forces the `Arc<Self>` receiver, the sync arm still
        // calls through `&*self`, and `into_procedures` wraps `self` in
        // an `Arc`. The async arm deserializes up front, then boxes a
        // future whose `Result` return resolves on `Ok`, rejects on
        // `Err` -- snapshotted so all of that stays visible.
        let output = super::expand(
            quote!(),
            quote! {
                trait Backup {
                    fn last(&self) -> String;
                    async fn save(&self, path: String) -> Result<(), SaveError>;
                }
            },
        )
        .expect("expansion failed");
        let file: syn::File = syn::parse2(output).expect("expansion is not valid Rust");
        insta::assert_snapshot!(prettyplease::unparse(&file));
    }

    #[test]
    fn channel_expansion() {
        // A Channel<T> param is both a wire argument (its id rides the
        // Args struct as a JavaScriptChannelId) and server-built (the
        // typed channel comes from ctx.channel). Snapshotted both ways:
        // sync builds it inline, async builds it in the prelude so the
        // owned channel moves into the spawned future.
        let output = super::expand(
            quote!(),
            quote! {
                trait Downloads {
                    fn track(&self, progress: ttipc::Channel<Progress>);
                    async fn track_async(&self, progress: ttipc::Channel<Progress>);
                }
            },
        )
        .expect("expansion failed");
        let file: syn::File = syn::parse2(output).expect("expansion is not valid Rust");
        insta::assert_snapshot!(prettyplease::unparse(&file));
    }

    #[test]
    fn async_injection_expansion() {
        // An async procedure taking both an AppHandle and State: the
        // handle is cloned out of the context in the synchronous prelude
        // and moved into the future; State is resolved inside the future
        // from the owned Arc<StateManager> the prelude clones out (a
        // borrow could not cross the spawn) -- snapshotted so both
        // resolutions stay visible and under test.
        let output = super::expand(
            quote!(),
            quote! {
                trait Vault {
                    async fn store(
                        &self,
                        app: tauri::AppHandle,
                        hits: tauri::State<'_, Hits>,
                        label: String,
                    ) -> u32;
                }
            },
        )
        .expect("expansion failed");
        let file: syn::File = syn::parse2(output).expect("expansion is not valid Rust");
        insta::assert_snapshot!(prettyplease::unparse(&file));
    }

    #[test]
    fn rejects() {
        let cases = [
            (
                quote! { trait T { fn a(self); } },
                "procedures take &self (shared access to the procedure set's state)",
            ),
            (
                quote! { trait T { fn a(&self, s: &str); } },
                "procedure arguments must be owned types (they are deserialized off the wire)",
            ),
            (
                quote! { trait T { const N: u8; } },
                "only methods are allowed in a procedure trait",
            ),
            (
                quote! { trait T { fn a(&self, app: tauri::AppHandle, other: AppHandle); } },
                "duplicate injected parameter: only one AppHandle per procedure",
            ),
            (
                quote! { trait T { fn a(&self, c: ttipc::Channel); } },
                "Channel parameters need a type argument, e.g. Channel<T>",
            ),
        ];
        for (input, message) in cases {
            let err = super::expand(quote!(), input).expect_err("input should be rejected");
            assert_eq!(err.to_string(), message);
        }
    }
}
