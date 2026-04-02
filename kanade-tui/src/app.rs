use crossterm::event::KeyCode;
use kanade_adapter_ws::command::{ClientMessage, WsCommand, WsRequest, WsResponse};
use kanade_core::state::PlaybackState;
use ratatui::widgets::ListState;
use tokio::sync::mpsc;

use crate::AppEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    NowPlaying,
    Queue,
    Library,
    Search,
}

impl Panel {
    pub fn title(&self) -> &str {
        match self {
            Panel::NowPlaying => "Now Playing",
            Panel::Queue => "Queue",
            Panel::Library => "Library",
            Panel::Search => "Search",
        }
    }

    fn next(&self) -> Self {
        match self {
            Panel::NowPlaying => Panel::Queue,
            Panel::Queue => Panel::Library,
            Panel::Library => Panel::Search,
            Panel::Search => Panel::NowPlaying,
        }
    }

    fn prev(&self) -> Self {
        match self {
            Panel::NowPlaying => Panel::Search,
            Panel::Queue => Panel::NowPlaying,
            Panel::Library => Panel::Queue,
            Panel::Search => Panel::Library,
        }
    }

    pub fn all() -> &'static [Panel] {
        &[Panel::NowPlaying, Panel::Queue, Panel::Library, Panel::Search]
    }
}

pub struct App {
    pub ws_tx: mpsc::Sender<ClientMessage>,
    pub active_panel: Panel,
    pub should_quit: bool,
    pub queue_list_state: ListState,
    pub library_list_state: ListState,
    pub search_query: String,
    pub search_results: Vec<kanade_core::model::Track>,
    pub albums: Vec<kanade_core::model::Album>,
    pub album_tracks: Vec<kanade_core::model::Track>,
    pub selected_album_idx: Option<usize>,
    pub in_album_view: bool,
    pub active_zone_id: String,
    pub req_counter: u64,
}

impl App {
    pub fn new(ws_tx: mpsc::Sender<ClientMessage>) -> Self {
        let tx = ws_tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(ClientMessage::Request {
                req_id: 1,
                req: WsRequest::GetAlbums,
            }).await;
        });

        Self {
            ws_tx,
            active_panel: Panel::NowPlaying,
            should_quit: false,
            queue_list_state: ListState::default(),
            library_list_state: ListState::default(),
            search_query: String::new(),
            search_results: Vec::new(),
            albums: Vec::new(),
            album_tracks: Vec::new(),
            selected_album_idx: None,
            in_album_view: false,
            active_zone_id: "default".to_string(),
            req_counter: 1,
        }
    }

    pub fn handle_response(&mut self, data: WsResponse) {
        match data {
            WsResponse::Albums { albums } => {
                self.albums = albums;
            }
            WsResponse::Tracks { tracks } => {
                self.album_tracks = tracks;
            }
            WsResponse::Queue { tracks, current_index } => {
                // Queue data received — could update local view if needed
                let _ = tracks;
                let _ = current_index;
            }
        }
    }

    pub async fn handle_event(&mut self, event: AppEvent, state: &PlaybackState) {
        let AppEvent::Key(key) = event;
        let zone_id = self.active_zone_id.clone();

        match key.code {
            KeyCode::Char('q') if !self.in_album_view => self.should_quit = true,
            KeyCode::Esc => {
                if self.in_album_view {
                    self.in_album_view = false;
                    self.album_tracks.clear();
                    self.library_list_state.select(None);
                }
            }
            KeyCode::Tab => {
                self.active_panel = self.active_panel.next();
            }
            KeyCode::BackTab => {
                self.active_panel = self.active_panel.prev();
            }
            KeyCode::Char(' ') => {
                let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::Play { zone_id: zone_id.clone() })).await;
            }
            KeyCode::Char('n') => {
                let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::Next { zone_id: zone_id.clone() })).await;
            }
            KeyCode::Char('p') => {
                let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::Previous { zone_id: zone_id.clone() })).await;
            }
            KeyCode::Char('s') => {
                let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::Stop { zone_id: zone_id.clone() })).await;
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if let Some(zone) = state.zone(&zone_id) {
                    let vol = zone.volume.saturating_add(5).min(100);
                    let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::SetVolume { zone_id: zone_id.clone(), volume: vol })).await;
                }
            }
            KeyCode::Char('-') => {
                if let Some(zone) = state.zone(&zone_id) {
                    let vol = zone.volume.saturating_sub(5);
                    let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::SetVolume { zone_id: zone_id.clone(), volume: vol })).await;
                }
            }
            KeyCode::Up => self.select_prev(state),
            KeyCode::Down => self.select_next(state),
            KeyCode::Enter => self.select_item(state).await,
            KeyCode::Char('/') => {
                self.active_panel = Panel::Search;
                self.search_query.clear();
            }
            KeyCode::Char(c) if self.active_panel == Panel::Search => {
                if c == '\n' {
                    // Execute search
                    self.req_counter += 1;
                    let req_id = self.req_counter;
                    let query = self.search_query.clone();
                    let tx = self.ws_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(ClientMessage::Request {
                            req_id,
                            req: WsRequest::Search { query },
                        }).await;
                    });
                } else if c == '\x7f' || c == '\x08' {
                    // Backspace
                    self.search_query.pop();
                } else {
                    self.search_query.push(c);
                }
            }
            _ => {}
        }
    }

    fn select_prev(&mut self, _state: &PlaybackState) {
        match self.active_panel {
            Panel::Queue => {
                self.queue_list_state.select(Some(
                    self.queue_list_state.selected().unwrap_or(0).saturating_sub(1),
                ));
            }
            Panel::Library => {
                self.library_list_state.select(Some(
                    self.library_list_state.selected().unwrap_or(0).saturating_sub(1),
                ));
            }
            Panel::Search | Panel::NowPlaying => {}
        }
    }

    fn select_next(&mut self, state: &PlaybackState) {
        match self.active_panel {
            Panel::Queue => {
                let len = state.zone(&self.active_zone_id)
                    .map(|z| z.queue.len())
                    .unwrap_or(0);
                let current = self.queue_list_state.selected().unwrap_or(0);
                if len == 0 || current + 1 < len {
                    self.queue_list_state.select(Some(current + 1));
                }
            }
            Panel::Library => {
                let items = if self.in_album_view {
                    self.album_tracks.len()
                } else {
                    self.albums.len()
                };
                let current = self.library_list_state.selected().unwrap_or(0);
                if items > 0 && current + 1 < items {
                    self.library_list_state.select(Some(current + 1));
                }
            }
            Panel::Search | Panel::NowPlaying => {}
        }
    }

    async fn select_item(&mut self, _state: &PlaybackState) {
        if self.active_panel != Panel::Library {
            return;
        }

        let zone_id = self.active_zone_id.clone();

        if self.in_album_view {
            if let Some(idx) = self.library_list_state.selected() {
                if let Some(track) = self.album_tracks.get(idx).cloned() {
                    let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::AddToQueue {
                        zone_id,
                        track,
                    })).await;
                }
            }
            return;
        }

        if let Some(idx) = self.library_list_state.selected() {
            if let Some(album) = self.albums.get(idx) {
                self.req_counter += 1;
                let req_id = self.req_counter;
                let album_id = album.id.clone();
                let tx = self.ws_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(ClientMessage::Request {
                        req_id,
                        req: WsRequest::GetAlbumTracks { album_id },
                    }).await;
                });
                self.in_album_view = true;
                self.library_list_state.select(None);
            }
        }
    }
}
