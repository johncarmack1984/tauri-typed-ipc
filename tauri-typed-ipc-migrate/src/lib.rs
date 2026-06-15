//! A source-to-source transform from a TauRPC procedures trait to a ttipc
//! `#[procedures]` trait, for migrating an existing TauRPC app.
//!
//! ttipc erases the runtime type parameter and injects by type, so the
//! per-method `<R: Runtime>` + `AppHandle<R>` boilerplate collapses to a plain
//! `&self` receiver and a bare `AppHandle`. [`transform`] applies that rewrite
//! to the trait definition and to the resolver impl (and drops the resolver
//! struct's `#[taurpc::ipc_type]`). TauRPC is async-only; a method whose resolver
//! body does no real async work is made sync (ttipc's default), transitively
//! through sibling calls, since ttipc also rejects an async procedure that
//! takes an injected `AppHandle` or `State<T>`.
//!
//! The transform parses with `syn` and re-emits with `prettyplease`, so the
//! output is clean Rust the migrator runs their own `rustfmt` over. `syn` drops
//! non-doc comments, so this targets the trait and impl items, not whole files.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::token::Comma;
use syn::visit::Visit;
use syn::visit_mut::VisitMut;
use syn::{
    Attribute, Expr, FnArg, ImplItem, Item, ItemImpl, ItemTrait, Meta, PathArguments, Signature,
    TraitItem, Type,
};

/// Rewrite TauRPC procedures-trait source into ttipc `#[procedures]`
/// source. Any trait carrying `#[taurpc::procedures(..)]` is transformed; every
/// other item passes through unchanged. Errors only when `src` is not parseable
/// Rust.
pub fn transform(src: &str) -> Result<String, syn::Error> {
    let file: syn::File = syn::parse_str(src)?;
    // Events declared in this file -- enough to rewrite same-file emit sites. The
    // multi-file driver passes a registry spanning every file instead, so emit
    // sites in a file other than the trait's also resolve.
    let mut registry = EventRegistry::default();
    collect_enums(&file, &mut registry.enums);
    collect_router_factories(&file, &mut registry.router_factories);
    extract_events(&file, &mut registry);
    Ok(render(file, &registry))
}

/// Preview a whole file: transform it and re-emit via `prettyplease`. This
/// reformats the file and drops non-doc comments -- fine for stdout, but not for
/// editing real files in place; [`transform_surgical`] is the in-place path.
fn render(mut file: syn::File, registry: &EventRegistry) -> String {
    let deasyncable = collect_deasyncable(&file);
    let resolver_types = resolver_struct_idents(&file);
    let findings = transform_ast(&mut file, &deasyncable, &resolver_types, registry);
    format!("{}{}", header(&findings), prettyplease::unparse(&file))
}

/// Apply every transform to a parsed file, given the file-level analysis
/// (de-asyncable methods, resolver struct idents) and the event registry.
/// Returns what it did, for the header. Used for the whole file (preview) and,
/// per item, on a one-item file (surgical) -- so the analysis is passed in rather
/// than recomputed, keeping a single item's view consistent with the whole file.
fn transform_ast(
    file: &mut syn::File,
    deasyncable: &HashSet<(String, String)>,
    resolver_types: &[syn::Ident],
    registry: &EventRegistry,
) -> Findings {
    let mut findings = Findings::default();
    // `#[taurpc(event)]` methods are lifted into `#[derive(Event)]` enums, each
    // inserted just after its source trait once the borrow loop ends.
    let mut event_enums: Vec<(usize, Item)> = Vec::new();
    for (index, item) in file.items.iter_mut().enumerate() {
        match item {
            Item::Trait(item_trait) if has_taurpc_procedures(item_trait) => {
                findings.transformed = true;
                if let Some(event_enum) =
                    transform_trait(item_trait, deasyncable, registry, &mut findings)
                {
                    event_enums.push((index, event_enum));
                }
            }
            Item::Impl(item_impl) if has_taurpc_resolvers(item_impl) => {
                findings.transformed = true;
                transform_resolver_impl(item_impl, deasyncable, &mut findings);
            }
            _ => {}
        }
    }
    // Descending so each insert leaves the earlier indices valid.
    for (index, event_enum) in event_enums.into_iter().rev() {
        file.items.insert(index + 1, event_enum);
    }
    // `#[taurpc::ipc_type]` is shorthand for the wire derives. The resolver
    // struct is the handler, not a wire type, so it just drops the attribute;
    // every other `ipc_type` struct is a DTO, so spell the derives out.
    for item in &mut file.items {
        if let Item::Struct(item_struct) = item {
            if resolver_types.contains(&item_struct.ident) {
                item_struct
                    .attrs
                    .retain(|attr| !is_taurpc_attr(attr, "ipc_type"));
            } else {
                convert_ipc_type(&mut item_struct.attrs, &mut findings);
            }
        }
    }
    // A payload enum (the central event-channel pattern, e.g. a single
    // `fn event(event: AppEvent)`) derives `ttipc::Event` in place, so its
    // variants become the events -- no wrapper enum, no name collision.
    for item in &mut file.items {
        if let Item::Enum(item_enum) = item
            && registry.derive_on.contains(&item_enum.ident.to_string())
        {
            item_enum
                .attrs
                .insert(0, syn::parse_quote!(#[derive(ttipc::Event)]));
            findings.events_payload_enum = true;
            findings.transformed = true;
        }
    }
    // The mount (often a separate `main.rs`/`lib.rs`): rewrite the TauRPC Router
    // chain into a ttipc `handler(..)`. Structural, so it works even where the
    // resolver impls live in another file.
    rewrite_mount(file, &mut findings);
    // Emit sites: `Trigger::new(h).method(args)` -> `Enum::Variant { .. }.emit(&h)`,
    // resolved through the registry (so a trait in another file still maps).
    rewrite_emits(file, registry, &mut findings);
    // Cross-file mount consumers: `factory().into_handler()` -> `ttipc::handler(..)`
    // (the factory may be defined in another file).
    rewrite_factory_consumers(file, &registry.router_factories, &mut findings);
    findings
}

/// The self-type idents of every `#[taurpc::resolvers]` impl in the file -- the
/// handler structs, which are not wire types (so their `ipc_type` is stripped,
/// not converted).
fn resolver_struct_idents(file: &syn::File) -> Vec<syn::Ident> {
    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Impl(item_impl) if has_taurpc_resolvers(item_impl) => self_type_ident(item_impl),
            _ => None,
        })
        .collect()
}

/// Migrate a whole project in place: build one event registry across every file
/// (so a `...EventTrigger` emit site resolves even when its trait is in another
/// file), then surgically transform each. Returns `(path, new_source)` for every
/// input; an unchanged file comes back byte-identical.
pub fn transform_project(files: &[(String, String)]) -> Result<Vec<(String, String)>, syn::Error> {
    let parsed: Vec<syn::File> = files
        .iter()
        .map(|(_, src)| syn::parse_str(src))
        .collect::<Result<_, _>>()?;
    let mut registry = EventRegistry::default();
    for file in &parsed {
        collect_enums(file, &mut registry.enums);
        collect_router_factories(file, &mut registry.router_factories);
    }
    for file in &parsed {
        extract_events(file, &mut registry);
    }
    files
        .iter()
        .map(|(path, src)| Ok((path.clone(), transform_surgical(src, &registry)?)))
        .collect()
}

/// Surgically apply the migration to one file: replace only the byte-spans of the
/// constructs that change, leaving the rest (comments, formatting, unrelated
/// code) byte-for-byte. Migrated taurpc items and the mount fn are re-emitted
/// whole (their inner non-doc comments are lost); emit sites elsewhere are
/// replaced expression-by-expression so the surrounding code is untouched.
fn transform_surgical(src: &str, registry: &EventRegistry) -> Result<String, syn::Error> {
    let file: syn::File = syn::parse_str(src)?;
    let deasyncable = collect_deasyncable(&file);
    let resolver_types = resolver_struct_idents(&file);
    let mut findings = Findings::default();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();

    for item in &file.items {
        if is_item_level(item, &registry.derive_on) {
            // Re-emit the whole item (and any event enum it spawns) by running the
            // pipeline on a one-item file, then replace the item's span with it.
            let mut mini = syn::File {
                shebang: None,
                attrs: Vec::new(),
                items: vec![item.clone()],
            };
            let item_findings = transform_ast(&mut mini, &deasyncable, &resolver_types, registry);
            merge_findings(&mut findings, &item_findings);
            let rendered = prettyplease::unparse(&mini).trim_end().to_string();
            edits.push((item.span().byte_range(), rendered));
        } else {
            // Leave the item verbatim; only its emit sites (if any) change.
            let mut collector = EmitEditCollector {
                registry,
                edits: Vec::new(),
                emit_rewritten: false,
                factory_rewritten: false,
                unresolved: false,
            };
            collector.visit_item(item);
            if !collector.edits.is_empty() {
                findings.transformed = true;
                edits.append(&mut collector.edits);
            }
            if collector.emit_rewritten {
                findings.emits_rewritten = true;
            }
            if collector.factory_rewritten {
                findings.mount_rewritten = true;
            }
            if collector.unresolved {
                findings.emits_unresolved = true;
                findings.transformed = true;
            }
        }
    }

    // Apply right-to-left so earlier byte offsets stay valid.
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut out = src.to_string();
    for (range, replacement) in edits {
        out.replace_range(range, &replacement);
    }
    Ok(format!("{}{}", header(&findings), out))
}

/// Items re-emitted whole (vs. left verbatim with only their emit sites changed):
/// the migrated taurpc constructs, and the fn holding the mount.
fn is_item_level(item: &Item, derive_on: &HashSet<String>) -> bool {
    match item {
        Item::Trait(item_trait) => has_taurpc_procedures(item_trait),
        Item::Impl(item_impl) => has_taurpc_resolvers(item_impl),
        Item::Struct(item_struct) => item_struct
            .attrs
            .iter()
            .any(|attr| is_taurpc_attr(attr, "ipc_type")),
        // A payload enum gains `#[derive(ttipc::Event)]`, so it is re-emitted.
        Item::Enum(item_enum) => derive_on.contains(&item_enum.ident.to_string()),
        Item::Fn(_) => item_has_mount(item),
        _ => false,
    }
}

