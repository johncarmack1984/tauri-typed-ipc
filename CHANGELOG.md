# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.2.0...v0.2.1) - 2026-11-04

### Other

- *(deps)* build on tauri 2.12.5, allowing its deprecated state accessor ([#73](https://github.com/johncarmack1984/tauri-typed-ipc/pull/73))

## [0.2.0](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.10...v0.2.0) - 2026-11-03

### Added

- `Bindings::map_datatypes`: one transform over every exported datatype (the collection's named types, each procedure's arguments, channels and return type, and each event payload). It's the hook for wire-level rewrites like specta-util's `Remapper::dangerous_bigints_as_number()`. ([#70](https://github.com/johncarmack1984/tauri-typed-ipc/pull/70))

### Changed

- Built on specta `2.0.0-rc.27` (specta-typescript and specta-serde 0.0.14, specta-jsonschema 0.0.5), still pinned exact. A consumer needs specta rc.27 too, which is what makes this release breaking. ([#70](https://github.com/johncarmack1984/tauri-typed-ipc/pull/70))
- A field whose serialize and deserialize shapes differ, like an `Option` with `skip_serializing_if`, renders as optional and nullable (`note?: string | null`) instead of failing the export. ([#70](https://github.com/johncarmack1984/tauri-typed-ipc/pull/70))
- With `validate`, the JSON Schema in the generated client is draft 2020-12 (`$defs`, integer bounds instead of `format`), so a validating client regenerates with different schema text. ([#70](https://github.com/johncarmack1984/tauri-typed-ipc/pull/70))

## [0.1.10](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.9...v0.1.10) - 2026-09-02

### Other

- set workspace homepage to johncarmack.com ([#58](https://github.com/johncarmack1984/tauri-typed-ipc/pull/58))

## [0.1.8](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.7...v0.1.8) - 2026-07-22

### Other

- emit the JSON Schema contract + a pre-invoke check into the client ([#49](https://github.com/johncarmack1984/tauri-typed-ipc/pull/49))

## [0.1.7](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.6...v0.1.7) - 2026-07-22

### Other

- opt-in runtime payload validation at the IPC boundary ([#47](https://github.com/johncarmack1984/tauri-typed-ipc/pull/47))
- weekly + PR job building against the latest tauri 2.x ([#46](https://github.com/johncarmack1984/tauri-typed-ipc/pull/46))

## [0.1.6](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.5...v0.1.6) - 2026-07-18

### Other

- async procedures take AppHandle and State ([#42](https://github.com/johncarmack1984/tauri-typed-ipc/pull/42))

## [0.1.4](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.3...v0.1.4) - 2026-07-03

### Other

- update the TauRPC comparison for upstream sync methods ([#31](https://github.com/johncarmack1984/tauri-typed-ipc/pull/31))
- Contract README phrasing; fix stale ROADMAP status; add seo-kit baseline ([#29](https://github.com/johncarmack1984/tauri-typed-ipc/pull/29))

## [0.1.3](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.2...v0.1.3) - 2026-06-30

### Other

- update status to 0.1.2 ([#24](https://github.com/johncarmack1984/tauri-typed-ipc/pull/24))

## [0.1.2](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.1...v0.1.2) - 2026-06-18

### Other

- test the mount and export examples instead of ignoring them ([#20](https://github.com/johncarmack1984/tauri-typed-ipc/pull/20))

## [0.1.1](https://github.com/johncarmack1984/tauri-typed-ipc/compare/v0.1.0...v0.1.1) - 2026-06-18

### Other

- make Result<_, String> a built-in drop-in ([#15](https://github.com/johncarmack1984/tauri-typed-ipc/pull/15))

## [0.1.0] - 2026-06-16

Initial release. Built on specta `2.0.0-rc.25` (pinned exactly; see the README).

### Added

- `#[procedures]`: define an IPC surface as a Rust trait, flattened into a dispatch
  core plus a descriptor. `handler` mounts it on tauri's invoke pipeline, wire-identical
  to a raw `#[tauri::command]` (same command name, same named-argument JSON, no envelope),
  so generated calls, hand-written `invoke`s, and raw commands interoperate.
- Sync commands by default, dispatched inline on the main thread; a single `async fn`
  opts that one procedure onto tauri's runtime.
- Typed events in both directions via `#[derive(Event)]` (`emit` and targeted `emit_to`).
- Typed errors: a `Result<_, E>` procedure rejects `E` on the wire via `#[derive(Error)]`,
  and the generated client types its `catch` against it.
- Type-matched injection of `AppHandle` and `tauri::State<T>` (by type, never by name).
- Streaming via `Channel<T>` parameters.
- TypeScript client generation behind the `export` feature, with a `check` mode that
  fails CI when the committed client drifts from the Rust definition.
- `ttipc-migrate`: a source codemod from TauRPC to tauri-typed-ipc.

[0.1.0]: https://github.com/johncarmack1984/tauri-typed-ipc/releases/tag/v0.1.0
