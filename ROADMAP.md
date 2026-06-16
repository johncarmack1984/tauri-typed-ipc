# Roadmap

Two tracks, in dependency order: help specta reach a stable 2.0.0, then build
tauri-typed-ipc on top of it.

## Context

- specta lives at [specta-rs/specta](https://github.com/specta-rs/specta),
  currently `2.0.0-rc.25`. Working fork: `johncarmack1984/specta`.
- The [Specta v2 RFCs](https://specta.dev/docs/specta/rfc)
  document six phases of v2 rework (Feb 2025 - Apr 2026), now mostly landed:
  the reference-system redesign (`ArcId`, named vs opaque references),
  format-agnostic attribute handling, TypeScript strictness, multi-file
  layouts, branded types, phase-specific type splitting, and BigInt handling.
- [Oscar's original v2 plan](https://hackmd.io/@oscartbeaumont/spectav2) frames
  the goals: Rust-centric core, exporters split into their own crates, and one
  structural question called out as critical -- how third-party `Type` impls
  survive the orphan rule without forcing major version bumps.

## Track 1: specta 2.0.0 stable

Goal: help close the gap between rc.25 and a release Oscar is happy to put
semver guarantees behind.

### Step 0: alignment

- Ask Oscar what he considers release-blocking, and propose collecting it into
  a `v2.0.0` milestone so the finish line is visible.
- Build trust with small, scoped fixes first; open an issue before anything
  design-shaped.

### Known gaps (from the RFC's "remaining" list)

- Const generics: conservative rules landed; float const generics unresolved
  (whether const `f64` can be hashed safely).
- `DataType::Literal` with floats commented out pending the `Hash` decision.
- `MaybeUndefined`: userspace pattern preferred; needs docs or a blessed impl.
- Nuanced field transforms (`Date`, `Uint8Array`) still buggy; the BigInt path
  is validated.
- Third-party impls vs the orphan rule: confirm where this landed -- solved,
  release-blocking, or explicitly deferred to v3 (issue #467 suggests a v3
  bucket exists).

### Open-issue candidates (as of 2026-06-12)

- #494: serde attribute `bound` parsing bug (fresh and scoped -- good first PR)
- #491: specta-jsonschema drops type info that specta-typescript preserves
- #481: reconsider `BigIntExportBehavior::Number` (design discussion)
- #228: configurable `Option` behavior
- #94: `specta::json` macro (help wanted)
- Exporter tracking issues (kotlin, openapi, rust, wit, rescript, valibot) are
  tier 2/3 per the v2 plan -- explicitly not release blockers.

### Definition of done

`specta` and `specta-typescript` stable on crates.io, with `tauri-specta` and
TauRPC released against them.

## Track 2: tauri-typed-ipc

Goal: a trait-based, type-safe Tauri IPC layer in the spirit of
[TauRPC](https://github.com/MatsDK/TauRPC), built from the ground up on stable
specta v2 -- with sync commands as the default.

### Why sync-first

Tauri runs async commands on the async runtime: that forces `Send + 'static`
bounds everywhere, rules out `!Send` state (`Rc`, `RefCell`), and pushes
main-thread-only platform APIs (windows and webviews, especially on macOS)
behind workarounds. Non-async commands run on the main thread with none of
those constraints. Most desktop IPC handlers are short and CPU-light; they do
not need an executor. tauri-typed-ipc makes the simple case simple -- a procedure is
a plain `fn` -- and the tradeoff stays honest: a slow sync handler blocks the
UI, so long-running work opts into async explicitly instead of every handler
paying the async tax.

### Design pillars

1. Sync by default; async opt-in per procedure.
2. specta v2 + specta-typescript are the only type machinery. No vendored type
   system. Any gap found here becomes a Track 1 issue.
3. Trait-defined procedure sets: one trait, one resolver impl, one generated
   TS client. Typed events in both directions. Channels for streaming.
4. Phase-aware bindings: specta v2's serialize/deserialize type splitting
   applied automatically (args use the deserialize phase, returns serialize).
5. BigInt-style integers (`i64`, `u64`, ...) are forbidden by default rather
   than silently truncated past 2^53; lossless transport awaits an ecosystem
   solution.
6. Deterministic codegen: stable output ordering, clean diffs, watch-mode
   friendly, and a check mode so bindings drift fails CI.
7. Minimal bootstrapping: zero to a first typed command in minutes -- one
   macro, one handler registration, one generated client, no multi-file
   ritual before "hello world".
8. The macro output is itself under test: expansion snapshots, compile-fail
   cases (trybuild), and snapshot tests of the generated TypeScript.

### Phases

- **R0 Design.** Requirements writeup and API sketch. Document Tauri's exact
  sync/async command threading model with citations into tauri source. Survey
  tauri-specta on v2 and TauRPC's specta-v2 upgrade (MatsDK/TauRPC#64), and
  audit [lux](https://github.com/johncarmack1984/lux) -- a live taurpc
  consumer -- for concrete bootstrap and testing pain. Positioning:
  complement tauri-specta, not compete (Oscar got a heads-up 2026-06-12).
- **R1 Walking skeleton.** One sync procedure end to end: macro, handler
  registration, generated TS, invoke. Built against rc.25; churn expected.
- **R2 Surface area.** Events, state access, error model.
- **R3 Async opt-in.** Plus channels/streaming.
- **R4 Feature parity.** Nested command/event routers, targeted (per-window)
  events, phase-aware bindings (pillar 4), BigInt transport (pillar 5), and
  drift-proof codegen with a check/watch mode (pillar 6).
- **R5 Ship.** Examples and docs ("hello-tauri-typed-ipc"), port lux to tauri-typed-ipc as
  the dogfood validation, publish 0.1.0 on specta rc.25.

### Open questions

- **Fire-and-forget bindings.** Tauri IPC is async at the transport, so any
  command that returns a value or can fail stays `Promise<T>` -- "sync by
  default" describes the Rust handler, not the JS call. But an infallible,
  no-return command (Rust `-> ()`) could instead generate a fire-and-forget
  binding: a `void`-returning call that does not await, routing transport
  errors to the console -- the pattern logging plugins use, and a natural fit
  for high-frequency calls like a fader `set` during a drag. Options: (A) keep
  `Promise<T>` uniformly; (B) auto-derive fire-and-forget from the `-> ()`
  signature, which also lets `no-floating-promises` flag exactly the
  fallible/returning calls and stay quiet on the rest; (C) opt in per command
  via an attribute, defaulting to `Promise<T>`. C is the most flexible and
  keeps await-ability by default; B may be the more ergonomic default. Decide
  in R2 with the error model. Whichever wins, a fire-and-forget binding must
  return `void`, not `Promise<void>`, or it trips `no-floating-promises` and
  loses the ergonomic.

### Non-goals for 0.1

- Middleware/plugin system
- Non-Tauri transports
- Exporters beyond TypeScript
- Framework-specific client adapters (React hooks, Svelte stores) -- shown in
  examples, not generated

### Beyond 0.1

- **Runtime-validated bindings (Zod).** TypeScript types vanish at runtime, so a
  malformed or version-skewed IPC payload is only caught where it gets misused,
  not at the boundary. Generating Zod schemas alongside the types via specta-zod
  would let the client validate payloads as they cross the wire -- catching
  Rust/TS drift and bad data at the edge rather than deep in the UI. A second
  exporter is a 0.1 non-goal, so this stays opt-in and purely additive to the
  TypeScript output; specta-zod already covers tauri-typed-ipc's wire types today but is
  pre-1.0, so shipping it is gated on specta-zod maturing.
