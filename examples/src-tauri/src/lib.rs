//! faders: the ttipc example app, Rust half.
//!
//! CURRENT STATE: the whole app is ttipc -- `set`, `level`, the async
//! `save_scene`, and the async streaming `fade` all run through one
//! `handler`, no raw commands and no fallback. `async fn` is the opt-in:
//! `set`/`level` run sync on the main thread, `save_scene`/`fade` earn
//! the runtime; `fade` streams its levels back over a `Channel<u8>`.

use std::sync::Mutex;

// Raw-Tauri pain receipt #1: state touched from an async procedure must
// be Send + Sync (save_scene runs on the async runtime), so the levels
// live behind a Mutex even though set/level only ever touch them on the
// main thread. ttipc's sync-first ideal is a RefCell here -- reached
// in the next slice via a main-thread hop that lets the async procedure
// touch !Send state soundly.
struct Desk {
    levels: Mutex<[u8; 512]>,
}

// Receipt #2 retired: ttipc's Error derive generates the wire
// Serialize -- each variant becomes { type, message } the client can
// branch on, no hand-written serialize_str.
#[derive(Debug, thiserror::Error, ttipc::Error)]
enum SaveError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

// Receipt #3 retired: the Event derive owns the wire name and payload on
// both ends. `Changed { .. }.emit(&app)` emits "fader:changed"; the
// generated bindings.ts listens on the same name, no stringly match by
// hand.
#[derive(ttipc::Event)]
enum FaderEvent {
    Changed { channel: u16, value: u8 },
}

// The procedures. Wire parity: the generated bindings.ts calls
// invoke("set", { channel, value }) and invoke("level", { channel })
// exactly as raw #[tauri::command]s did -- generation changed nothing
// on the wire. `app` is injected by TYPE (it is not a wire argument),
// never by parameter name. `save_scene` is the async opt-in: a plain fn
// runs sync on the main thread, an `async fn` earns the runtime. `fade`
// adds a streaming `Channel<u8>`: like the injected `app` it is not a
// wire argument (the client passes a `Channel`, its id rides the wire),
// but unlike `app` it stays in the binding -- the client supplies it.
#[ttipc::procedures]
trait Faders {
    fn set(&self, app: tauri::AppHandle, channel: u16, value: u8);
    fn level(&self, channel: u16) -> u8;
    async fn save_scene(&self, path: String) -> Result<(), SaveError>;
    async fn fade(&self, channel: u16, levels: ttipc::Channel<u8>);
}

impl Faders for Desk {
    fn set(&self, app: tauri::AppHandle, channel: u16, value: u8) {
        self.levels.lock().unwrap()[channel as usize] = value;
        FaderEvent::Changed { channel, value }
            .emit(&app)
            .expect("event emit failed");
    }

    fn level(&self, channel: u16) -> u8 {
        self.levels.lock().unwrap()[channel as usize]
    }

    async fn save_scene(&self, path: String) -> Result<(), SaveError> {
        // Copy out under the lock, release it, then do the IO -- the
        // guard never crosses the await (it is !Send anyway).
        let scene = self.levels.lock().unwrap().to_vec();
        tokio::fs::write(&path, scene).await?;
        Ok(())
    }

    async fn fade(&self, channel: u16, levels: ttipc::Channel<u8>) {
        // A request-scoped stream: ramp one channel 0 -> full over a
        // second, streaming each level to the one caller that asked. An
        // async procedure can't emit events yet (no AppHandle), so the
        // Channel IS its output -- the streaming-over-time case channels
        // are for. The lock is scoped per step, never held across the
        // await; the slider does not follow (no event), only the swatch
        // painted from the stream.
        const STEPS: u16 = 16;
        for step in 0..=STEPS {
            let value = (u32::from(step) * 255 / u32::from(STEPS)) as u8;
            self.levels.lock().unwrap()[channel as usize] = value;
            levels.send(value).expect("fade send failed");
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
    }
}

/// The ttipc client generator for [`Faders`] and [`FaderEvent`],
/// configured but not yet rendered. The committed `src/bindings.ts` is
/// what this renders; the binding test keeps them in lockstep with
/// `check` (regenerate with `REGEN_BINDINGS=1 cargo test`). The whole
/// client is generated, `save_scene` and the streaming `fade` included --
/// down to the `SaveError` union the former's rejection is typed against.
pub fn ttipc_bindings() -> ttipc::Bindings {
    ttipc::Bindings::new()
        .register::<FadersProcedures>()
        .register_events::<FaderEvent>()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // One handler, no fallback: every command -- sync and async -- is a
    // ttipc procedure now.
    tauri::Builder::default()
        .invoke_handler(ttipc::handler(
            Desk {
                levels: Mutex::new([0u8; 512]),
            }
            .into_procedures(),
        ))
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ---------------------------------------------------------------------
// This file IS the target now: one trait, one impl, one `handler`; the
// Error and Event derives; generated bindings; sync set/level dispatched
// inline and async save_scene on the runtime.
//
// The early sketch here imagined `RefCell<[u8; 512]>` levels as the
// "sync-first finish". That is ruled out, not pending: tauri's
// `invoke_handler` is `Fn(..) -> bool + Send + Sync`, so the captured
// impl must be `Sync`, which `RefCell` is not -- and ttipc bans the
// `unsafe impl Send/Sync` escape tauri uses internally. So shared state
// stays `Mutex` (uncontended, ~ns), and an async procedure's `&self` is
// `Send` for free. Sync-first's real win is the inline dispatch above --
// no spawn, no executor -- not `RefCell` vs `Mutex`. Receipts:
// docs/tauri-threading.md and tests/compile_fail/refcell_state.rs.
// ---------------------------------------------------------------------
