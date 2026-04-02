use std::cell::RefCell;

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
    pub queue_list: RefCell<ListState>,
    pub library_list: RefCell<ListState>,
    pub search_list: RefCell<ListState>,
    pub search_query: String,
    pub search_results: Vec<kanade_core::model::Track>,
    pub albums: Vec<kanade_core::model::Album>,
    pub album_tracks: Vec<kanade_core::model::Track>,
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
            queue_list: RefCell::new(ListState::default()),
            library_list: RefCell::new(ListState::default()),
            search_list: RefCell::new(ListState::default()),
            search_query: String::new(),
            search_results: Vec::new(),
            albums: Vec::new(),
            album_tracks: Vec::new(),
            in_album_view: false,
            active_zone_id: "default".to_string(),
            req_counter: 1,
        }
    }

    pub fn handle_response(&mut self, data: WsResponse) {
        match data {
            WsResponse::Albums { albums } => {
                self.albums = albums;
                self.library_list.borrow_mut().select(None);
            }
            WsResponse::Tracks { tracks } => {
                let empty = tracks.is_empty();
                if self.in_album_view {
                    self.album_tracks = tracks;
                    let mut list = self.library_list.borrow_mut();
                    list.select(if empty { None } else { Some(0) });
                } else {
                    self.search_results = tracks;
                    let mut list = self.search_list.borrow_mut();
                    list.select(if empty { None } else { Some(0) });
                }
            }
            WsResponse::Queue { tracks: _, current_index: _ } => {}
        }
    }

    pub async fn handle_event(&mut self, event: AppEvent, state: &PlaybackState) {
        let AppEvent::Key(key) = event;
        let zone_id = self.active_zone_id.clone();

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => {
                if self.in_album_view {
                    self.in_album_view = false;
                    self.album_tracks.clear();
                    self.library_list.borrow_mut().select(None);
                } else if self.active_panel == Panel::Search {
                    self.search_query.clear();
                    self.search_results.clear();
                    self.search_list.borrow_mut().select(None);
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
            KeyCode::Backspace => {
                if self.active_panel == Panel::Search {
                    self.search_query.pop();
                }
            }
            KeyCode::Char(c) if self.active_panel == Panel::Search => {
                if !c.is_control() {
                    self.search_query.push(c);
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
                }
            }
            _ => {}
        }
    }

    fn select_prev(&self, state: &PlaybackState) {
        match self.active_panel {
            Panel::Queue => {
                let mut list = self.queue_list.borrow_mut();
                let cur = list.selected().unwrap_or(0);
                if cur > 0 {
                    list.select(Some(cur - 1));
                }
            }
            Panel::Library => {
                let max = if self.in_album_view {
                    self.album_tracks.len()
                } else {
                    self.albums.len()
                };
                let mut list = self.library_list.borrow_mut();
                let cur = list.selected().unwrap_or(0);
                if cur > 0 {
                    list.select(Some(cur - 1));
                }
                let _ = max;
            }
            Panel::Search => {
                let mut list = self.search_list.borrow_mut();
                let cur = list.selected().unwrap_or(0);
                if cur > 0 {
                    list.select(Some(cur - 1));
                }
            }
            Panel::NowPlaying => {}
        }
    }

    fn select_next(&self, state: &PlaybackState) {
        match self.active_panel {
            Panel::Queue => {
                let len = state.zone(&self.active_zone_id)
                    .map(|z| z.queue.len())
                    .unwrap_or(0);
                let mut list = self.queue_list.borrow_mut();
                let cur = list.selected().unwrap_or(0);
                if len == 0 || cur + 1 < len {
                    list.select(Some(cur + 1));
                }
            }
            Panel::Library => {
                let max = if self.in_album_view {
                    self.album_tracks.len()
                } else {
                    self.albums.len()
                };
                let mut list = self.library_list.borrow_mut();
                let cur = list.selected().unwrap_or(0);
                if max > 0 && cur + 1 < max {
                    list.select(Some(cur + 1));
                }
            }
            Panel::Search => {
                let len = self.search_results.len();
                let mut list = self.search_list.borrow_mut();
                let cur = list.selected().unwrap_or(0);
                if len > 0 && cur + 1 < len {
                    list.select(Some(cur + 1));
                }
            }
            Panel::NowPlaying => {}
        }
    }

    async fn select_item(&mut self, state: &PlaybackState) {
        let zone_id = self.active_zone_id.clone();

        match self.active_panel {
            Panel::Library => {
                if self.in_album_view {
                    let idx = self.library_list.borrow().selected();
                    if let Some(i) = idx {
                        if let Some(track) = self.album_tracks.get(i).cloned() {
                            let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::AddToQueue {
                                zone_id,
                                track,
                            })).await;
                        }
                    }
                } else {
                    let idx = self.library_list.borrow().selected();
                    if let Some(i) = idx {
                        if let Some(album) = self.albums.get(i) {
                            self.req_counter += 1;
                            let req_id = self.req_counter;
                            let album_id = album.id.clone();
                            let tx = self.ws_tx.clone();
                            self.in_album_view = true;
                            tokio::spawn(async move {
                                let _ = tx.send(ClientMessage::Request {
                                    req_id,
                                    req: WsRequest::GetAlbumTracks { album_id },
                                }).await;
                            });
                        }
                    }
                }
            }
            Panel::Search => {
                let idx = self.search_list.borrow().selected();
                if let Some(i) = idx {
                    if let Some(track) = self.search_results.get(i).cloned() {
                        let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::AddToQueue {
                            zone_id,
                            track,
                        })).await;
                    }
                }
            }
            Panel::Queue => {
                let idx = self.queue_list.borrow().selected();
                if let Some(i) = idx {
                    let _ = self.ws_tx.send(ClientMessage::Command(WsCommand::PlayIndex {
                        zone_id,
                        index: i,
                    })).await;
                }
            }
            Panel::NowPlaying => {}
        }
    }
}
