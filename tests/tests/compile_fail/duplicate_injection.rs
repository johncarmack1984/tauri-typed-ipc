#[ttipc::procedures]
trait Faders {
    fn set(&self, app: tauri::AppHandle, again: tauri::AppHandle, value: u8);
}

fn main() {}
