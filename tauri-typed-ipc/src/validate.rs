//! Opt-in runtime payload validation (`validate` feature, default off).
//!
//! The typed client and the drift `check` keep the two sides in agreement at
//! build time, but TypeScript types vanish at runtime: a malformed or
//! version-skewed payload -- a hand-written `invoke`, a stale client, bad data
//! from the webview -- is otherwise only caught where serde deserialization
//! trips, deep in a dispatch, with a serde message. This feature adds a schema
//! contract at the boundary.
//!
//! The contract is a single JSON Schema derived from the *same* specta graph
//! the bindings are rendered from, so both ends check one document: the client
//! validates its arguments before `invoke`, and [`handler_validated`] validates
//! the incoming payload before dispatch. JSON Schema (not a TypeScript-only
//! schema) is deliberate -- it is the only contract the Rust side can validate
//! against too.
//!
//! Off by default: without the feature the crate pulls no validator and a
//! [`handler`](crate::handler) does no per-payload schema work.
//!
//! ```no_run
//! # use tauri_typed_ipc::procedures;
//! # #[procedures]
//! # trait Greeter { fn greet(&self, name: String) -> String; }
//! # struct Backend;
//! # impl Greeter for Backend { fn greet(&self, name: String) -> String { name } }
//! # fn mount<R: tauri::Runtime>(builder: tauri::Builder<R>) -> Result<tauri::Builder<R>, Box<dyn std::error::Error>> {
//! let validator = tauri_typed_ipc::Validator::new().register::<GreeterProcedures>()?;
//! Ok(builder.invoke_handler(tauri_typed_ipc::handler_validated(
//!     Backend.into_procedures(),
//!     validator,
//! )))
//! # }
//! ```

use std::collections::HashMap;

use serde_json::{Value, json};
use specta::Types;
use specta::datatype::{DataType, Field, NamedDataType, NamedReferenceType, Reference, Struct};

use crate::ProcedureSet;

/// Compiled per-command argument validators, built from one or more procedure
/// sets. Reuses the binding descriptor a `#[procedures]` trait already
/// generates, so validation needs no separate schema definition and cannot
/// drift from the client's types.
///
/// Build one with [`new`](Self::new) and [`register`](Self::register), then
/// pass it to [`handler_validated`](crate::handler_validated).
#[must_use]
pub struct Validator {
    /// Keyed by wire command name (namespaced when the set is), so a lookup
    /// matches exactly what the handler routes on.
    commands: HashMap<String, jsonschema::Validator>,
}

impl Validator {
    /// An empty validator that passes every command. Add sets with
    /// [`register`](Self::register).
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }

    /// Compile validators for every command in a procedure set, from its
    /// generated `{Trait}Procedures` descriptor.
    ///
    /// Errors if the JSON Schema cannot be built or compiled -- structural,
    /// so it surfaces at startup, when the validator is assembled, not
    /// per call.
    pub fn register<P: ProcedureSet>(mut self) -> Result<Self, ValidatorError> {
        // Populate the type registry and collect each command's wire shape --
        // the same call the bindings generator makes.
        let mut types = Types::default();
        let procedures = P::procedures(&mut types);

        // Synthesize one named args struct per command into the shared
        // registry, so a single export renders every command's object schema
        // plus the named types they reference (resolved through `$ref`).
        let mut wire_names = Vec::with_capacity(procedures.len());
        for (index, procedure) in procedures.iter().enumerate() {
            // A unique, collision-proof definition name; the wire name is the
            // map key. Only the arguments are modelled -- a `Channel<T>`'s id
            // and forward-compatible extras ride through as permitted
            // additional properties.
            let def_name = format!("TtipcArgs{index}");
            let fields: Vec<(String, DataType)> = procedure
                .args
                .iter()
                .map(|(name, ty)| ((*name).to_string(), inline_leaf(ty.clone())))
                .collect();
            NamedDataType::new(def_name.clone(), &mut types, |_types, ndt| {
                let mut builder = Struct::named();
                for (name, ty) in &fields {
                    builder = builder.field(name.clone(), Field::new(ty.clone()));
                }
                ndt.ty = Some(builder.build());
            });

            let wire = match P::NAMESPACE {
                Some(namespace) => format!("{namespace}.{}", procedure.name),
                None => procedure.name.to_string(),
            };
            wire_names.push((wire, def_name));
        }

        // One export of the whole registry, then one compiled validator per
        // command rooted at its args definition (the shared `definitions`
        // supply every `$ref`).
        let document = specta_jsonschema::JsonSchema::default()
            .export(&types, specta_serde::Format)
            .map_err(|source| ValidatorError::Schema(source.to_string()))?;
        let document: Value = serde_json::from_str(&document)
            .map_err(|source| ValidatorError::Schema(source.to_string()))?;
        let definitions = document
            .get("definitions")
            .cloned()
            .unwrap_or_else(|| json!({}));

        for (wire, def_name) in wire_names {
            let schema = json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "$ref": format!("#/definitions/{def_name}"),
                "definitions": definitions,
            });
            let compiled = jsonschema::validator_for(&schema)
                .map_err(|source| ValidatorError::Compile(source.to_string()))?;
            self.commands.insert(wire, compiled);
        }

        Ok(self)
    }

    /// Validate a call's JSON arguments against the command's contract.
    /// `Ok(())` for a command this validator does not cover -- routing, not
    /// validation, decides whether an unknown command is an error.
    pub fn validate(&self, command: &str, args: &Value) -> Result<(), ValidationError> {
        let Some(compiled) = self.commands.get(command) else {
            return Ok(());
        };
        if compiled.is_valid(args) {
            return Ok(());
        }
        // First failure is enough for a boundary rejection; the full set is
        // available to callers who want it via the compiled schema.
        let message = compiled.iter_errors(args).next().map_or_else(
            || "does not match the schema".to_string(),
            |err| err.to_string(),
        );
        Err(ValidationError {
            command: command.to_string(),
            message,
        })
    }
}