/// Does the item construct a TauRPC mount (`Router::new()`/`create_ipc_handler`)?
fn item_has_mount(item: &Item) -> bool {
    let mut scan = MountScan { found: false };
    scan.visit_item(item);
    scan.found
}

/// Collects expression-level edits for emit sites within an otherwise-verbatim
/// item, so the surrounding code is preserved byte-for-byte.
struct EmitEditCollector<'a> {
    registry: &'a EventRegistry,
    edits: Vec<(Range<usize>, String)>,
    /// An emit site (`Trigger::new(h).method(..)`) was rewritten.
    emit_rewritten: bool,
    /// A cross-file `factory().into_handler()` mount consumer was rewritten.
    factory_rewritten: bool,
    /// A registered trigger was constructed somewhere the direct rewrite could
    /// not reach (bound to a variable, or `.send_to(..)` targeted).
    unresolved: bool,
}

impl<'ast> Visit<'ast> for EmitEditCollector<'_> {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Some(replacement) = emit_replacement(expr, self.registry) {
            let indent = expr.span().start().column;
            self.edits
                .push((expr.span().byte_range(), format_expr(&replacement, indent)));
            self.emit_rewritten = true;
            return; // the whole emit expr is replaced; do not recurse into it
        }
        // A cross-file mount consumer (`factory().into_handler()`) in an otherwise
        // verbatim item -- the factory is defined in another file.
        if let Some(replacement) =
            factory_consumer_replacement(expr, &self.registry.router_factories)
        {
            let indent = expr.span().start().column;
            self.edits
                .push((expr.span().byte_range(), format_expr(&replacement, indent)));
            self.factory_rewritten = true;
            return;
        }
        // A registered `Trigger::new(..)` reached here (not consumed by a direct
        // emit above, which returns early) is an emit site we cannot rewrite.
        if let Some((trigger, _)) = trigger_new(expr)
            && self.registry.triggers.contains_key(&trigger)
        {
            self.unresolved = true;
        }
        syn::visit::visit_expr(self, expr);
    }
}

