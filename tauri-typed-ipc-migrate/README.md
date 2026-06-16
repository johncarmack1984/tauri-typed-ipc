# tauri-typed-ipc-migrate

`ttipc-migrate` is a source-to-source codemod that rewrites a
[TauRPC](https://github.com/MatsDK/TauRPC) codebase to
[`tauri-typed-ipc`](https://crates.io/crates/tauri-typed-ipc).

It transforms the `#[taurpc::procedures]` trait, the resolver impl,
`#[taurpc::ipc_type]` DTOs, event triggers, and the router mount into their
tauri-typed-ipc equivalents, and leaves a header comment listing the manual
follow-ups it cannot do automatically.

## Install

```sh
cargo install tauri-typed-ipc-migrate
```

## Usage

Preview one file (writes the migrated source to stdout; the original is untouched):

```sh
ttipc-migrate src/ipc.rs
```

Migrate a whole project in place. This edits files surgically (only the changed
spans; comments preserved) and lands the result as a single commit on a fresh
`ttipc-migration` branch, so the diff is easy to review and to drop:

```sh
ttipc-migrate --write src/**/*.rs
```

The working tree must be clean first. After it runs, review the diff and the
header comment's follow-up notes, then add the `ttipc = { package = "tauri-typed-ipc" }`
dependency.
