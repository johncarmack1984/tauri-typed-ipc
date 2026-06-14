# faders

The tauri-typed-ipc example app: a DMX desk pushing fader moves around,
borrowed from [lux](https://github.com/johncarmack1984/lux). One Tauri
app, both halves of the contract -- the `#[procedures]` trait in
`src-tauri/src/main.rs` and the generated client consumed in
`src/main.ts`.

Design target, not working code (design-by-example): the example comes
first, and the library gets built until this app compiles and runs. It
is a separate crate outside the root build graph, so `cargo build` and
`cargo test` at the repo root stay green. Expect it to start compiling
as the R1 walking skeleton lands.

Once real:

    npm install
    npm run tauri dev

Open design questions are marked `OPEN (R0)` in `src-tauri/src/main.rs`.
Calling convention: the TS side always awaits -- the IPC wire is async
even when the Rust side runs sync on the main thread.

The swatch row is a virtual rig standing in for DMX hardware: each
swatch renders a channel level as brightness, updated only from the
typed event stream (Rust's canonical state read back), never
optimistically from input. If swatches and sliders stay in lockstep
under fast scrubbing, the high-frequency roundtrip works; divergence is
a bug made visible.
