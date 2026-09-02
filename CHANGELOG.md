# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
