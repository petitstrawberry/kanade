use std::collections::HashMap;

use wasmtime::component::ResourceTable;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView};

use crate::bindings;

pub struct HostState {
    pub config: HashMap<String, String>,
    pub table: ResourceTable,
    pub wasi: WasiCtx,
}

impl HostState {
    pub fn new(config: HashMap<String, String>) -> Self {
        Self {
            config,
            table: ResourceTable::new(),
            wasi: WasiCtxBuilder::new().inherit_stdio().build(),
        }
    }
}

impl wasmtime_wasi::WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// Required by bindgen's add_to_linker so HostWithStore is satisfied
impl wasmtime::component::HasData for HostState {
    type Data<'a> = &'a mut HostState;
}

impl bindings::kanade::plugin::host::Host for HostState {
    fn http_post(&mut self, url: String, body: String) -> Result<String, String> {
        let _ = (url, body);
        Err("http-post not yet implemented".to_string())
    }

    fn get_config(&mut self, key: String) -> Option<String> {
        self.config.get(&key).cloned()
    }

    fn log(&mut self, message: String) {
        tracing::info!(target: "wasm_plugin", %message);
    }
}
