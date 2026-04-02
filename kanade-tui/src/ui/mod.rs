pub mod now_playing;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame,
};

use crate::app::{App, Panel};
use kanade_core::state::PlaybackState;

pub fn draw(f: &mut Frame, app: &App, state: &PlaybackState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(f.area());

    render_tabs(f, chunks[0], app);

    match app.active_panel {
        Panel::NowPlaying => now_playing::draw(f, chunks[1], state),
        Panel::Queue => render_queue(f, chunks[1], app, state),
        Panel::Library => render_library(f, chunks[1], app),
        Panel::Search => render_search(f, chunks[1], app),
    }
}

fn render_tabs(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let titles: Vec<Line> = Panel::all()
        .iter()
        .map(|p| {
            let style = if *p == app.active_panel {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Line::from(Span::styled(p.title(), style))
        })
        .collect();

    let idx = Panel::all()
        .iter()
        .position(|p| *p == app.active_panel)
        .unwrap_or(0);

    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::NONE))
        .select(idx)
        .divider(Span::raw(" | "));

    f.render_widget(tabs, area);
}

fn render_queue(f: &mut Frame, area: ratatui::layout::Rect, app: &App, state: &PlaybackState) {
    let zone = state.zone(&app.active_zone_id);
    let queue = zone.map(|z| z.queue.as_slice()).unwrap_or(&[]);

    let items: Vec<ListItem> = queue
        .iter()
        .map(|track| {
            let title = track.title.as_deref().unwrap_or("(untitled)");
            let artist = track.artist.as_deref().unwrap_or("(unknown)");
            ListItem::new(format!("{title} - {artist}"))
        })
        .collect();

    let highlight = Style::default().bg(Color::DarkGray).fg(Color::White);

    let list = List::new(items)
        .block(Block::default().title("Queue").borders(Borders::ALL))
        .highlight_style(highlight)
        .highlight_symbol("> ");

    let mut list_state = app.queue_list_state.clone();
    if !queue.is_empty() && list_state.selected().is_none() {
        list_state.select(Some(0));
    }
    if !queue.is_empty() {
        if let Some(sel) = list_state.selected() {
            if sel >= queue.len() {
                list_state.select(Some(queue.len() - 1));
            }
        }
    }
    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_library(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let (title, items): (String, Vec<ListItem>) = if app.in_album_view {
        (
            "Tracks".to_string(),
            app.album_tracks
                .iter()
                .map(|t| {
                    let name = t.title.as_deref().unwrap_or("(untitled)");
                    let artist = t.artist.as_deref().unwrap_or("");
                    if artist.is_empty() {
                        ListItem::new(name.to_string())
                    } else {
                        ListItem::new(format!("{name} - {artist}"))
                    }
                })
                .collect(),
        )
    } else {
        (
            "Library".to_string(),
            app.albums
                .iter()
                .map(|album| {
                    let t = album.title.as_deref().unwrap_or("(untitled album)");
                    ListItem::new(t.to_string())
                })
                .collect(),
        )
    };

    let has_items = !items.is_empty();

    let highlight = Style::default().bg(Color::DarkGray).fg(Color::White);

    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(highlight)
        .highlight_symbol("> ");

    let mut list_state = app.library_list_state.clone();
    if has_items && list_state.selected().is_none() {
        list_state.select(Some(0));
    }
    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_search(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let query_line = if app.search_query.is_empty() {
        Line::from(Span::styled(
            "Type to search...",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(Span::styled(
            format!("/{}", app.search_query),
            Style::default().fg(Color::Yellow),
        ))
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let search_input =
        Paragraph::new(query_line).block(Block::default().title("Search").borders(Borders::ALL));
    f.render_widget(search_input, chunks[0]);

    let results: Vec<ListItem> = app
        .search_results
        .iter()
        .map(|t| {
            let name = t.title.as_deref().unwrap_or("(untitled)");
            let artist = t.artist.as_deref().unwrap_or("");
            if artist.is_empty() {
                ListItem::new(name.to_string())
            } else {
                ListItem::new(format!("{name} - {artist}"))
            }
        })
        .collect();

    let results_list =
        List::new(results).block(Block::default().title("Results").borders(Borders::ALL));
    f.render_widget(results_list, chunks[1]);
}
