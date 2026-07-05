use anyhow::{Context, Result};
use wasmtime::*;

pub struct WasmRunner {
    engine: Engine,
    module: Module,
}

impl WasmRunner {
    pub fn load(wasm: &[u8]) -> Result<Self> {
        let engine = Engine::default();
        let module = Module::from_binary(&engine, wasm).context("parse wasm module")?;
        Ok(Self { engine, module })
    }

    /// Run one player tick. Returns Ok(true) if the script finished, Ok(false) if it yielded.
    pub fn tick(&self) -> Result<bool> {
        let mut store = Store::new(&self.engine, ());
        let instance = Instance::new(&mut store, &self.module, &[])
            .context("instantiate wasm module")?;
        let realm_tick = instance
            .get_typed_func::<(), i32>(&mut store, "realm_tick")
            .context("export realm_tick")?;
        let result = realm_tick.call(&mut store, ()).context("call realm_tick")?;
        Ok(result != 0)
    }
}