impl Default for Validator {
    fn default() -> Self {
        Self::new()
    }
}

/// Unwrap inline named references to the datatype they render as, recursively,
/// so a bare primitive argument (`name: String`) and container elements
/// (`tags: Vec<String>`) carry a real schema rather than an empty `{}`.
///
/// specta returns a primitive as a named reference whose `inner` inlines the
/// primitive; JSON Schema renders such a reference as an empty definition,
/// which matches anything. A genuine named type (`Progress`) has a
/// `Reference` inner and is kept as a `$ref` for the exporter to render.
fn inline_leaf(ty: DataType) -> DataType {
    match ty {
        DataType::Reference(Reference::Named(named)) => {
            if let NamedReferenceType::Inline { dt, .. } = &named.inner {
                inline_leaf((**dt).clone())
            } else {
                DataType::Reference(Reference::Named(named))
            }
        }
        DataType::List(mut list) => {
            list.ty = Box::new(inline_leaf(*list.ty));
            DataType::List(list)
        }
        DataType::Map(mut map) => {
            let key = inline_leaf(map.key_ty().clone());
            let value = inline_leaf(map.value_ty().clone());
            *map.key_ty_mut() = key;
            *map.value_ty_mut() = value;
            DataType::Map(map)
        }
        DataType::Tuple(mut tuple) => {
            tuple.elements = tuple.elements.into_iter().map(inline_leaf).collect();
            DataType::Tuple(tuple)
        }
        DataType::Nullable(inner) => DataType::Nullable(Box::new(inline_leaf(*inner))),
        DataType::Intersection(parts) => {
            DataType::Intersection(parts.into_iter().map(inline_leaf).collect())
        }
        // Primitive, Struct, Enum, Generic, and opaque references render as-is.
        // Unwrapping never over-constrains, so a variant left alone at most
        // under-validates -- it never rejects a valid payload.
        other => other,
    }
}

/// Failure assembling a [`Validator`] from a procedure set: the contract's
/// JSON Schema could not be built or compiled. Structural, so it is raised
/// once at [`register`](Validator::register), never per call.
#[derive(Debug, thiserror::Error)]
pub enum ValidatorError {
    /// The JSON Schema could not be rendered from the specta graph.
    #[error("building the JSON Schema contract failed: {0}")]
    Schema(String),
    /// The rendered schema could not be compiled into a validator.
    #[error("compiling the validator failed: {0}")]
    Compile(String),
}

/// A payload that failed its command's contract. Carried into
/// [`DispatchError::Invalid`](crate::DispatchError::Invalid) by a validating
/// handler and rejected at the boundary, before dispatch.
#[derive(Debug, thiserror::Error)]
#[error("invalid payload for {command:?}: {message}")]
pub struct ValidationError {
    /// The wire command whose arguments failed.
    pub command: String,
    /// The first schema violation, as a human-readable message.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProcedureSet, ProcedureType};
    use specta::Type;

    // A hand-built procedure set, so the Validator is exercised without the
    // `#[procedures]` macro (which cannot expand inside the defining crate).
    // Two commands: a bare-primitive arg and a Vec-of-primitive arg -- the two
    // cases the inline-leaf transform exists to cover.
    struct Fixture;
    impl ProcedureSet for Fixture {
        const OBJECT: &'static str = "fixture";
        fn procedures(types: &mut Types) -> Vec<ProcedureType> {
            vec![
                ProcedureType {
                    name: "greet",
                    args: vec![("name", <String as Type>::definition(types))],
                    channels: vec![],
                    output: <String as Type>::definition(types),
                    error: None,
                },
                ProcedureType {
                    name: "sum",
                    args: vec![("values", <Vec<u32> as Type>::definition(types))],
                    channels: vec![],
                    output: <u32 as Type>::definition(types),
                    error: None,
                },
            ]
        }
    }

    fn validator() -> Validator {
        Validator::new()
            .register::<Fixture>()
            .expect("fixture validator builds")
    }

    #[test]
    fn accepts_well_typed_payloads() {
        let v = validator();
        assert!(v.validate("greet", &json!({ "name": "world" })).is_ok());
        assert!(v.validate("sum", &json!({ "values": [1, 2, 3] })).is_ok());
    }

    #[test]
    fn rejects_a_wrong_typed_primitive_argument() {
        // The load-bearing case: a bare String arg is really validated, not
        // waved through as an empty schema.
        let v = validator();
        let err = v
            .validate("greet", &json!({ "name": 5 }))
            .expect_err("a number is not a string");
        assert_eq!(err.command, "greet");
    }

    #[test]
    fn rejects_a_wrong_typed_container_element() {
        // The recursive transform: Vec<u32> validates its elements, not just
        // that the value is an array.
        let v = validator();
        assert!(v.validate("sum", &json!({ "values": ["x"] })).is_err());
    }

    #[test]
    fn rejects_a_missing_required_argument() {
        let v = validator();
        assert!(v.validate("greet", &json!({})).is_err());
    }

    #[test]
    fn passes_unknown_commands_through() {
        // Not this validator's concern -- routing decides.
        let v = validator();
        assert!(v.validate("not_registered", &json!({})).is_ok());
    }
}