/// Render a replacement expression with `prettyplease`, indenting continuation
/// lines to `indent` so it drops cleanly into the original column.
fn format_expr(expr: &Expr, indent: usize) -> String {
    let wrapper: syn::File = syn::parse_quote!(
        fn __wrap() {
            #expr
        }
    );
    let rendered = prettyplease::unparse(&wrapper);
    let pad = " ".repeat(indent);
    rendered
        .lines()
        .skip(1) // the `fn __wrap() {` line
        .take_while(|line| *line != "}")
        .enumerate()
        .map(|(i, line)| {
            let dedented = line.strip_prefix("    ").unwrap_or(line);
            if i == 0 {
                dedented.to_string()
            } else {
                format!("{pad}{dedented}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// OR one item's findings into the file's running findings.
fn merge_findings(into: &mut Findings, from: &Findings) {
    into.transformed |= from.transformed;
    into.result_returns |= from.result_returns;
    into.dropped_generics |= from.dropped_generics;
    into.channels |= from.channels;
    into.deasynced |= from.deasynced;
    into.async_injection |= from.async_injection;
    into.window_param |= from.window_param;
    into.ipc_type_converted |= from.ipc_type_converted;
    into.events_lifted |= from.events_lifted;
    into.events_payload_enum |= from.events_payload_enum;
    into.event_payload_external |= from.event_payload_external;
    into.multi_level_event |= from.multi_level_event;
    into.alias_dropped |= from.alias_dropped;
    into.mount_rewritten |= from.mount_rewritten;
    into.emits_rewritten |= from.emits_rewritten;
    into.emits_unresolved |= from.emits_unresolved;
}

/// What the transform did, so it can flag the steps it cannot do itself.
#[derive(Default)]
struct Findings {
    transformed: bool,
    result_returns: bool,
    dropped_generics: bool,
    channels: bool,
    deasynced: bool,
    async_injection: bool,
    window_param: bool,
    ipc_type_converted: bool,
    events_lifted: bool,
    events_payload_enum: bool,
    event_payload_external: bool,
    multi_level_event: bool,
    alias_dropped: bool,
    mount_rewritten: bool,
    emits_rewritten: bool,
    emits_unresolved: bool,
}

/// A comment block listing the manual follow-ups: the error type must derive
/// `ttipc::Error`, and the TauRPC-only imports go stale. Empty when nothing
/// was transformed, or when nothing needs flagging.
fn header(findings: &Findings) -> String {
    if !findings.transformed {
        return String::new();
    }
    let mut notes: Vec<&str> = Vec::new();
    if findings.result_returns {
        notes.push(
            "//   - errors: each `Result<_, E>` needs `E: ttipc::Error` (derive it on the error type).",
        );
    }
    if findings.dropped_generics || findings.channels {
        notes.push(
            "//   - imports: drop the now-unused `Runtime`; `Channel` is now `ttipc::Channel`.",
        );
    }
    if findings.ipc_type_converted {
        notes.push(
            "//   - dto deps: converted `ipc_type` structs derive `serde` and `specta::Type` directly; add `serde` (with `derive`) and `specta` as dependencies.",
        );
    }
    if findings.deasynced {
        notes.push(
            "//   - de-async: methods with no blocking `.await` were made sync (ttipc's default), and their `.await`s on now-sync siblings were dropped.",
        );
    }
    if findings.async_injection {
        notes.push(
            "//   - async injection: ttipc rejects an `async` procedure that takes `AppHandle`/`State<T>`; make it sync, or rework so the async body needs no injected handle/state.",
        );
    }
    if findings.window_param {
        notes.push(
            "//   - windows: ttipc injects only `AppHandle`/`State<T>`; a `Window`/`WebviewWindow` parameter needs manual rework (obtain it from the injected `AppHandle`).",
        );
    }
    if findings.events_lifted {
        notes.push(
            "//   - events: `#[taurpc(event)]` methods were lifted into a `#[derive(ttipc::Event)]` enum (matching emit sites were rewritten to `Enum::Variant.emit(&h)`); drop any now-empty trait/impl.",
        );
    }
    if findings.events_payload_enum {
        notes.push(
            "//   - events: a `#[taurpc(event)]` method carrying a payload enum was removed; `#[derive(ttipc::Event)]` was added to that enum (its variants are the events). Emit it directly as `event.emit(&h)`; drop any now-empty trait/impl.",
        );
    }
    if findings.event_payload_external {
        notes.push(
            "//   - events: a `#[taurpc(event)]` method carrying an external payload enum was removed; add `#[derive(ttipc::Event)]` to that enum in its defining crate (its group must match the namespace), and emit it as `event.emit(&h)`.",
        );
    }
    if findings.emits_unresolved {
        notes.push(
            "//   - emit sites: a trigger bound to a variable was left as-is (only inline `Trigger::new(h)...` sites are rewritten); do it by hand as `Enum::Variant { .. }.emit(&h)` (or `.emit_to(&h, target)` for a `.send_to` site).",
        );
    }
    if findings.multi_level_event {
        notes.push(
            "//   - event group: a multi-level `path` has no single ttipc event group; rename the generated enum so its group matches your namespace.",
        );
    }
    if findings.alias_dropped {
        notes.push(
            "//   - alias: `#[taurpc(alias = \"..\")]` was dropped (ttipc names methods by `MethodCase`); rename the method or update the call sites if the alias mattered.",
        );
    }
    if findings.mount_rewritten {
        notes.push(
            "//   - mount: the TauRPC Router/handler became `ttipc::handler(..)` (a `-> Router` factory now returns `ttipc::Procedures`); generate bindings separately (`ttipc::Bindings`, replacing the dropped `export_config`), keep app state on `.manage(..)`, and drop any now-unused local config bindings.",
        );
    }
    if notes.is_empty() {
        return String::new();
    }
    format!(
        "// Migrated from TauRPC by ttipc-migrate. Manual follow-ups:\n{}\n\n",
        notes.join("\n")
    )
}

/// Does the trait carry a `#[taurpc::procedures(..)]` (or a bare imported
/// `#[procedures(..)]`) attribute?
fn has_taurpc_procedures(item_trait: &ItemTrait) -> bool {
    item_trait
        .attrs
        .iter()
        .any(|attr| is_taurpc_attr(attr, "procedures"))
}

fn has_taurpc_resolvers(item_impl: &ItemImpl) -> bool {
    item_impl
        .attrs
        .iter()
        .any(|attr| is_taurpc_attr(attr, "resolvers"))
}

/// A `#[taurpc::<name>(..)]` attribute (or a bare imported `#[<name>(..)]`).
fn is_taurpc_attr(attr: &Attribute, name: &str) -> bool {
    let segments = &attr.path().segments;
    let last_matches = segments.last().is_some_and(|seg| seg.ident == name);
    let rooted_in_taurpc = segments.first().is_some_and(|seg| seg.ident == "taurpc");
    last_matches && (rooted_in_taurpc || segments.len() == 1)
}

/// Transform the trait in place and, if it had any `#[taurpc(event)]` methods,
/// return the `#[derive(Event)]` enum they were lifted into.
fn transform_trait(
    item_trait: &mut ItemTrait,
    deasyncable: &HashSet<(String, String)>,
    registry: &EventRegistry,
    findings: &mut Findings,
) -> Option<Item> {
    // Read the namespace from the TauRPC attr before rewriting it, for the
    // event group below.
    let namespace = trait_namespace(item_trait);
    // Resolve the payload-enum pattern before the attr is rewritten (the trigger
    // name reads the TauRPC `event_trigger`, which the rewrite drops).
    let payload_enum = registry
        .triggers
        .get(&trait_trigger_name(item_trait))
        .filter(|info| info.payload_method.is_some())
        .map(|info| info.enum_name.clone());
    for attr in &mut item_trait.attrs {
        if is_taurpc_attr(attr, "procedures") {
            rewrite_procedures_attr(attr);
        }
    }
    // Lift the `#[taurpc(event)]` methods out: ttipc events are a separate
    // `#[derive(Event)]` enum, not procedures.
    let mut event_methods: Vec<syn::TraitItemFn> = Vec::new();
    item_trait.items.retain(|item| {
        if let TraitItem::Fn(method) = item
            && is_event_method(method)
        {
            event_methods.push(method.clone());
            return false;
        }
        true
    });
    let trait_name = item_trait.ident.to_string();
    for item in &mut item_trait.items {
        if let TraitItem::Fn(method) = item {
            strip_alias(&mut method.attrs, findings);
            transform_signature(&mut method.sig, findings);
            deasync(&mut method.sig, &trait_name, deasyncable, findings);
            flag_async_injection(&method.sig, findings);
        }
    }
    if event_methods.is_empty() {
        return None;
    }
    // Payload-enum pattern: the event method is dropped and the payload enum
    // itself derives `ttipc::Event` (added in `transform_ast` for an in-file
    // enum, flagged for an external one) -- so no wrapper enum is generated.
    if let Some(enum_name) = payload_enum {
        if registry.derive_on.contains(&enum_name) {
            findings.events_payload_enum = true;
        } else {
            findings.event_payload_external = true;
        }
        return None;
    }
    findings.events_lifted = true;
    Some(build_event_enum(
        item_trait,
        namespace.as_deref(),
        &event_methods,
        findings,
    ))
}

/// The `path`/`namespace` string of a `#[taurpc::procedures(..)]` attribute, if
/// any. Read before the attr is rewritten, so it matches on the TauRPC form.
fn trait_namespace(item_trait: &ItemTrait) -> Option<String> {
    let attr = item_trait
        .attrs
        .iter()
        .find(|attr| is_taurpc_attr(attr, "procedures"))?;
    let Meta::List(list) = &attr.meta else {
        return None;
    };
    let args = list
        .parse_args_with(Punctuated::<Meta, Comma>::parse_terminated)
        .ok()?;
    args.iter().find_map(|meta| match meta {
        Meta::NameValue(nv) if is_namespace_arg(meta) => match &nv.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) => Some(s.value()),
            _ => None,
        },
        _ => None,
    })
}

/// Is this a `#[taurpc(event)]` method?
fn is_event_method(method: &syn::TraitItemFn) -> bool {
    method.attrs.iter().any(is_taurpc_event_attr)
}

/// A `#[taurpc(event)]` helper attribute: a `taurpc(..)` list whose args
/// include `event`.
fn is_taurpc_event_attr(attr: &Attribute) -> bool {
    if !attr.path().is_ident("taurpc") {
        return false;
    }
    let Meta::List(list) = &attr.meta else {
        return false;
    };
    list.parse_args_with(Punctuated::<Meta, Comma>::parse_terminated)
        .map(|args| args.iter().any(|meta| meta.path().is_ident("event")))
        .unwrap_or(false)
}

/// Drop a `#[taurpc(alias = "..")]` helper attr; ttipc has no per-method
/// alias (it names methods by `MethodCase`).
fn strip_alias(attrs: &mut Vec<Attribute>, findings: &mut Findings) {
    let before = attrs.len();
    attrs.retain(|attr| !is_taurpc_alias_attr(attr));
    if attrs.len() != before {
        findings.alias_dropped = true;
    }
}

/// A `#[taurpc(alias = "..")]` helper attribute: a `taurpc(..)` list whose args
/// include an `alias` name-value.
fn is_taurpc_alias_attr(attr: &Attribute) -> bool {
    if !attr.path().is_ident("taurpc") {
        return false;
    }
    let Meta::List(list) = &attr.meta else {
        return false;
    };
    list.parse_args_with(Punctuated::<Meta, Comma>::parse_terminated)
        .map(|args| args.iter().any(|meta| meta.path().is_ident("alias")))
        .unwrap_or(false)
}

/// Build the `#[derive(Event)]` enum for a trait's lifted event methods. The
/// enum name is chosen so ttipc's group (name minus a trailing `Event`,
/// lower-camel) equals the trait's namespace, so the bindings factory wires
/// `{namespace}.event.on`.
fn build_event_enum(
    item_trait: &ItemTrait,
    namespace: Option<&str>,
    event_methods: &[syn::TraitItemFn],
    findings: &mut Findings,
) -> Item {
    if namespace.is_some_and(|ns| ns.contains('.')) {
        findings.multi_level_event = true;
    }
    let enum_name = event_enum_name(&item_trait.ident.to_string(), namespace);
    let enum_ident = syn::Ident::new(&enum_name, item_trait.ident.span());
    let vis = &item_trait.vis;
    let variants: Vec<syn::Variant> = event_methods.iter().map(event_variant).collect();
    syn::parse_quote! {
        #[derive(ttipc::Event)]
        #vis enum #enum_ident {
            #(#variants),*
        }
    }
}

/// The event enum name: chosen so ttipc's group (name minus a trailing
/// `Event`, lower-camel) equals the trait's single-level `path`. A multi-level
/// path has no single group, so it falls back to the trait name.
fn event_enum_name(trait_ident: &str, namespace: Option<&str>) -> String {
    let base = match namespace {
        Some(ns) if !ns.contains('.') => pascal_case(ns),
        _ => trait_ident.to_string(),
    };
    format!("{base}Event")
}

/// One event method becomes one enum variant: its name PascalCased, its params
/// the variant's named fields (a unit variant when it takes none).
fn event_variant(method: &syn::TraitItemFn) -> syn::Variant {
    let span = method.sig.ident.span();
    let variant_ident = syn::Ident::new(&pascal_case(&method.sig.ident.to_string()), span);
    let fields = event_fields(method);
    if fields.is_empty() {
        syn::parse_quote!(#variant_ident)
    } else {
        let names: Vec<&syn::Ident> = fields.iter().map(|(name, _)| name).collect();
        let types: Vec<&syn::Type> = fields.iter().map(|(_, ty)| ty).collect();
        syn::parse_quote!(#variant_ident { #(#names: #types),* })
    }
}

/// The `(name, type)` of each named parameter of an event method.
fn event_fields(method: &syn::TraitItemFn) -> Vec<(syn::Ident, syn::Type)> {
    method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(pat_type) => match &*pat_type.pat {
                syn::Pat::Ident(pat_ident) => {
                    Some((pat_ident.ident.clone(), (*pat_type.ty).clone()))
                }
                _ => None,
            },
            FnArg::Receiver(_) => None,
        })
        .collect()
}

/// `state_changed` -> `StateChanged`, `ev` -> `Ev`.
fn pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect()
}

/// Maps each TauRPC event-trigger type to the ttipc enum its events were
/// lifted into, so an emit site resolves even when the trait is in another file.
#[derive(Default)]
struct EventRegistry {
    triggers: HashMap<String, TriggerInfo>,
    /// Every enum defined across the migrated files, by name -- lets the
    /// payload-enum event pattern tell an in-file enum (derive on it directly)
    /// from an external one (flag it for a manual derive).
    enums: HashSet<String>,
    /// In-file payload enums that get `#[derive(ttipc::Event)]` added.
    derive_on: HashSet<String>,
    /// Router-factory function names (fns returning `Router<..>`), so a
    /// cross-file `factory().into_handler()` consumer resolves to
    /// `ttipc::handler(factory())`.
    router_factories: HashSet<String>,
}

struct TriggerInfo {
    enum_name: String,
    /// event-method name -> the variant it became (the per-method model).
    methods: HashMap<String, EventVariantInfo>,
    /// Set when the trait used the payload-enum pattern: one `#[taurpc(event)]`
    /// method (this name) carrying the whole `enum_name`. Emit sites become
    /// `<payload>.emit(&h)`, and `enum_name` itself derives `ttipc::Event`.
    payload_method: Option<String>,
}

struct EventVariantInfo {
    variant: String,
    /// Field names in declaration order (empty for a unit event).
    fields: Vec<String>,
}

/// Record every `#[taurpc(event)]`-bearing trait's trigger -> enum/variant
/// mapping. Read-only and run before the file is transformed; the multi-file
/// driver folds several files' events into one registry.
fn extract_events(file: &syn::File, registry: &mut EventRegistry) {
    for item in &file.items {
        if let Item::Trait(item_trait) = item
            && has_taurpc_procedures(item_trait)
        {
            let namespace = trait_namespace(item_trait);
            // The payload-enum pattern (one event method carrying a whole enum)
            // takes precedence: the existing enum becomes the event channel,
            // not a generated wrapper.
            if let Some((method, enum_name)) = payload_enum_event(item_trait, namespace.as_deref())
            {
                if registry.enums.contains(&enum_name) {
                    registry.derive_on.insert(enum_name.clone());
                }
                registry.triggers.insert(
                    trait_trigger_name(item_trait),
                    TriggerInfo {
                        enum_name,
                        methods: HashMap::new(),
                        payload_method: Some(method),
                    },
                );
                continue;
            }
            // The per-method model: each event method becomes one enum variant.
            let methods: HashMap<String, EventVariantInfo> = item_trait
                .items
                .iter()
                .filter_map(|it| match it {
                    TraitItem::Fn(method) if is_event_method(method) => {
                        let name = method.sig.ident.to_string();
                        let info = EventVariantInfo {
                            variant: pascal_case(&name),
                            fields: event_fields(method)
                                .into_iter()
                                .map(|(ident, _)| ident.to_string())
                                .collect(),
                        };
                        Some((name, info))
                    }
                    _ => None,
                })
                .collect();
            if methods.is_empty() {
                continue;
            }
            let enum_name = event_enum_name(&item_trait.ident.to_string(), namespace.as_deref());
            registry.triggers.insert(
                trait_trigger_name(item_trait),
                TriggerInfo {
                    enum_name,
                    methods,
                    payload_method: None,
                },
            );
        }
    }
}

/// Collect the name of every enum defined in `file`.
fn collect_enums(file: &syn::File, enums: &mut HashSet<String>) {
    for item in &file.items {
        if let Item::Enum(item_enum) = item {
            enums.insert(item_enum.ident.to_string());
        }
    }
}

/// Collect the name of every Router-factory fn (one returning `Router<..>`) in
/// `file`, so a `factory().into_handler()` call in another file resolves.
fn collect_router_factories(file: &syn::File, factories: &mut HashSet<String>) {
    for item in &file.items {
        if let Item::Fn(item_fn) = item
            && returns_router(&item_fn.sig.output)
        {
            factories.insert(item_fn.sig.ident.to_string());
        }
    }
}

/// The payload-enum event pattern: a trait whose ONLY `#[taurpc(event)]` method
/// has exactly one non-injected parameter whose type is the namespace's event
/// enum (`{Pascal(namespace)}Event` -- the inverse of [`event_enum_name`], which
/// excludes primitives and guarantees the bindings factory's group wires). That
/// enum becomes the ttipc event channel rather than a generated wrapper.
/// Returns the event method name and the payload enum name.
fn payload_enum_event(item_trait: &ItemTrait, namespace: Option<&str>) -> Option<(String, String)> {
    let ns = namespace?;
    if ns.contains('.') {
        return None;
    }
    let mut events = item_trait.items.iter().filter_map(|it| match it {
        TraitItem::Fn(method) if is_event_method(method) => Some(method),
        _ => None,
    });
    let method = events.next()?;
    if events.next().is_some() {
        return None; // more than one event method: the per-method model fits better
    }
    let mut payloads = method.sig.inputs.iter().filter_map(|arg| match arg {
        FnArg::Typed(pat_type) if !is_injected_ty(&pat_type.ty) => Some(&*pat_type.ty),
        _ => None,
    });
    let ty = payloads.next()?;
    if payloads.next().is_some() {
        return None; // more than one payload param: not a single payload enum
    }
    let enum_name = last_segment_ident(ty)?;
    (enum_name == format!("{}Event", pascal_case(ns)))
        .then(|| (method.sig.ident.to_string(), enum_name))
}

/// Is this an injected parameter type (`AppHandle`/`State`/`Window`/
/// `WebviewWindow`), which ttipc supplies rather than carrying as payload?
fn is_injected_ty(ty: &Type) -> bool {
    last_segment_is(ty, "AppHandle")
        || last_segment_is(ty, "State")
        || last_segment_is(ty, "Window")
        || last_segment_is(ty, "WebviewWindow")
}

/// The last path segment's ident as a string (`a::b::Foo<T>` -> `Foo`).
fn last_segment_ident(ty: &Type) -> Option<String> {
    match ty {
        Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .map(|seg| seg.ident.to_string()),
        _ => None,
    }
}

/// The event-trigger type a trait emits through: a custom `event_trigger = X`, or
/// TauRPC's default `TauRpc{Trait}EventTrigger`.
fn trait_trigger_name(item_trait: &ItemTrait) -> String {
    custom_event_trigger(item_trait)
        .unwrap_or_else(|| format!("TauRpc{}EventTrigger", item_trait.ident))
}

fn custom_event_trigger(item_trait: &ItemTrait) -> Option<String> {
    let attr = item_trait
        .attrs
        .iter()
        .find(|attr| is_taurpc_attr(attr, "procedures"))?;
    let Meta::List(list) = &attr.meta else {
        return None;
    };
    let args = list
        .parse_args_with(Punctuated::<Meta, Comma>::parse_terminated)
        .ok()?;
    args.iter().find_map(|meta| match meta {
        Meta::NameValue(nv) if nv.path.is_ident("event_trigger") => match &nv.value {
            Expr::Path(path) => path.path.segments.last().map(|seg| seg.ident.to_string()),
            _ => None,
        },
        _ => None,
    })
}

/// Drop `#[taurpc::resolvers]`, rewrite each method signature the same way the
/// trait's are, and keep the bodies and other attributes (e.g. `#[instrument]`).
fn transform_resolver_impl(
    item_impl: &mut ItemImpl,
    deasyncable: &HashSet<(String, String)>,
    findings: &mut Findings,
) {
    item_impl
        .attrs
        .retain(|attr| !is_taurpc_attr(attr, "resolvers"));
    let trait_name = impl_trait_name(item_impl).unwrap_or_default();
    let self_ty = self_type_ident(item_impl);
    // The method names being made sync in this impl, so their `.await`s can drop.
    let deasynced_here: HashSet<String> = deasyncable
        .iter()
        .filter(|(t, _)| *t == trait_name)
        .map(|(_, m)| m.clone())
        .collect();
    for item in &mut item_impl.items {
        if let ImplItem::Fn(method) = item {
            transform_signature(&mut method.sig, findings);
            deasync(&mut method.sig, &trait_name, deasyncable, findings);
            strip_sibling_awaits(&mut method.block, self_ty.as_ref(), &deasynced_here);
            flag_async_injection(&method.sig, findings);
        }
    }
}

/// The last path segment of the impl's self type, e.g. `SyncEndpoint` in
/// `impl SyncMethods for SyncEndpoint`.
fn self_type_ident(item_impl: &ItemImpl) -> Option<syn::Ident> {
    match &*item_impl.self_ty {
        Type::Path(type_path) => type_path.path.segments.last().map(|seg| seg.ident.clone()),
        _ => None,
    }
}

/// Flag a method that stays `async` while taking an injected `AppHandle`/`State`:
/// ttipc rejects that pair, so it needs manual rework.
fn flag_async_injection(sig: &Signature, findings: &mut Findings) {
    if sig.asyncness.is_some() && has_injected_param(sig) {
        findings.async_injection = true;
    }
}

/// Does the signature take an injected `AppHandle` or managed `State<T>`?
fn has_injected_param(sig: &Signature) -> bool {
    sig.inputs.iter().any(|input| {
        matches!(input, FnArg::Typed(pat_type)
            if last_segment_is(&pat_type.ty, "AppHandle") || last_segment_is(&pat_type.ty, "State"))
    })
}

fn last_segment_is(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(type_path)
        if type_path.path.segments.last().is_some_and(|seg| seg.ident == name))
}

