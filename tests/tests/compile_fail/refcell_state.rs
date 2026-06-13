use ttipc::procedures;
use std::cell::RefCell;

#[procedures]
trait Desk {
    fn level(&self, channel: u16) -> u8;
}

struct Levels {
    inner: RefCell<[u8; 512]>,
}

impl Desk for Levels {
    fn level(&self, channel: u16) -> u8 {
        self.inner.borrow()[channel as usize]
    }
}

fn main() {
    // RefCell is !Sync, so this set cannot satisfy tauri's Send + Sync
    // invoke_handler bound. Registration fails to compile here -- ttipc
    // bans the unsafe Send/Sync workaround, so shared state must be Sync
    // (a Mutex). See docs/tauri-threading.md.
    let _ = Levels {
        inner: RefCell::new([0; 512]),
    }
    .into_procedures();
}
