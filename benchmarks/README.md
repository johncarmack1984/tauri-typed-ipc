# benchmarks

Cross-framework comparison: the same `greet` procedure through raw
`#[tauri::command]`, tauri-typed-ipc, and [TauRPC](https://github.com/MatsDK/TauRPC).

## Layout

Three detached crates, each with its own lockfile:

- `common/` -- mock-IPC plumbing shared by both arms, so the harness is
  byte-identical by construction.
- `tauri-typed-ipc/` -- raw control arm + tauri-typed-ipc arm.
- `taurpc/` -- raw control arm + taurpc arm.

The split is forced: taurpc 0.5.2 pins `specta = "=2.0.0-rc.22"` and
tauri-typed-ipc pins `=2.0.0-rc.25`, which can never resolve in one dependency
graph. Both twins pin identical tauri feature sets so the differences
between them are the IPC layers, not tauri configuration.

## Runtime cost

```sh
(cd tauri-typed-ipc && cargo bench)
(cd taurpc && cargo bench)
```

Each binary measures the full invoke pipeline through tauri's mock
runtime (routing, payload deserialize, call, response serialize,
responder). The mock runtime skips the webview and the process hop, so
the absolute numbers are the Rust-side cost only.

Read the results as each layer's **delta over the `raw_command` control
in its own binary** -- that cancels machine and run variance across the
split. The taurpc delta includes its executor round-trip (its resolvers
are async-only); that is the design difference under measurement, not
an artifact.

## Compile cost

```sh
./compile-time.sh
```

Cold (full graph from `cargo clean`) and hot (rebuild after touching
the file with the procedure trait) per twin, wall-clock, single-shot.
Run it twice if the first run had to fetch crates. The twin delta is
what adopting the layer costs in build time.

```sh
./scaling.sh
```

How tauri-typed-ipc's macro cost grows with the number of procedures in one
trait. It generates a fixture, warms the dependency tree once, then
times a hot rebuild (fixture crate only) at N = 1, 8, 64, 256 -- so the
curve isolates macro expansion and codegen from the constant tree. One
cold build at the largest N is the from-clean reference. The fixture
crate lands in the gitignored `scaling-tmp/`.
