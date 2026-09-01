//! WebAssembly-facing entry points for TRI-SYNC.
//!
//! This module exposes a minimal, C-ABI compatible surface that can be
//! compiled to `wasm32-unknown-unknown` and called from JS or a host
//! runtime. For now, it’s a stub that proves the build works.

#[no_mangle]
pub extern "C" fn replay_log_status() -> u32 {
    // TODO: wire this into the real TRI-SYNC core:
    // - accept input via linear memory
    // - run ReplayEngine
    // - return a status code or digest handle
    0
}