/// Make a method sync when it is in the de-asyncable set (see
/// [`collect_deasyncable`]).
fn deasync(
    sig: &mut Signature,
    trait_name: &str,
    deasyncable: &HashSet<(String, String)>,
    findings: &mut Findings,
) {
    if sig.asyncness.is_some()
        && deasyncable.contains(&(trait_name.to_string(), sig.ident.to_string()))
    {
        sig.asyncness = None;
        findings.deasynced = true;
    }
}

/// The `(trait, method)` pairs that can be made sync. A resolver method is
/// de-asyncable when its body has no blocking `.await` -- one on anything but a
/// call to a de-asyncable sibling. This is transitive: a method awaiting only
/// siblings that do no real async work is itself de-asyncable. Only same-file
/// resolver impls are visible; a method whose impl is elsewhere stays `async`
/// (the safe default).
fn collect_deasyncable(file: &syn::File) -> HashSet<(String, String)> {
    let mut set = HashSet::new();
    for item in &file.items {
        if let Item::Impl(item_impl) = item
            && has_taurpc_resolvers(item_impl)
            && let Some(trait_name) = impl_trait_name(item_impl)
        {
            for method in deasyncable_in_impl(item_impl) {
                set.insert((trait_name.clone(), method));
            }
        }
    }
    set
}

/// The async method names in one resolver impl that can be made sync. A method
/// qualifies when it has no blocking `.await` and every sibling it awaits also
/// qualifies; the fixed point below shrinks the candidate set until it is stable.
fn deasyncable_in_impl(item_impl: &ItemImpl) -> HashSet<String> {
    let self_ty = self_type_ident(item_impl);
    let async_methods: HashSet<String> = item_impl
        .items
        .iter()
        .filter_map(async_method_name)
        .collect();

    // Per async method: does it await anything that is not a sibling call
    // (blocking), and which siblings does it await?
    let mut blocking: HashSet<String> = HashSet::new();
    let mut awaited_siblings: HashMap<String, HashSet<String>> = HashMap::new();
    for impl_item in &item_impl.items {
        if let ImplItem::Fn(method) = impl_item
            && method.sig.asyncness.is_some()
        {
            let mut classifier = AwaitClassifier {
                self_ty: self_ty.as_ref(),
                siblings: &async_methods,
                blocking: false,
                targets: HashSet::new(),
            };
            classifier.visit_block(&method.block);
            let name = method.sig.ident.to_string();
            if classifier.blocking {
                blocking.insert(name.clone());
            }
            awaited_siblings.insert(name, classifier.targets);
        }
    }

    // Start from every async method with no blocking await, then drop any whose
    // awaited siblings are not (or no longer) de-asyncable until nothing changes.
    let mut deasyncable: HashSet<String> = async_methods
        .into_iter()
        .filter(|m| !blocking.contains(m))
        .collect();
    loop {
        let before = deasyncable.len();
        let current = deasyncable.clone();
        deasyncable.retain(|m| awaited_siblings[m].iter().all(|t| current.contains(t)));
        if deasyncable.len() == before {
            break;
        }
    }
    deasyncable
}

/// The method name of an async impl method.
fn async_method_name(item: &ImplItem) -> Option<String> {
    match item {
        ImplItem::Fn(method) if method.sig.asyncness.is_some() => {
            Some(method.sig.ident.to_string())
        }
        _ => None,
    }
}

/// Classifies the `.await`s in one method body: each is either a call to a
/// sibling resolver (a `target`) or `blocking` (anything else -- a real future).
struct AwaitClassifier<'a> {
    self_ty: Option<&'a syn::Ident>,
    siblings: &'a HashSet<String>,
    blocking: bool,
    targets: HashSet<String>,
}

impl<'a, 'ast> Visit<'ast> for AwaitClassifier<'a> {
    fn visit_expr_await(&mut self, node: &'ast syn::ExprAwait) {
        match self_call_method(&node.base, self.self_ty) {
            Some(name) if self.siblings.contains(&name) => {
                self.targets.insert(name);
            }
            _ => self.blocking = true,
        }
        syn::visit::visit_expr_await(self, node);
    }
}

/// The trait name of an `impl Trait for Struct` block (its last path segment).
fn impl_trait_name(item_impl: &ItemImpl) -> Option<String> {
    item_impl
        .trait_
        .as_ref()
        .and_then(|(_, path, _)| path.segments.last())
        .map(|seg| seg.ident.to_string())
}

/// If `base` is `self.m(..)` or `<SelfType>.m(..)`, the method name `m`. The
/// receiver must be `self` or the impl's own type, so a same-named method on an
/// unrelated value is never matched -- the basis for safely stripping `.await`.
fn self_call_method(base: &Expr, self_ty: Option<&syn::Ident>) -> Option<String> {
    let Expr::MethodCall(call) = base else {
        return None;
    };
    receiver_is_self_or(&call.receiver, self_ty).then(|| call.method.to_string())
}

fn receiver_is_self_or(receiver: &Expr, self_ty: Option<&syn::Ident>) -> bool {
    let Expr::Path(path) = receiver else {
        return false;
    };
    match path.path.segments.last() {
        Some(seg) => seg.ident == "self" || self_ty.is_some_and(|ty| seg.ident == *ty),
        None => false,
    }
}

