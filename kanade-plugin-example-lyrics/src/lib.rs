mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "kanade-plugin",
    });

    export!(LyricsPlugin);
}

struct LyricsPlugin;

impl bindings::exports::kanade::plugin::plugin::Guest for LyricsPlugin {
    fn on_event(event: bindings::exports::kanade::plugin::plugin::PlaybackEvent) {
        match event {
            bindings::exports::kanade::plugin::plugin::PlaybackEvent::TrackChanged((
                prev,
                current,
            )) => {
                bindings::kanade::plugin::host::log(&format!(
                    "♪ Now playing: {} - {}",
                    current.artist.as_deref().unwrap_or("?"),
                    current.title.as_deref().unwrap_or("?"),
                ));
            }
            _ => {}
        }
    }

    fn name() -> String {
        "example-lyrics".to_string()
    }
}
