//! Microbenchmark: emitting a typed event through tauri's mock runtime
//! -- the derive's match and payload build plus tauri's emit machinery.
//! Fixtures: ../src/lib.rs.

use ttipc_tests::{FaderEvent, mock_webview};

fn main() {
    divan::main();
}

#[divan::bench]
fn emit(bencher: divan::Bencher) {
    let (app, _webview) = mock_webview(|_| false);
    let handle = app.handle().clone();
    bencher.bench(|| {
        FaderEvent::Changed {
            channel: 3,
            value: 200,
        }
        .emit(&handle)
    });
}

#[divan::bench]
fn emit_to(bencher: divan::Bencher) {
    let (app, _webview) = mock_webview(|_| false);
    let handle = app.handle().clone();
    bencher.bench(|| {
        FaderEvent::Changed {
            channel: 3,
            value: 200,
        }
        .emit_to(&handle, "main")
    });
}