/// Drop `.await` from every call to a de-asynced sibling in this body, so a now-
/// sync call is no longer awaited (e.g. `self.sync_buffer(b).await?` becomes
/// `self.sync_buffer(b)?`). Receiver-matched via [`self_call_method`], so only
/// sibling calls are touched -- a real future is never stripped.
fn strip_sibling_awaits(
    block: &mut syn::Block,
    self_ty: Option<&syn::Ident>,
    deasynced: &HashSet<String>,
) {
    let mut stripper = AwaitStripper { self_ty, deasynced };
    stripper.visit_block_mut(block);
}

struct AwaitStripper<'a> {
    self_ty: Option<&'a syn::Ident>,
    deasynced: &'a HashSet<String>,
}

impl VisitMut for AwaitStripper<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        syn::visit_mut::visit_expr_mut(self, expr);
        if let Expr::Await(await_expr) = expr
            && let Some(name) = self_call_method(&await_expr.base, self.self_ty)
            && self.deasynced.contains(&name)
        {
            *expr = (*await_expr.base).clone();
        }
    }
}

/// `#[taurpc::procedures(path = "x", export_to = "..", event_trigger = X)]`
/// becomes `#[ttipc::procedures(path = "x")]`. ttipc accepts `path` as a
/// `namespace` alias; `export_to` and `event_trigger` map to ttipc's separate
/// `Bindings::export_to` and `#[derive(Event)]`, so they are dropped.
fn rewrite_procedures_attr(attr: &mut Attribute) {
    let kept: Punctuated<Meta, Comma> = match &attr.meta {
        Meta::List(list) => list
            .parse_args_with(Punctuated::<Meta, Comma>::parse_terminated)
            .map(|args| args.into_iter().filter(is_namespace_arg).collect())
            .unwrap_or_default(),
        _ => Punctuated::new(),
    };
    attr.meta = if kept.is_empty() {
        syn::parse_quote!(ttipc::procedures)
    } else {
        syn::parse_quote!(ttipc::procedures(#kept))
    };
}

fn is_namespace_arg(meta: &Meta) -> bool {
    meta.path().is_ident("path") || meta.path().is_ident("namespace")
}

/// Drop the `<R: Runtime>` generics, give the method a `&self` receiver, and
/// rewrite the injected and streaming parameter types.
fn transform_signature(sig: &mut Signature, findings: &mut Findings) {
    if !sig.generics.params.is_empty() {
        findings.dropped_generics = true;
    }
    sig.generics.params = Punctuated::new();
    sig.generics.where_clause = None;

    if let syn::ReturnType::Type(_, ty) = &sig.output
        && let Type::Path(type_path) = &**ty
        && type_path
            .path
            .segments
            .last()
            .is_some_and(|seg| seg.ident == "Result")
    {
        findings.result_returns = true;
    }

    let has_receiver = matches!(sig.inputs.first(), Some(FnArg::Receiver(_)));
    let by_value_self =
        matches!(sig.inputs.first(), Some(FnArg::Receiver(r)) if r.reference.is_none());
    if !has_receiver {
        // The trait definition omits the receiver; ttipc wants `&self`.
        sig.inputs.insert(0, syn::parse_quote!(&self));
    } else if by_value_self {
        // The resolver impl takes `self` by value; ttipc wants `&self`.
        if let Some(first) = sig.inputs.first_mut() {
            *first = syn::parse_quote!(&self);
        }
    }

    for input in &mut sig.inputs {
        if let FnArg::Typed(pat_type) = input {
            rewrite_type(&mut pat_type.ty, findings);
        }
    }
}

/// `AppHandle<R>` becomes `AppHandle` (ttipc injects it by type);
/// `Window<R>`/`WebviewWindow<R>` lose their now-dangling `<R>` too, but ttipc
/// does not inject them, so they are also flagged; `Channel<T>` becomes
/// `ttipc::Channel<T>` (the wrapper that erases the runtime).
fn rewrite_type(ty: &mut Type, findings: &mut Findings) {
    let Type::Path(type_path) = ty else { return };
    let Some(last) = type_path.path.segments.last_mut() else {
        return;
    };
    if last.ident == "AppHandle" {
        last.arguments = PathArguments::None;
    } else if last.ident == "Window" || last.ident == "WebviewWindow" {
        // The `<R: Runtime>` generic was dropped, so strip the `<R>` here to
        // avoid a dangling type parameter; the bare type defaults to `Wry`.
        last.arguments = PathArguments::None;
        findings.window_param = true;
    } else if last.ident == "Channel" {
        findings.channels = true;
        let arguments = last.arguments.clone();
        type_path.path = syn::parse_quote!(ttipc::Channel);
        if let Some(last) = type_path.path.segments.last_mut() {
            last.arguments = arguments;
        }
    }
}

/// Replace a wire DTO's `#[taurpc::ipc_type]` with the derives it expands to.
fn convert_ipc_type(attrs: &mut [Attribute], findings: &mut Findings) {
    for attr in attrs.iter_mut() {
        if is_taurpc_attr(attr, "ipc_type") {
            *attr = syn::parse_quote!(
                #[derive(serde::Serialize, serde::Deserialize, specta::Type, Clone)]
            );
            findings.ipc_type_converted = true;
        }
    }
}

/// Rewrite a TauRPC mount into ttipc's. Only runs when the file actually
/// mounts TauRPC (a `Router::new()` or `create_ipc_handler` call), so an
/// unrelated `into_handler` elsewhere is never touched. Two passes:
/// 1. collapse the `Router::new().export_config(..).merge(X.into_handler())..`
///    chain into `X.into_procedures().merge(..)`, and turn
///    `create_ipc_handler(X.into_handler())` into `ttipc::handler(..)`;
/// 2. the remaining `<router>.into_handler()` becomes `ttipc::handler(router)`.
fn rewrite_mount(file: &mut syn::File, findings: &mut Findings) {
    if !has_taurpc_mount(file) {
        return;
    }
    MountChains.visit_file_mut(file);
    RouterHandlers.visit_file_mut(file);
    // A `build() -> Router<R>` factory now returns `ttipc::Procedures` (the
    // collapsed `X.into_procedures().merge(..)` chain), its `<R>` dropped.
    for item in &mut file.items {
        if let Item::Fn(item_fn) = item
            && returns_router(&item_fn.sig.output)
        {
            item_fn.sig.generics.params = Punctuated::new();
            item_fn.sig.generics.where_clause = None;
            item_fn.sig.output = syn::parse_quote!(-> ttipc::Procedures);
        }
    }
    findings.transformed = true;
    findings.mount_rewritten = true;
}

/// Does this function return a TauRPC `Router<..>` (the factory shape)?
fn returns_router(output: &syn::ReturnType) -> bool {
    matches!(output, syn::ReturnType::Type(_, ty) if last_segment_is(ty, "Router"))
}

/// Rewrite cross-file mount consumers: `factory().into_handler()` (a call to a
/// known Router-factory fn) -> `ttipc::handler(factory())`. Runs on every file
/// so a consumer resolves even when the factory is defined elsewhere; gated on a
/// known factory name, so an unrelated `.into_handler()` is never touched.
fn rewrite_factory_consumers(
    file: &mut syn::File,
    factories: &HashSet<String>,
    findings: &mut Findings,
) {
    if factories.is_empty() {
        return;
    }
    let mut rewriter = FactoryConsumerRewriter {
        factories,
        findings,
    };
    rewriter.visit_file_mut(file);
}

struct FactoryConsumerRewriter<'a> {
    factories: &'a HashSet<String>,
    findings: &'a mut Findings,
}

impl VisitMut for FactoryConsumerRewriter<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        syn::visit_mut::visit_expr_mut(self, expr);
        if let Some(replacement) = factory_consumer_replacement(expr, self.factories) {
            *expr = replacement;
            self.findings.transformed = true;
            self.findings.mount_rewritten = true;
        }
    }
}

