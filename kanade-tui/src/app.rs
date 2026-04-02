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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryMode {
    Albums,
    Artists,
    Genres,
}

impl LibraryMode {
    pub fn label(&self) -> &'static str {
        match self {
            LibraryMode::Albums => "Albums",
            LibraryMode::Artists => "Artists",
            LibraryMode::Genres => "Genres",
        }
    }

    fn next(self) -> Self {
        match self {
            LibraryMode::Albums => LibraryMode::Artists,
            LibraryMode::Artists => LibraryMode::Genres,
            LibraryMode::Genres => LibraryMode::Albums,
        }
    }

    fn prev(self) -> Self {
        match self {
            LibraryMode::Albums => LibraryMode::Genres,
            LibraryMode::Artists => LibraryMode::Albums,
            LibraryMode::Genres => LibraryMode::Artists,
        }
    }
}

pub struct App {
    pub ws_tx: mpsc::Sender<ClientMessage>,
    pub active_panel: Panel,
    pub should_quit: bool,
    pub queue_list: RefCell<ListState>,
    pub library_list: RefCell<ListState>,
    pub library_detail: RefCell<ListState>,
    pub search_list: RefCell<ListState>,
    pub search_query: String,
    pub search_results: Vec<kanade_core::model::Track>,
    pub albums: Vec<kanade_core::model::Album>,
    pub album_tracks: Vec<kanade_core::model::Track>,
    pub artists: Vec<String>,
    pub artist_albums: Vec<kanade_core::model::Album>,
    pub artist_tracks: Vec<kanade_core::model::Track>,
    pub genres: Vec<String>,
    pub genre_albums: Vec<kanade_core::model::Album>,
    pub genre_tracks: Vec<kanade_core::model::Track>,
    pub library_mode: LibraryMode,
    pub library_level: u8,
    pub library_selected_artist: Option<String>,
    pub library_selected_genre: Option<String>,
    pub active_zone_id: String,
    pub req_counter: u64,
    pub in_search_input: bool,
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
            library_detail: RefCell::new(ListState::default()),
            search_list: RefCell::new(ListState::default()),
            search_query: String::new(),
            search_results: Vec::new(),
            albums: Vec::new(),
            album_tracks: Vec::new(),
            artists: Vec::new(),
            artist_albums: Vec::new(),
            artist_tracks: Vec::new(),
            genres: Vec::new(),
            genre_albums: Vec::new(),
            genre_tracks: Vec::new(),
            library_mode: LibraryMode::Albums,
            library_level: 0,
            library_selected_artist: None,
            library_selected_genre: None,
            active_zone_id: "default".to_string(),
            req_counter: 1,
            in_search_input: false,
        }
    }

    fn library_master_len(&self) -> usize {
        match self.library_mode {
            LibraryMode::Albums => self.albums.len(),
            LibraryMode::Artists => self.artists.len(),
            LibraryMode::Genres => self.genres.len(),
        }
    }

    pub fn library_browse_tracks(&self) -> &[kanade_core::model::Track] {
        match self.library_mode {
            LibraryMode::Albums => &self.album_tracks,
            LibraryMode::Artists => &self.artist_tracks,
            LibraryMode::Genres => &self.genre_tracks,
        }
    }

    pub fn library_detail_albums(&self) -> &[kanade_core::model::Album] {
        match self.library_mode {
            LibraryMode::Albums => &[],
            LibraryMode::Artists => &self.artist_albums,
            LibraryMode::Genres => &self.genre_albums,
        }
    }

    fn request_library_list(&mut self) {
        self.req_counter += 1;
        let req_id = self.req_counter;
        let tx = self.ws_tx.clone();
        let req = match self.library_mode {
            LibraryMode::Albums => WsRequest::GetAlbums,
            LibraryMode::Artists => WsRequest::GetArtists,
            LibraryMode::Genres => WsRequest::GetGenres,
        };
        tokio::spawn(async move {
            let _ = tx.send(ClientMessage::Request { req_id, req }).await;
        });
    }

    fn library_enter(&mut self) {
        match self.library_mode {
            LibraryMode::Albums => {
                if self.library_level == 0 {
                    self.library_enter_album_tracks();
                }
            }
            LibraryMode::Artists => {
                if self.library_level == 0 {
                    self.library_enter_artist_albums();
                } else if self.library_level == 1 {
                    self.library_enter_artist_album_tracks();
                }
            }
            LibraryMode::Genres => {
                if self.library_level == 0 {
                    self.library_enter_genre_albums();
                } else if self.library_level == 1 {
                    self.library_enter_genre_album_tracks();
                }
            }
        }
    }

    fn library_back(&mut self) {
        if self.library_level == 0 {
            return;
        }
        self.library_level -= 1;
        if self.library_level == 0 {
            self.artist_albums.clear();
            self.artist_tracks.clear();
            self.album_tracks.clear();
            self.genre_albums.clear();
            self.genre_tracks.clear();
            self.library_selected_artist = None;
            self.library_selected_genre = None;
            self.library_detail.borrow_mut().select(None);
        } else if self.library_level == 1 && (self.library_mode == LibraryMode::Artists || self.library_mode == LibraryMode::Genres) {
            self.artist_tracks.clear();
            self.genre_tracks.clear();
            self.library_detail.borrow_mut().select(None);
        }
    }

    fn library_enter_album_tracks(&mut self) {
        let idx = self.library_list.borrow().selected();
        let Some(i) = idx else { return };
        let Some(album) = self.albums.get(i) else { return };

        self.req_counter += 1;
        let req_id = self.req_counter;
        let album_id = album.id.clone();
        let tx = self.ws_tx.clone();
        self.library_level = 1;
        self.library_detail.borrow_mut().select(None);
        tokio::spawn(async move {
            let _ = tx.send(ClientMessage::Request {
                req_id,
                req: WsRequest::GetAlbumTracks { album_id },
            }).await;
        });
    }

    fn library_enter_artist_albums(&mut self) {
        let idx = self.library_list.borrow().selected();
        let Some(i) = idx else { return };
        let Some(artist) = self.artists.get(i) else { return };

        let artist = artist.clone();
        self.library_selected_artist = Some(artist.clone());
        self.req_counter += 1;
        let req_id = self.req_counter;
        let tx = self.ws_tx.clone();
        self.library_level = 1;
        self.library_detail.borrow_mut().select(None);
        tokio::spawn(async move {
            let _ = tx.send(ClientMessage::Request {
                req_id,
                req: WsRequest::GetArtistAlbums { artist },
            }).await;
        });
    }

    fn library_enter_artist_album_tracks(&mut self) {
        let idx = self.library_detail.borrow().selected();
        let Some(i) = idx else { return };
        let artist = match self.library_selected_artist.as_ref() {
            Some(a) => a.clone(),
            None => return,
        };

        if i == 0 {
            self.library_enter_artist_all_tracks(&artist);
        } else {
            let album_idx = i - 1;
            if let Some(album) = self.artist_albums.get(album_idx) {
                self.req_counter += 1;
                let req_id = self.req_counter;
                let album_id = album.id.clone();
                let tx = self.ws_tx.clone();
                self.library_level = 2;
                self.library_detail.borrow_mut().select(None);
                tokio::spawn(async move {
                    let _ = tx.send(ClientMessage::Request {
                        req_id,
                        req: WsRequest::GetAlbumTracks { album_id },
                    }).await;
                });
            }
        }
    }

    fn library_enter_artist_all_tracks(&mut self, artist: &str) {
        self.req_counter += 1;
        let req_id = self.req_counter;
        let artist = artist.to_string();
        let tx = self.ws_tx.clone();
        self.library_level = 2;
        self.library_detail.borrow_mut().select(None);
        tokio::spawn(async move {
            let _ = tx.send(ClientMessage::Request {
                req_id,
                req: WsRequest::GetArtistTracks { artist },
            }).await;
        });
    }

    fn library_enter_genre_albums(&mut self) {
        let idx = self.library_list.borrow().selected();
        let Some(i) = idx else { return };
        let Some(genre) = self.genres.get(i) else { return };

        let genre = genre.clone();
        self.library_selected_genre = Some(genre.clone());
        self.req_counter += 1;
        let req_id = self.req_counter;
        let tx = self.ws_tx.clone();
        self.library_level = 1;
        self.library_detail.borrow_mut().select(None);
        tokio::spawn(async move {
            let _ = tx.send(ClientMessage::Request {
                req_id,
                req: WsRequest::GetGenreAlbums { genre },
            }).await;
        });
    }

    fn library_enter_genre_album_tracks(&mut self) {
        let idx = self.library_detail.borrow().selected();
        let Some(i) = idx else { return };
        let genre = match self.library_selected_genre.as_ref() {
            Some(g) => g.clone(),
            None => return,
        };

        if i == 0 {
            self.library_enter_genre_all_tracks(&genre);
        } else {
            let album_idx = i - 1;
            if let Some(album) = self.genre_albums.get(album_idx) {
                self.req_counter += 1;
                let req_id = self.req_counter;
                let album_id = album.id.clone();
                let tx = self.ws_tx.clone();
                self.library_level = 2;
                self.library_detail.borrow_mut().select(None);
                tokio::spawn(async move {
                    let _ = tx.send(ClientMessage::Request {
                        req_id,
                        req: WsRequest::GetAlbumTracks { album_id },
                    }).await;
                });
            }
        }
    }

    fn library_enter_genre_all_tracks(&mut self, genre: &str) {
        self.req_counter += 1;
        let req_id = self.req_counter;
        let genre = genre.to_string();
        let tx = self.ws_tx.clone();
        self.library_level = 2;
        self.library_detail.borrow_mut().select(None);
        tokio::spawn(async move {
            let _ = tx.send(ClientMessage::Request {
                req_id,
                req: WsRequest::GetGenreTracks { genre },
            }).await;
        });
    }

    pub fn handle_response(&mut self, data: WsResponse) {
        match data {
            WsResponse::Albums { albums } => {
                self.albums = albums;
                self.library_list
                    .borrow_mut()
                    .select(if self.albums.is_empty() { None } else { Some(0) });
            }
            WsResponse::AlbumTracks { tracks } => {
                let empty = tracks.is_empty();
                self.album_tracks = tracks;
                self.library_detail.borrow_mut().select(if empty { None } else { Some(0) });
            }
            WsResponse::Artists { artists } => {
                self.artists = artists;
                self.library_list
                    .borrow_mut()
                    .select(if self.artists.is_empty() { None } else { Some(0) });
            }
            WsResponse::ArtistAlbums { albums } => {
                let empty = albums.is_empty();
                self.artist_albums = albums;
                self.library_detail.borrow_mut().select(if empty { None } else { Some(0) });
            }
            WsResponse::ArtistTracks { tracks } => {
                let empty = tracks.is_empty();
                self.artist_tracks = tracks;
                self.library_detail.borrow_mut().select(if empty { None } else { Some(0) });
            }
            WsResponse::Genres { genres } => {
                self.genres = genres;
                self.library_list
                    .borrow_mut()
                    .select(if self.genres.is_empty() { None } else { Some(0) });
            }
            WsResponse::GenreAlbums { albums } => {
                let empty = albums.is_empty();
                self.genre_albums = albums;
                self.library_detail.borrow_mut().select(if empty { None } else { Some(0) });
            }
            WsResponse::GenreTracks { tracks } => {
                let empty = tracks.is_empty();
                self.genre_tracks = tracks;
                self.library_detail.borrow_mut().select(if empty { None } else { Some(0) });
            }
            WsResponse::SearchResults { tracks } => {
                let empty = tracks.is_empty();
                self.search_results = tracks;
                self.search_list.borrow_mut().select(if empty { None } else { Some(0) });
            }
            WsResponse::Queue { tracks: _, current_index: _ } => {}
        }
    }

    pub async fn handle_event(&mut self, event: AppEvent, state: &PlaybackState) {
        let AppEvent::Key(key) = event;
        let zone_id = self.active_zone_id.clone();

        if self.active_panel == Panel::Search && self.in_search_input {
            match key.code {
                KeyCode::Esc => {
                    self.in_search_input = false;
                    return;
                }
                KeyCode::Enter => {
                    self.in_search_input = false;
                    self.search_list
                        .borrow_mut()
                        .select(if self.search_results.is_empty() { None } else { Some(0) });
                    return;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    if !c.is_control() {
                        self.search_query.push(c);
                    }
                }
                _ => return,
            }

            self.req_counter += 1;
            let req_id = self.req_counter;
            let query = self.search_query.clone();
            let tx = self.ws_tx.clone();
            tokio::spawn(async move {
                let _ = tx
                    .send(ClientMessage::Request {
                        req_id,
                        req: WsRequest::Search { query },
                    })
                    .await;
            });
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => {
                if self.active_panel == Panel::Library && self.library_level > 0 {
                    self.library_back();
                } else if self.active_panel == Panel::Search {
                    self.search_query.clear();
                    self.search_results.clear();
                    self.search_list.borrow_mut().select(None);
                    self.in_search_input = false;
                }
            }
            KeyCode::Tab => {
                if !self.in_search_input {
                    self.active_panel = self.active_panel.next();
                }
            }
            KeyCode::BackTab => {
                if !self.in_search_input {
                    self.active_panel = self.active_panel.prev();
                }
            }
            KeyCode::Char(' ') => {
                if self.active_panel != Panel::Search {
                    let tx = self.ws_tx.clone();
                    let zid = zone_id.clone();
                    let cmd = match state.zone(&zone_id).map(|z| &z.status) {
                        Some(kanade_core::model::PlaybackStatus::Playing) => {
                            WsCommand::Pause { zone_id: zid.clone() }
                        }
                        _ => WsCommand::Play { zone_id: zid.clone() },
                    };
                    tokio::spawn(async move {
                        let _ = tx.send(ClientMessage::Command(cmd)).await;
                    });
                }
            }
            KeyCode::Char('n') => {
                if self.active_panel != Panel::Search {
                    let tx = self.ws_tx.clone();
                    let zid = zone_id.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(ClientMessage::Command(WsCommand::Next { zone_id: zid })).await;
                    });
                }
            }
            KeyCode::Char('p') => {
                if self.active_panel != Panel::Search {
                    let tx = self.ws_tx.clone();
                    let zid = zone_id.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(ClientMessage::Command(WsCommand::Previous { zone_id: zid })).await;
                    });
                }
            }
            KeyCode::Char('s') => {
                if self.active_panel != Panel::Search {
                    let tx = self.ws_tx.clone();
                    let zid = zone_id.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(ClientMessage::Command(WsCommand::Stop { zone_id: zid })).await;
                    });
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if self.active_panel != Panel::Search {
                    if let Some(zone) = state.zone(&zone_id) {
                        let vol = zone.volume.saturating_add(5).min(100);
                        let tx = self.ws_tx.clone();
                        let zid = zone_id.clone();
                        tokio::spawn(async move {
                            let _ = tx.send(ClientMessage::Command(WsCommand::SetVolume {
                                zone_id: zid, volume: vol,
                            })).await;
                        });
                    }
                }
            }
            KeyCode::Char('-') => {
                if self.active_panel != Panel::Search {
                    if let Some(zone) = state.zone(&zone_id) {
                        let vol = zone.volume.saturating_sub(5);
                        let tx = self.ws_tx.clone();
                        let zid = zone_id.clone();
                        tokio::spawn(async move {
                            let _ = tx.send(ClientMessage::Command(WsCommand::SetVolume {
                                zone_id: zid, volume: vol,
                            })).await;
                        });
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(state),
            KeyCode::Down | KeyCode::Char('j') => self.select_next(state),
            KeyCode::Enter => self.select_item(state).await,
            KeyCode::Char('/') => {
                self.active_panel = Panel::Search;
                self.in_search_input = true;
            }
            KeyCode::Char('m') => {
                if self.active_panel == Panel::Library && self.library_level == 0 {
                    self.library_mode = self.library_mode.next();
                    self.request_library_list();
                }
            }
            KeyCode::Char('M') => {
                if self.active_panel == Panel::Library && self.library_level == 0 {
                    self.library_mode = self.library_mode.prev();
                    self.request_library_list();
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.active_panel == Panel::Library {
                    self.library_enter();
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.active_panel == Panel::Library {
                    self.library_back();
                }
            }
            KeyCode::Char('d') => {
                if self.active_panel == Panel::Queue {
                    self.queue_remove(state);
                }
            }
            KeyCode::Char('J') => {
                if self.active_panel == Panel::Queue {
                    self.queue_move_down(state);
                }
            }
            KeyCode::Char('K') => {
                if self.active_panel == Panel::Queue {
                    self.queue_move_up(state);
                }
            }
            _ => {}
        }
    }

    fn select_prev(&self, _state: &PlaybackState) {
        match self.active_panel {
            Panel::Queue => {
                let mut list = self.queue_list.borrow_mut();
                let cur = list.selected().unwrap_or(0);
                if cur > 0 {
                    list.select(Some(cur - 1));
                }
            }
            Panel::Library => {
                if self.library_level > 0 {
                    let len = self.library_browse_tracks().len();
                    let mut list = self.library_detail.borrow_mut();
                    let cur = list.selected().unwrap_or(0);
                    if cur > 0 {
                        list.select(Some(cur - 1));
                    }
                    let _ = len;
                } else {
                    let mut list = self.library_list.borrow_mut();
                    let cur = list.selected().unwrap_or(0);
                    if cur > 0 {
                        list.select(Some(cur - 1));
                    }
                }
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
                if self.library_level > 0 {
                    let len = self.library_browse_tracks().len();
                    let mut list = self.library_detail.borrow_mut();
                    let cur = list.selected().unwrap_or(0);
                    if len > 0 && cur + 1 < len {
                        list.select(Some(cur + 1));
                    }
                } else {
                    let max = self.library_master_len();
                    let mut list = self.library_list.borrow_mut();
                    let cur = list.selected().unwrap_or(0);
                    if max > 0 && cur + 1 < max {
                        list.select(Some(cur + 1));
                    }
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

    fn queue_remove(&self, state: &PlaybackState) {
        let idx = self.queue_list.borrow().selected();
        if let Some(i) = idx {
            let queue_len = state.zone(&self.active_zone_id)
                .map(|z| z.queue.len())
                .unwrap_or(0);
            if i < queue_len {
                let tx = self.ws_tx.clone();
                let zid = self.active_zone_id.clone();
                tokio::spawn(async move {
                    let _ = tx.send(ClientMessage::Command(WsCommand::RemoveFromQueue {
                        zone_id: zid, index: i,
                    })).await;
                });
            }
        }
    }

    fn queue_move_up(&self, state: &PlaybackState) {
        let idx = self.queue_list.borrow().selected();
        if let Some(i) = idx {
            if i == 0 {
                return;
            }
            let queue_len = state.zone(&self.active_zone_id)
                .map(|z| z.queue.len())
                .unwrap_or(0);
            if i < queue_len {
                let tx = self.ws_tx.clone();
                let zid = self.active_zone_id.clone();
                tokio::spawn(async move {
                    let _ = tx.send(ClientMessage::Command(WsCommand::MoveInQueue {
                        zone_id: zid, from: i, to: i - 1,
                    })).await;
                });
                let _ = idx;
                self.queue_list.borrow_mut().select(Some(i - 1));
            }
        }
    }

    fn queue_move_down(&self, state: &PlaybackState) {
        let idx = self.queue_list.borrow().selected();
        if let Some(i) = idx {
            let queue_len = state.zone(&self.active_zone_id)
                .map(|z| z.queue.len())
                .unwrap_or(0);
            if i + 1 >= queue_len {
                return;
            }
            let tx = self.ws_tx.clone();
            let zid = self.active_zone_id.clone();
            tokio::spawn(async move {
                let _ = tx.send(ClientMessage::Command(WsCommand::MoveInQueue {
                    zone_id: zid, from: i, to: i + 1,
                })).await;
            });
            let _ = idx;
            self.queue_list.borrow_mut().select(Some(i + 1));
        }
    }

    async fn select_item(&mut self, _state: &PlaybackState) {
        let zone_id = self.active_zone_id.clone();

        match self.active_panel {
            Panel::Library => {
                if self.library_level > 0 {
                    let idx = self.library_detail.borrow().selected();
                    if let Some(i) = idx {
                        if let Some(track) = self.library_browse_tracks().get(i).cloned() {
                            let tx = self.ws_tx.clone();
                            tokio::spawn(async move {
                                let _ = tx.send(ClientMessage::Command(WsCommand::AddToQueue {
                                    zone_id, track,
                                })).await;
                            });
                        }
                    }
                } else {
                    self.library_enter();
                }
            }
            Panel::Search => {
                let idx = self.search_list.borrow().selected();
                if let Some(i) = idx {
                    if let Some(track) = self.search_results.get(i).cloned() {
                        let tx = self.ws_tx.clone();
                        tokio::spawn(async move {
                            let _ = tx.send(ClientMessage::Command(WsCommand::AddToQueue {
                                zone_id, track,
                            })).await;
                        });
                    }
                }
            }
            Panel::Queue => {
                let idx = self.queue_list.borrow().selected();
                if let Some(i) = idx {
                    let tx = self.ws_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(ClientMessage::Command(WsCommand::PlayIndex {
                            zone_id, index: i,
                        })).await;
                    });
                }
            }
            Panel::NowPlaying => {}
        }
    }
}
