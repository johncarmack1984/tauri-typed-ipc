#[ttipc::procedures]
trait Backup {
    async fn snapshot(&self, app: tauri::AppHandle, label: String) -> String;
}

fn main() {}