/// If `expr` is `factory(..).into_handler()` for a known Router-factory fn, the
/// `ttipc::handler(factory(..))` that replaces it.
fn factory_consumer_replacement(expr: &Expr, factories: &HashSet<String>) -> Option<Expr> {
    let Expr::MethodCall(call) = expr else {
        return None;
    };
    if call.method != "into_handler" || !calls_known_factory(&call.receiver, factories) {
        return None;
    }
    let receiver = (*call.receiver).clone();
    Some(syn::parse_quote!(ttipc::handler(#receiver)))
}

/// Is `expr` a call to a known Router-factory fn (`build()`, `router::build()`)?
fn calls_known_factory(expr: &Expr, factories: &HashSet<String>) -> bool {
    matches!(expr, Expr::Call(call)
        if matches!(&*call.func, Expr::Path(path)
            if path.path.segments.last().is_some_and(|seg| factories.contains(&seg.ident.to_string()))))
}

/// Does the file mount TauRPC -- a `Router::new()` or `create_ipc_handler` call?
fn has_taurpc_mount(file: &syn::File) -> bool {
    let mut scan = MountScan { found: false };
    scan.visit_file(file);
    scan.found
}

struct MountScan {
    found: bool,
}

impl<'ast> Visit<'ast> for MountScan {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        if is_router_new(expr) {
            self.found = true;
        }
        if let Expr::Call(call) = expr
            && is_create_ipc_handler(&call.func)
        {
            self.found = true;
        }
        syn::visit::visit_expr(self, expr);
    }
}

/// Pass 1: the Router builder chain and `create_ipc_handler`.
struct MountChains;

impl VisitMut for MountChains {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        syn::visit_mut::visit_expr_mut(self, expr);
        match expr {
            // `Router::new().merge(X.into_handler())` -> `X.into_procedures()`;
            // a later `.merge(Y.into_handler())` -> `.merge(Y.into_procedures())`.
            Expr::MethodCall(call) if call.method == "merge" => {
                rewrite_into_handler_arg(&mut call.args);
                if is_router_new(&call.receiver)
                    && let Some(arg) = call.args.first().cloned()
                {
                    *expr = arg;
                }
            }
            // Any other Router builder method (`export_config`, `semantic_types`,
            // `dangerously_cast_bigints_to_number`, ...) has no ttipc
            // equivalent: drop it so the chain collapses to the merges. The
            // structural `into_handler`/`into_procedures` are left for the merge
            // arm and pass 2.
            Expr::MethodCall(call)
                if roots_in_router_new(&call.receiver)
                    && call.method != "into_handler"
                    && call.method != "into_procedures" =>
            {
                *expr = (*call.receiver).clone();
            }
            // `create_ipc_handler(X.into_handler())` -> `ttipc::handler(..)`.
            Expr::Call(call) if is_create_ipc_handler(&call.func) => {
                rewrite_into_handler_arg(&mut call.args);
                if let Some(arg) = call.args.first().cloned() {
                    *expr = syn::parse_quote!(ttipc::handler(#arg));
                }
            }
            _ => {}
        }
    }
}

/// Pass 2: the leftover `<router>.into_handler()` (pass 1 already turned every
/// impl `into_handler` into `into_procedures`).
struct RouterHandlers;

impl VisitMut for RouterHandlers {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        syn::visit_mut::visit_expr_mut(self, expr);
        if let Expr::MethodCall(call) = expr
            && call.method == "into_handler"
        {
            let receiver = (*call.receiver).clone();
            *expr = syn::parse_quote!(ttipc::handler(#receiver));
        }
    }
}

/// If the first argument is `X.into_handler()`, rename it to `X.into_procedures()`.
fn rewrite_into_handler_arg(args: &mut Punctuated<Expr, Comma>) {
    if let Some(Expr::MethodCall(call)) = args.first_mut()
        && call.method == "into_handler"
    {
        call.method = syn::Ident::new("into_procedures", call.method.span());
    }
}

/// A `<..>::Router::new()` call (the TauRPC router constructor).
fn is_router_new(expr: &Expr) -> bool {
    let Expr::Call(call) = expr else {
        return false;
    };
    let Expr::Path(path) = &*call.func else {
        return false;
    };
    let segments = &path.path.segments;
    let n = segments.len();
    n >= 2 && segments[n - 1].ident == "new" && segments[n - 2].ident == "Router"
}

/// Does the receiver chain root in `Router::new()`? (`Router::new().a().b()` --
/// the base of any `.b()` is the `Router::new()` call.)
fn roots_in_router_new(expr: &Expr) -> bool {
    let mut current = expr;
    loop {
        if is_router_new(current) {
            return true;
        }
        match current {
            Expr::MethodCall(call) => current = &call.receiver,
            _ => return false,
        }
    }
}

/// A `<..>::create_ipc_handler` function path (the router-less TauRPC mount).
fn is_create_ipc_handler(func: &Expr) -> bool {
    matches!(func, Expr::Path(path)
        if path.path.segments.last().is_some_and(|seg| seg.ident == "create_ipc_handler"))
}

/// Rewrite emit sites through the registry: `Trigger::new(h).method(args)` into
/// `Enum::Variant { .. }.emit(&h)`, and `Trigger::new(h).send_to(target)
/// .method(args)` into `.emit_to(&h, target)`. A trailing `.unwrap()`/`?`/
/// `.map_err(..)` rides along (the receiver is what we replace). A registered
/// trigger bound to a variable (its handle not inline) is flagged for manual
/// rework.
fn rewrite_emits(file: &mut syn::File, registry: &EventRegistry, findings: &mut Findings) {
    if registry.triggers.is_empty() {
        return;
    }
    let mut rewriter = EmitRewriter { registry, findings };
    rewriter.visit_file_mut(file);
    if has_unrewritten_trigger(file, registry) {
        findings.emits_unresolved = true;
        findings.transformed = true;
    }
}

struct EmitRewriter<'a> {
    registry: &'a EventRegistry,
    findings: &'a mut Findings,
}

impl VisitMut for EmitRewriter<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        syn::visit_mut::visit_expr_mut(self, expr);
        if let Some(emit) = emit_replacement(expr, self.registry) {
            *expr = emit;
            self.findings.emits_rewritten = true;
            self.findings.transformed = true;
        }
    }
}

/// If `expr` is `Trigger::new(h).method(args)` (broadcast) or
/// `Trigger::new(h).send_to(target).method(args)` (targeted) for a registered
/// trigger+method with matching arity, the `Enum::Variant { .. }.emit(&h)` or
/// `.emit_to(&h, target)` that replaces it.
fn emit_replacement(expr: &Expr, registry: &EventRegistry) -> Option<Expr> {
    let Expr::MethodCall(call) = expr else {
        return None;
    };
    let (trigger, handle, target) = trigger_chain(&call.receiver)?;
    let info = registry.triggers.get(&trigger)?;
    // Payload-enum pattern: `Trigger::new(h).<method>(payload)` -- the payload
    // value itself is the event, so `payload.emit(&h)` / `.emit_to(&h, target)`.
    if let Some(method) = &info.payload_method {
        if call.method == *method && call.args.len() == 1 {
            let payload = &call.args[0];
            return Some(match target {
                Some(target) => syn::parse_quote!(#payload.emit_to(&#handle, #target)),
                None => syn::parse_quote!(#payload.emit(&#handle)),
            });
        }
        return None;
    }
    let variant = info.methods.get(&call.method.to_string())?;
    if call.args.len() != variant.fields.len() {
        return None;
    }
    let span = call.method.span();
    let enum_ident = syn::Ident::new(&info.enum_name, span);
    let variant_ident = syn::Ident::new(&variant.variant, span);
    let event: Expr = if variant.fields.is_empty() {
        syn::parse_quote!(#enum_ident::#variant_ident)
    } else {
        let names: Vec<syn::Ident> = variant
            .fields
            .iter()
            .map(|field| syn::Ident::new(field, span))
            .collect();
        let args: Vec<&Expr> = call.args.iter().collect();
        syn::parse_quote!(#enum_ident::#variant_ident { #(#names: #args),* })
    };
    // taurpc's `send_to(target)` and ttipc's `emit_to(&h, target)` share the
    // `I: Into<tauri::EventTarget>` bound, so the target expression carries over.
    Some(match target {
        Some(target) => syn::parse_quote!(#event.emit_to(&#handle, #target)),
        None => syn::parse_quote!(#event.emit(&#handle)),
    })
}

/// The trigger construction at the head of an emit site: `Trigger::new(h)`
/// (broadcast) or `Trigger::new(h).send_to(target)` (targeted). Returns the
/// trigger type name, its handle, and the optional `send_to` target. A trigger
/// bound to a variable is not matched here (its handle is not inline) -- those
/// stay flagged for manual rework.
fn trigger_chain(expr: &Expr) -> Option<(String, &Expr, Option<&Expr>)> {
    if let Expr::MethodCall(call) = expr
        && call.method == "send_to"
    {
        let (trigger, handle) = trigger_new(&call.receiver)?;
        let target = call.args.first()?;
        return Some((trigger, handle, Some(target)));
    }
    let (trigger, handle) = trigger_new(expr)?;
    Some((trigger, handle, None))
}

/// `<..>::<Trigger>::new(<handle>)` -> the trigger type name and its handle arg.
fn trigger_new(expr: &Expr) -> Option<(String, &Expr)> {
    let Expr::Call(call) = expr else {
        return None;
    };
    let Expr::Path(path) = &*call.func else {
        return None;
    };
    let segments = &path.path.segments;
    let n = segments.len();
    if n < 2 || segments[n - 1].ident != "new" {
        return None;
    }
    let trigger = segments[n - 2].ident.to_string();
    let handle = call.args.first()?;
    Some((trigger, handle))
}

/// Is a registered trigger still being constructed anywhere (an emit site the
/// direct rewrite could not reach)?
fn has_unrewritten_trigger(file: &syn::File, registry: &EventRegistry) -> bool {
    let mut scan = TriggerScan {
        registry,
        found: false,
    };
    scan.visit_file(file);
    scan.found
}

struct TriggerScan<'a> {
    registry: &'a EventRegistry,
    found: bool,
}

impl<'ast> Visit<'ast> for TriggerScan<'_> {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Some((trigger, _)) = trigger_new(expr)
            && self.registry.triggers.contains_key(&trigger)
        {
            self.found = true;
        }
        syn::visit::visit_expr(self, expr);
    }
}

#[cfg(test)]
mod tests {
    use super::{transform, transform_project};

    #[test]
    fn surgical_preserves_code_around_edits() {
        // Cross-file: the trigger `CmdBus` (declared in cmd.rs) resolves in
        // channels.rs via the project registry. The emit expression is replaced
        // surgically -- every comment and the surrounding domain code stay
        // byte-for-byte.
        let trait_file = r#"
#[taurpc::procedures(path = "cmd", event_trigger = CmdBus)]
pub trait Cmd {
    #[taurpc(event)]
    async fn updated(value: u8);
}
"#;
        let emit_file = r#"// domain logic -- keep this comment
pub struct Store;

impl Store {
    /// Save and notify.
    pub fn save(&self, app: AppHandle, value: u8) -> Result<(), String> {
        // emit the update
        CmdBus::new(app).updated(value).map_err(|e| e.to_string())?;
        Ok(()) // trailing comment
    }
}
"#;
        let out = transform_project(&[
            ("cmd.rs".into(), trait_file.into()),
            ("channels.rs".into(), emit_file.into()),
        ])
        .expect("valid Rust");
        let channels = &out[1].1;
        assert!(
            channels.contains("// domain logic -- keep this comment"),
            "leading comment lost:\n{channels}"
        );
        assert!(
            channels.contains("// emit the update"),
            "inner comment lost:\n{channels}"
        );
        assert!(
            channels.contains("Ok(()) // trailing comment"),
            "trailing comment lost:\n{channels}"
        );
        assert!(
            channels.contains("/// Save and notify."),
            "doc comment lost:\n{channels}"
        );
        assert!(
            channels.contains("CmdEvent::Updated { value: value }.emit(&app)")
                && !channels.contains("CmdBus::new"),
            "emit not rewritten:\n{channels}"
        );
        insta::assert_snapshot!(channels);
    }

    #[test]
    fn surgical_item_level_replaces_attrs_and_keeps_surroundings() {
        // The taurpc trait is re-emitted whole (its attr becomes
        // `#[ttipc::procedures]`, not duplicated), while the leading comment,
        // the `use`, and the unrelated fn around it stay byte-for-byte.
        let src = r#"// top comment
use tauri::AppHandle;

#[taurpc::procedures(path = "greet")]
pub trait Greet {
    async fn hello<R: Runtime>(app_handle: AppHandle<R>) -> Result<String, String>;
}

// bottom comment
fn unrelated() -> u8 { 7 } // keep me
"#;
        let out = transform_project(&[("lib.rs".into(), src.into())]).expect("valid Rust");
        let s = &out[0].1;
        assert!(
            s.contains("#[ttipc::procedures(path = \"greet\")]")
                && !s.contains("taurpc::procedures"),
            "attr not replaced in place:\n{s}"
        );
        assert!(
            s.contains("// top comment") && s.contains("// bottom comment"),
            "comments lost:\n{s}"
        );
        assert!(s.contains("use tauri::AppHandle;"), "use lost:\n{s}");
        assert!(
            s.contains("fn unrelated() -> u8 { 7 } // keep me"),
            "unrelated fn changed:\n{s}"
        );
        insta::assert_snapshot!(s);
    }

    #[test]
    fn trait_signature_transform() {
        // Drops `<R: Runtime>`, adds `&self`, bares the `AppHandle`, wraps the
        // `Channel`, leaves a plain arg and the `Result` return alone, swaps the
        // macro path.
        let out = transform(
            r#"
#[taurpc::procedures(path = "greeter")]
pub trait Greeter {
    async fn greet<R: Runtime>(app_handle: AppHandle<R>, name: String) -> Result<String>;
    async fn stream<R: Runtime>(app_handle: AppHandle<R>, sink: Channel<Tick>) -> Result<()>;
}
"#,
        )
        .expect("valid Rust");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn attr_keeps_only_the_namespace() {
        // `export_to` and `event_trigger` map to ttipc's separate Bindings
        // and Event derive, so the attr rewrite keeps only `path`.
        let out = transform(
            r#"
#[taurpc::procedures(path = "app", export_to = "../bindings.ts", event_trigger = AppTrigger)]
pub trait App {
    async fn ping<R: Runtime>(app_handle: AppHandle<R>) -> Result<()>;
}
"#,
        )
        .expect("valid Rust");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn leaves_non_taurpc_items_alone() {
        // A trait without the attribute is returned verbatim (modulo
        // prettyplease normalization), so the transform is scoped.
        let out = transform(
            r#"
pub trait Plain {
    fn untouched(&self) -> u8;
}
"#,
        )
        .expect("valid Rust");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn resolver_impl_transform() {
        // The `#[taurpc::resolvers]` impl drops its attr; each method gets
        // `&self` (taurpc takes `self` by value) with `<R>` and `AppHandle<R>`
        // collapsed, and the body plus `#[instrument]` carry over. The resolver
        // struct loses `#[taurpc::ipc_type]` and becomes a plain struct.
        let out = transform(
            r#"
#[taurpc::procedures(path = "greeter")]
pub trait Greeter {
    async fn greet<R: Runtime>(app_handle: AppHandle<R>, name: String) -> Result<String>;
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
"#,
        )
        .expect("valid Rust");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn no_header_when_nothing_needs_flagging() {
        // A procedure with no `Result`, no generics, and no channel leaves
        // nothing to flag, so no header is prepended even though the trait was
        // transformed.
        let out = transform(
            r#"
#[taurpc::procedures(path = "ping")]
pub trait Ping {
    async fn ping();
}
"#,
        )
        .expect("valid Rust");
        assert!(
            !out.contains("Manual follow-ups"),
            "unexpected header:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn deasyncs_methods_without_await() {
        // TauRPC is async-only. A resolver body that never `.await`s becomes a
        // sync `fn` (in both the trait and the impl); one that awaits a real
        // future (`fetch(..)`, not a sibling) stays `async` -- and since it keeps
        // its `AppHandle`, it is also flagged for manual rework. The header notes
        // both.
        let out = transform(
            r#"
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
"#,
        )
        .expect("valid Rust");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn deasync_is_transitive_and_strips_sibling_awaits() {
        // `load`/`save` do no real async work, so they de-async. `refresh` only
        // `.await`s those siblings, so it de-asyncs too (transitively) and its
        // `.await`s on them are dropped. All three end up sync, so none trips the
        // async-injection flag despite taking `AppHandle`. This is lux's `sync.rs`.
        let out = transform(
            r#"
#[taurpc::procedures(path = "store")]
pub trait Store {
    async fn load<R: Runtime>(app_handle: AppHandle<R>) -> Result<u8, String>;
    async fn save<R: Runtime>(app_handle: AppHandle<R>) -> Result<u8, String>;
    async fn refresh<R: Runtime>(app_handle: AppHandle<R>) -> Result<u8, String>;
}

#[taurpc::ipc_type]
pub struct StoreImpl;

#[taurpc::resolvers]
impl Store for StoreImpl {
    async fn load<R: Runtime>(self, app_handle: AppHandle<R>) -> Result<u8, String> {
        Ok(1)
    }
    async fn save<R: Runtime>(self, app_handle: AppHandle<R>) -> Result<u8, String> {
        Ok(2)
    }
    async fn refresh<R: Runtime>(self, app_handle: AppHandle<R>) -> Result<u8, String> {
        StoreImpl.load(app_handle.clone()).await?;
        StoreImpl.save(app_handle).await
    }
}
"#,
        )
        .expect("valid Rust");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn flags_async_procedure_that_keeps_apphandle() {
        // `watch` awaits a real future (`tick()`), so it cannot de-async, and it
        // keeps its `AppHandle` -- a pair ttipc rejects. The `.await` is left
        // intact and the header flags the method for manual rework.
        let out = transform(
            r#"
#[taurpc::procedures(path = "api")]
pub trait Api {
    async fn watch<R: Runtime>(app_handle: AppHandle<R>);
}

#[taurpc::ipc_type]
pub struct ApiImpl;

#[taurpc::resolvers]
impl Api for ApiImpl {
    async fn watch<R: Runtime>(self, app_handle: AppHandle<R>) {
        tick().await;
        let _ = app_handle;
    }
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("async injection"),
            "expected async-injection flag:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn bares_window_params_and_flags_them() {
        // Dropping `<R: Runtime>` would leave `Window<R>`/`WebviewWindow<R>` with a
        // dangling `R`, so the `<R>` is stripped (the bare type defaults to `Wry`).
        // ttipc does not inject these, so the header flags them for rework.
        let out = transform(
            r#"
#[taurpc::procedures(path = "ui")]
pub trait Ui {
    async fn get_window<R: Runtime>(window: Window<R>);
    async fn get_webview<R: Runtime>(webview_window: WebviewWindow<R>);
}

#[taurpc::ipc_type]
pub struct UiImpl;

#[taurpc::resolvers]
impl Ui for UiImpl {
    async fn get_window<R: Runtime>(self, window: Window<R>) {
        println!("{}", window.label());
    }
    async fn get_webview<R: Runtime>(self, webview_window: WebviewWindow<R>) {
        println!("{}", webview_window.label());
    }
}
"#,
        )
        .expect("valid Rust");
        assert!(
            !out.contains("<R>"),
            "expected the dangling `R` to be stripped:\n{out}"
        );
        assert!(out.contains("windows"), "expected the windows flag:\n{out}");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn converts_ipc_type_dtos_to_derives() {
        // A wire DTO's `#[taurpc::ipc_type]` becomes the derives it expands to,
        // doc comments preserved. The resolver struct (`#[derive(Clone)]`, the
        // handler) is not a wire type, so it is left untouched.
        let out = transform(
            r#"
#[taurpc::ipc_type]
pub struct User {
    /// The user's id
    uid: i32,
    name: String,
}

#[taurpc::procedures(path = "users")]
pub trait Users {
    async fn get<R: Runtime>(app_handle: AppHandle<R>) -> User;
}

#[derive(Clone)]
pub struct UsersImpl;

#[taurpc::resolvers]
impl Users for UsersImpl {
    async fn get<R: Runtime>(self, app_handle: AppHandle<R>) -> User {
        User { uid: 1, name: "a".into() }
    }
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("serde::Serialize") && out.contains("specta::Type"),
            "expected the ipc_type DTO to gain the derives:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn lifts_events_into_a_derive_event_enum() {
        // `#[taurpc(event)]` methods leave the procedures trait and become a
        // `#[derive(ttipc::Event)]` enum: one variant per method, PascalCased,
        // params as named fields and no params as a unit variant. The enum name
        // makes ttipc's group equal the trait's `path`. The real procedure
        // stays in the trait.
        let out = transform(
            r#"
#[taurpc::procedures(path = "app", event_trigger = AppTrigger)]
pub trait App {
    async fn ping() -> u8;

    #[taurpc(event)]
    async fn ready();

    #[taurpc(event)]
    async fn progress(percent: u8);

    #[taurpc(event)]
    async fn moved(x: i32, y: i32);
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("enum AppEvent") && out.contains("#[derive(ttipc::Event)]"),
            "expected an AppEvent enum:\n{out}"
        );
        assert!(out.contains("fn ping"), "the procedure should stay:\n{out}");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn lifts_events_under_a_multi_level_path() {
        // A multi-level `path` has no single ttipc event group, so the enum
        // falls back to the trait name and the header flags it for a rename. The
        // trait, left with no procedures, is empty.
        let out = transform(
            r#"
#[taurpc::procedures(path = "api.ui")]
pub trait UiApi {
    #[taurpc(event)]
    async fn refreshed();
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("enum UiApiEvent"),
            "expected the fallback name:\n{out}"
        );
        assert!(
            out.contains("event group"),
            "expected the multi-level flag:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn derives_event_on_an_in_file_payload_enum() {
        // The central-event pattern: ONE `#[taurpc(event)]` method carrying a
        // payload enum defined in the same file. The enum itself gains
        // `#[derive(ttipc::Event)]` (its variants are the events) -- no wrapper
        // enum, no name collision -- and the event method is dropped.
        let out = transform(
            r#"
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
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("#[derive(ttipc::Event)]") && out.contains("pub enum AppEvent"),
            "the payload enum should derive Event:\n{out}"
        );
        assert!(
            out.contains("AuthChanged(bool)") && !out.contains("event: AppEvent"),
            "the enum keeps its own variants, no wrapper field:\n{out}"
        );
        assert!(out.contains("fn ping"), "the real procedure stays:\n{out}");
        assert!(
            out.contains("payload enum"),
            "expected the payload-enum note:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn flags_an_external_payload_enum() {
        // The payload enum lives in another crate, so it cannot be derived here:
        // the event method is dropped and the header flags adding the derive by
        // hand. An injected `AppHandle<R>` on the event method is dropped with the
        // method, so no dangling `R` and no emitter-as-payload.
        let out = transform(
            r#"
use other_crate::FeedEvent;

#[taurpc::procedures(path = "feed", event_trigger = FeedEventTrigger)]
pub trait FeedMethods {
    #[taurpc(event)]
    async fn feed_event(app_handle: AppHandle<R>, event: FeedEvent);
    async fn active() -> bool;
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("external payload enum"),
            "expected the external-enum flag:\n{out}"
        );
        assert!(
            !out.contains("enum FeedEvent"),
            "no wrapper enum should be generated for an external payload:\n{out}"
        );
        assert!(
            !out.contains("AppHandle<R>") && !out.contains("event: FeedEvent"),
            "no dangling R and no emitter-as-payload:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn rewrites_payload_enum_emit_site() {
        // With the payload-enum pattern, an inline `Trigger::new(h).event(v)`
        // becomes `v.emit(&h)` (the payload value is the event), and a `.send_to`
        // becomes `v.emit_to(&h, target)`.
        let out = transform(
            r#"
pub enum AppEvent {
    Tick,
}

#[taurpc::procedures(path = "app", event_trigger = AppEventTrigger)]
pub trait AppMethods {
    #[taurpc(event)]
    async fn event(event: AppEvent);
}

pub fn fire(h: AppHandle, e: AppEvent) {
    AppEventTrigger::new(h.clone()).event(e.clone()).unwrap();
    AppEventTrigger::new(h).send_to(EventTarget::Any).event(e)?;
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("e.clone().emit(&h.clone())"),
            "broadcast payload emit:\n{out}"
        );
        assert!(
            out.contains("e.emit_to(&h, EventTarget::Any)"),
            "targeted payload emit:\n{out}"
        );
        assert!(
            !out.contains("AppEventTrigger::new"),
            "trigger construction consumed:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn drops_method_alias() {
        // ttipc has no per-method alias, so `#[taurpc(alias = "..")]` is
        // dropped (the method keeps its Rust name) and the header flags it.
        let out = transform(
            r#"
#[taurpc::procedures(path = "app")]
pub trait App {
    #[taurpc(alias = "method_with_alias")]
    async fn with_alias() -> u8;
}
"#,
        )
        .expect("valid Rust");
        assert!(
            !out.contains("method_with_alias"),
            "the alias attr should be dropped:\n{out}"
        );
        assert!(out.contains("- alias:"), "expected the alias flag:\n{out}");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn rewrites_router_mount() {
        // The TauRPC Router chain collapses: `Router::new()` and `export_config`
        // drop, each `.merge(X.into_handler())` becomes `X.into_procedures()`, and
        // `router.into_handler()` becomes `ttipc::handler(router)`. `.manage(..)`
        // stays. This is a mount-only file (the impls live elsewhere), so the only
        // header note is the mount one.
        let out = transform(
            r#"
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
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("AppImpl.into_procedures().merge(LogImpl.into_procedures())"),
            "expected the merge chain to collapse:\n{out}"
        );
        assert!(
            out.contains("ttipc::handler(router)") && !out.contains("bindings()"),
            "expected handler + dropped export_config:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn rewrites_router_factory_with_builder_methods() {
        // A `build() -> Router<R>` factory with extra TauRPC builder methods
        // (`semantic_types`, `dangerously_cast_bigints_to_number`): the builder
        // methods drop, the merge chain collapses, and the factory returns
        // `ttipc::Procedures` with its `<R>` gone -- so a caller's
        // `build().into_handler()` becomes `ttipc::handler(build())`.
        let out = transform(
            r#"
pub fn build<R: Runtime>() -> Router<R> {
    let typescript = config();
    taurpc::Router::new()
        .export_config(typescript)
        .semantic_types(semantic())
        .dangerously_cast_bigints_to_number()
        .merge(AppImpl.into_handler())
        .merge(LogImpl.into_handler())
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("AppImpl.into_procedures().merge(LogImpl.into_procedures())"),
            "the merge chain collapses:\n{out}"
        );
        assert!(
            !out.contains("Router::new")
                && !out.contains("semantic_types")
                && !out.contains("dangerously_cast_bigints_to_number"),
            "Router::new and the builder methods are dropped:\n{out}"
        );
        assert!(
            out.contains("fn build() -> ttipc::Procedures"),
            "the factory returns ttipc::Procedures, <R> dropped:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn rewrites_create_ipc_handler() {
        // The router-less mount: `create_ipc_handler(X.into_handler())` becomes
        // `ttipc::handler(X.into_procedures())`.
        let out = transform(
            r#"
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(taurpc::create_ipc_handler(AppImpl.into_handler()))
        .run(tauri::generate_context!())
        .expect("error while running");
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("ttipc::handler(AppImpl.into_procedures())"),
            "expected create_ipc_handler -> ttipc::handler:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn rewrites_cross_file_factory_consumer() {
        // A split mount: a `build() -> Router` factory in one file, the
        // `build().into_handler()` call in another (with no local `Router::new`).
        // The factory collapses + returns `ttipc::Procedures`; the consumer
        // file resolves `build().into_handler()` to `ttipc::handler(build())`
        // through the project-wide factory registry, while an unrelated
        // `.into_handler()` in the same file is left untouched.
        let out = transform_project(&[
            (
                "router.rs".into(),
                r#"
pub fn build<R: Runtime>() -> Router<R> {
    taurpc::Router::new().merge(AppImpl.into_handler())
}
"#
                .into(),
            ),
            (
                "lib.rs".into(),
                r#"
pub fn run() {
    let handler = crate::router::build().into_handler();
    let unrelated = some_lib::Thing::new().into_handler();
    tauri::Builder::default()
        .manage(unrelated)
        .invoke_handler(handler)
        .run(tauri::generate_context!())
        .unwrap();
}
"#
                .into(),
            ),
        ])
        .expect("valid Rust");
        let lib = &out[1].1;
        assert!(
            lib.contains("ttipc::handler(crate::router::build())"),
            "the cross-file consumer resolves:\n{lib}"
        );
        assert!(
            lib.contains("some_lib::Thing::new().into_handler()"),
            "an unrelated into_handler is left untouched:\n{lib}"
        );
        assert!(
            lib.contains("- mount:"),
            "the consumer file gets the mount note:\n{lib}"
        );
        insta::assert_snapshot!(lib);
    }

    #[test]
    fn leaves_non_taurpc_into_handler_alone() {
        // No `Router::new()`/`create_ipc_handler`, so the file is not a TauRPC
        // mount and an unrelated `.into_handler()` is left untouched.
        let out = transform(
            r#"
pub fn run() {
    let h = other_lib::Thing::new().into_handler();
    serve(h);
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("into_handler") && !out.contains("ttipc::handler"),
            "an unrelated into_handler must be left alone:\n{out}"
        );
        assert!(
            !out.contains("Manual follow-ups"),
            "no header for a non-taurpc file:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn rewrites_emit_sites() {
        // `Trigger::new(h).method(args).tail` -> `Enum::Variant { fields }.emit(&h)
        // .tail`, the trigger resolved through the registry: the default
        // `TauRpc{Trait}EventTrigger` and a custom `event_trigger =` both map, unit
        // / single / multi-field arities all handled.
        let out = transform(
            r#"
#[taurpc::procedures(path = "app")]
pub trait App {
    #[taurpc(event)]
    async fn ready();
}

#[taurpc::procedures(path = "log", event_trigger = LogBus)]
pub trait Log {
    #[taurpc(event)]
    async fn line(text: String);
    #[taurpc(event)]
    async fn at(x: i32, y: i32);
}

pub fn fire(h: AppHandle) {
    TauRpcAppEventTrigger::new(h.clone()).ready().unwrap();
    LogBus::new(h.clone()).line(msg()).unwrap();
    LogBus::new(h).at(1, 2)?;
}
"#,
        )
        .expect("valid Rust");
        assert!(out.contains("AppEvent::Ready"), "default trigger:\n{out}");
        assert!(
            out.contains("LogEvent::Line") && out.contains("text: msg()"),
            "custom trigger + field:\n{out}"
        );
        assert!(
            out.contains("LogEvent::At") && out.contains("x: 1") && out.contains("y: 2"),
            "multi-field emit:\n{out}"
        );
        assert!(out.contains(".emit(&h)"), "handle passed to emit:\n{out}");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn rewrites_send_to_emit_sites() {
        // taurpc's `Trigger::new(h).send_to(target).method(args)` (a per-target
        // send) maps to ttipc's `Enum::Variant { .. }.emit_to(&h, target)`:
        // the target expression carries over (both take `Into<EventTarget>`).
        // Unit and field arities both handled; a broadcast site on the same
        // trigger still becomes `.emit(&h)`.
        let out = transform(
            r#"
#[taurpc::procedures(path = "log", event_trigger = LogBus)]
pub trait Log {
    #[taurpc(event)]
    async fn ready();
    #[taurpc(event)]
    async fn at(x: i32, y: i32);
}

pub fn fire(h: AppHandle) {
    LogBus::new(h).send_to(EventTarget::Any).ready().unwrap();
    LogBus::new(h).send_to("main").at(1, 2)?;
    LogBus::new(h).ready().unwrap();
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("LogEvent::Ready.emit_to(&h, EventTarget::Any)"),
            "unit variant, targeted:\n{out}"
        );
        assert!(
            out.contains("LogEvent::At")
                && out.contains("emit_to(&h, \"main\")")
                && out.contains("x: 1")
                && out.contains("y: 2"),
            "field variant, targeted:\n{out}"
        );
        assert!(
            out.contains("LogEvent::Ready.emit(&h)"),
            "broadcast site on the same trigger stays .emit:\n{out}"
        );
        assert!(
            !out.contains("LogBus::new") && !out.contains(".send_to("),
            "trigger construction and send_to consumed by the rewrite:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn flags_bound_trigger_emit() {
        // A trigger bound to a variable can't be rewritten by the direct pass, so
        // it is left as-is and the header flags it.
        let out = transform(
            r#"
#[taurpc::procedures(path = "app", event_trigger = AppBus)]
pub trait App {
    #[taurpc(event)]
    async fn ready();
}

pub fn fire(h: AppHandle) {
    let bus = AppBus::new(h);
    bus.ready().unwrap();
}
"#,
        )
        .expect("valid Rust");
        assert!(
            out.contains("AppBus::new(h)") && out.contains("bus.ready()"),
            "the bound trigger should be left as-is:\n{out}"
        );
        assert!(out.contains("- emit sites:"), "expected the flag:\n{out}");
        insta::assert_snapshot!(out);
    }
}
