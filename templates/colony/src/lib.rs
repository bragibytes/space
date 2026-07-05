//! Your colony AI — edit this file in VS Code (or any editor).
//! Save → the Creeps client recompiles and reloads automatically.

use realm_sdk::LordContext;

/// Called every simulation tick by the game host.
#[no_mangle]
pub extern "C" fn realm_tick() -> bool {
    tick(&mut LordContext { _private: () })
}

pub fn tick(_ctx: &mut LordContext) -> bool {
    // Starter: no-op. Hardcoded harvest AI runs until you program creeps here.
    // log("tick");
    true
}