pub mod now_playing;

use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs},
    Frame,
};

use crate::app::{App, LibraryMode, Panel};
use kanade_core::state::PlaybackState;

pub fn draw(f: &mut Frame, app: &App, state: &PlaybackState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    render_tabs(f, chunks[0], app);

    match app.active_panel {
        Panel::NowPlaying => now_playing::draw(f, chunks[1], state),
        Panel::Queue => render_queue(f, chunks[1], app, state),
        Panel::Library => render_library(f, chunks[1], app),
        Panel::Search => render_search(f, chunks[1], app),
    }

    render_help(f, chunks[2], app);
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
    let current_index = zone.and_then(|z| z.current_index);

    let items: Vec<ListItem> = queue
        .iter()
        .enumerate()
        .map(|(i, track)| {
            let title = track.title.as_deref().unwrap_or("(untitled)");
            let artist = track.artist.as_deref().unwrap_or("(unknown)");
            if current_index == Some(i) {
                ListItem::new(Line::from(vec![
                    Span::styled("▶ ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!("{title} - {artist}"),
                        Style::default().fg(Color::Cyan),
                    ),
                ]))
            } else {
                ListItem::new(format!("  {title} - {artist}"))
            }
        })
        .collect();

    let highlight = Style::default().bg(Color::DarkGray).fg(Color::White);

    let list = List::new(items)
        .block(Block::default().title("Queue").borders(Borders::ALL))
        .highlight_style(highlight)
        .highlight_symbol("> ");

    let mut list_state = app.queue_list.borrow_mut();
    if !queue.is_empty() && list_state.selected().is_none() {
        list_state.select(Some(0));
    }
    if let Some(sel) = list_state.selected() {
        if !queue.is_empty() && sel >= queue.len() {
            list_state.select(Some(queue.len() - 1));
        }
    }
    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_library(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    if app.library_level == 0 {
        render_library_master(f, area, app);
    } else {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Min(0)])
            .split(area);

        render_library_master(f, columns[0], app);
        render_library_right(f, columns[1], app);
    }
}

fn render_library_master(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let mode_label = app.library_mode.label();
    let (items, has_items): (Vec<ListItem>, bool) = match app.library_mode {
        LibraryMode::Albums => {
            let has = !app.albums.is_empty();
            let list = app
                .albums
                .iter()
                .map(|album| {
                    let t = album.title.as_deref().unwrap_or("(untitled album)");
                    ListItem::new(t.to_string())
                })
                .collect();
            (list, has)
        }
        LibraryMode::Artists => {
            let has = !app.artists.is_empty();
            let list = app
                .artists
                .iter()
                .map(|a| ListItem::new(a.clone()))
                .collect();
            (list, has)
        }
        LibraryMode::Genres => {
            let has = !app.genres.is_empty();
            let list = app
                .genres
                .iter()
                .map(|g| ListItem::new(g.clone()))
                .collect();
            (list, has)
        }
    };

    let highlight = Style::default().bg(Color::DarkGray).fg(Color::White);

    let title = format!("{} (m/M:switch)", mode_label);
    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(highlight)
        .highlight_symbol("> ");

    let mut list_state = app.library_list.borrow_mut();
    if has_items && list_state.selected().is_none() {
        list_state.select(Some(0));
    }
    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_library_right(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let highlight = Style::default().bg(Color::DarkGray).fg(Color::White);

    if (app.library_mode == LibraryMode::Artists || app.library_mode == LibraryMode::Genres)
        && app.library_level == 1
    {
        let albums = app.library_detail_albums();
        let mut items: Vec<ListItem> = vec![ListItem::new("All albums")];
        for album in albums {
            let t = album.title.as_deref().unwrap_or("(untitled album)");
            items.push(ListItem::new(t.to_string()));
        }

        let list = List::new(items)
            .block(
                Block::default()
                    .title("Albums (Enter:open)")
                    .borders(Borders::ALL),
            )
            .highlight_style(highlight)
            .highlight_symbol("> ");

        let mut list_state = app.library_detail.borrow_mut();
        if !albums.is_empty() && list_state.selected().is_none() {
            list_state.select(Some(0));
        }
        f.render_stateful_widget(list, area, &mut list_state);
    } else {
        let tracks = app.library_browse_tracks();
        let has_items = !tracks.is_empty();

        let items: Vec<ListItem> = tracks
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

        let list = List::new(items)
            .block(
                Block::default()
                    .title("Tracks (Enter:add, h/Esc:back)")
                    .borders(Borders::ALL),
            )
            .highlight_style(highlight)
            .highlight_symbol("> ");

        let mut list_state = app.library_detail.borrow_mut();
        if has_items && list_state.selected().is_none() {
            list_state.select(Some(0));
        }
        f.render_stateful_widget(list, area, &mut list_state);
    }
}

fn render_search(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let query_line = if app.search_query.is_empty() {
        Line::from(Span::styled(
            if app.in_search_input {
                "Type to search..."
            } else {
                "Press / to start typing..."
            },
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(Span::styled(
            format!("/{}", app.search_query),
            if app.in_search_input {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Yellow)
            },
        ))
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    let search_input = Paragraph::new(query_line).block(
        Block::default()
            .title(if app.in_search_input {
                "Search (Enter: finish, Esc: cancel)"
            } else {
                "Search (/ to edit, Esc: clear)"
            })
            .borders(Borders::ALL),
    );
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

    let highlight = Style::default().bg(Color::DarkGray).fg(Color::White);

    let results_list = List::new(results)
        .block(
            Block::default()
                .title("Results (Enter: add to queue)")
                .borders(Borders::ALL),
        )
        .highlight_style(highlight)
        .highlight_symbol("> ");

    let mut list_state = app.search_list.borrow_mut();
    if !app.search_results.is_empty() && list_state.selected().is_none() {
        list_state.select(Some(0));
    }
    f.render_stateful_widget(results_list, chunks[1], &mut list_state);
}

fn render_help(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let help = match app.active_panel {
        Panel::NowPlaying => {
            "Space:play/pause  n:next  p:prev  s:stop  +/-:vol  Tab:switch  q:quit"
        }
        Panel::Queue => "j/k:nav  Enter:play  d:del  J/K:move  Tab:switch  q:quit",
        Panel::Library => "j/k:nav  l/Enter:open  h/Esc:back  m/M:mode  Tab:switch  q:quit",
        Panel::Search => {
            if app.in_search_input {
                "type:search  Backspace:delete  Enter:finish  Esc:cancel  q:quit"
            } else {
                "/:edit  ↑/↓:navigate  Enter:add to queue  Esc:clear  q:quit"
            }
        }
    };

    let line = Line::from(Span::styled(help, Style::default().fg(Color::DarkGray)));
    let content = Paragraph::new(line);
    f.render_widget(content, area);
}
