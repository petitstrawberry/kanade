mod bindings;
mod host;

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use kanade_core::model::Track;
use kanade_core::plugin::{KanadePlugin, PlaybackEvent};
use tokio::sync::Mutex;
use wasmtime::component::Component;

use crate::host::HostState;
use bindings::exports::kanade::plugin::plugin::{PlaybackEvent as WitPlaybackEvent, TrackInfo};

pub struct WasmPluginRuntime {
    engine: wasmtime::Engine,
    linker: wasmtime::component::Linker<HostState>,
    config: HashMap<String, String>,
    plugins: Vec<LoadedPlugin>,
}

struct LoadedPlugin {
    name: String,
    instance: bindings::KanadePlugin,
    store: Mutex<wasmtime::Store<HostState>>,
}

impl WasmPluginRuntime {
    pub fn new(config: HashMap<String, String>) -> Result<Self> {
        let engine = wasmtime::Engine::default();
        let mut linker = wasmtime::component::Linker::<HostState>::new(&engine);

        bindings::KanadePlugin::add_to_linker::<HostState, HostState>(
            &mut linker,
            |state: &mut HostState| -> &mut HostState { state },
        )?;
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;

        Ok(Self {
            engine,
            linker,
            config,
            plugins: Vec::new(),
        })
    }

    pub async fn load_plugin(&mut self, path: &Path) -> Result<String> {
        let component = Component::from_file(&self.engine, path)?;
        let mut store = wasmtime::Store::new(&self.engine, HostState::new(self.config.clone()));

        let instance =
            bindings::KanadePlugin::instantiate_async(&mut store, &component, &self.linker).await?;

        // call_name is sync (wasmtime handles async internally)
        let name: String = instance.kanade_plugin_plugin().call_name(&mut store)?;

        self.plugins.push(LoadedPlugin {
            name: name.clone(),
            instance,
            store: Mutex::new(store),
        });

        Ok(name)
    }

    pub async fn load_plugins_from_dir(&mut self, dir: &Path) -> Result<Vec<String>> {
        let mut loaded = Vec::new();

        if !dir.is_dir() {
            return Ok(loaded);
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("wasm") {
                loaded.push(self.load_plugin(&path).await?);
            }
        }

        Ok(loaded)
    }
}

fn convert_track(track: &Track) -> TrackInfo {
    TrackInfo {
        id: track.id.clone(),
        title: track.title.clone(),
        artist: track.artist.clone(),
        album_title: track.album_title.clone(),
        album_artist: track.album_artist.clone(),
        duration_secs: track.duration_secs,
        track_number: track.track_number,
    }
}

fn convert_event(event: &PlaybackEvent) -> WitPlaybackEvent {
    match event {
        PlaybackEvent::TrackChanged { previous, current } => WitPlaybackEvent::TrackChanged((
            previous.as_ref().map(convert_track),
            convert_track(current),
        )),
        PlaybackEvent::PlaybackPaused {
            track,
            position_secs,
        } => WitPlaybackEvent::PlaybackPaused((track.as_ref().map(convert_track), *position_secs)),
        PlaybackEvent::PlaybackResumed {
            track,
            position_secs,
        } => WitPlaybackEvent::PlaybackResumed((track.as_ref().map(convert_track), *position_secs)),
        PlaybackEvent::PlaybackStopped { track } => {
            WitPlaybackEvent::PlaybackStopped(track.as_ref().map(convert_track))
        }
        PlaybackEvent::ScrobblePoint { track } => {
            WitPlaybackEvent::ScrobblePoint(convert_track(track))
        }
    }
}

#[async_trait]
impl KanadePlugin for WasmPluginRuntime {
    fn name(&self) -> &str {
        "wasm-runtime"
    }

    async fn on_event(&self, event: &PlaybackEvent) {
        let wit_event = convert_event(event);

        for plugin in &self.plugins {
            let mut store = plugin.store.lock().await;
            if let Err(error) = plugin
                .instance
                .kanade_plugin_plugin()
                .call_on_event(&mut *store, &wit_event)
            {
                tracing::warn!(plugin = %plugin.name, %error, "wasm plugin on_event failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runtime_has_expected_plugin_identity() {
        let runtime = WasmPluginRuntime::new(HashMap::new()).expect("runtime should initialize");
        assert_eq!(runtime.name(), "wasm-runtime");
    }

    #[tokio::test]
    async fn on_event_is_noop_when_no_plugins_loaded() {
        let runtime = WasmPluginRuntime::new(HashMap::new()).expect("runtime should initialize");
        runtime
            .on_event(&PlaybackEvent::PlaybackStopped { track: None })
            .await;
    }
}
