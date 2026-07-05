//! # Creeps SDK
//!
//! Write colony AI in `~/.creeps/colony/src/lib.rs` (or any directory you set in-game
//! with F2). The client watches that folder and hot-reloads WASM on every save.
//!
//! ```text
//! rustup target add wasm32-unknown-unknown
//! code ~/.creeps/colony
//! cargo run -p realm-game
//! ```

/// Opaque handle to the simulation context (implemented by the game host).
pub struct LordContext {
    _private: (),
}

/// Player logic entry point — implement this in your colony `lib.rs` and export
/// via `realm_tick` (see `templates/colony/src/lib.rs`).
pub fn tick(_ctx: &mut LordContext) -> bool {
    true
}

/// Log a line to the in-game console (host provides implementation).
#[inline(always)]
pub fn log(_msg: &str) {
    // Host links this to ScriptLog messages once WASM imports are wired.
}