#!/bin/sh
# Compile-time comparison between the benchmark twins. Run from
# benchmarks/. Wall-clock, single-shot, machine-local -- directional
# numbers, not CI grade. Run it twice if the first run had to fetch
# crates; only the second is a pure compile measurement.
#
# cold: the full dependency graph from cargo clean -- the cost of
#       adopting the layer. The twins pin identical tauri feature sets,
#       so the delta between them is the IPC layer's tree.
# hot:  rebuild after touching the fixture lib (where the procedure
#       trait lives) -- the edit-a-procedure developer loop.
set -eu
cd "$(dirname "$0")"

for twin in tauri-typed-ipc taurpc; do
    echo "== $twin: cold (cargo clean + cargo build --release)"
    (cd "$twin" && cargo clean -q && /usr/bin/time -p cargo build --release -q)
    echo "== $twin: hot (touch src/lib.rs + cargo build --release)"
    (cd "$twin" && touch src/lib.rs && /usr/bin/time -p cargo build --release -q)
done
