use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Gauge, Paragraph},
    Frame,
};

use kanade_core::{
    model::{Node, PlaybackStatus, RepeatMode},
    state::PlaybackState,
};

pub fn draw(f: &mut Frame, area: Rect, state: &PlaybackState) {
    let node = state
        .selected_node_id
        .as_deref()
        .and_then(|node_id| state.node(node_id));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(7), // track info
            Constraint::Length(1), // progress
            Constraint::Length(1), // status
            Constraint::Min(0),    // details
        ])
        .split(area);

    render_track_info(f, chunks[0], state);
    render_progress(f, chunks[1], node, state);
    render_status(f, chunks[2], node, state);
    render_details(f, chunks[3], node, state);
}

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::default().fg(Color::DarkGray))
}

fn render_track_info(f: &mut Frame, area: Rect, state: &PlaybackState) {
    let current = state.current_track();

    let lines: Vec<Line<'static>> = current
        .map(|t| {
            let title = t.title.as_deref().unwrap_or("(no title)").to_string();
            let artist = t.artist.as_deref().unwrap_or("(no artist)").to_string();
            let album = t.album_title.as_deref().unwrap_or("").to_string();
            let composer = t.composer.as_deref().map(|c| c.to_string());

            let mut lines = vec![
                Line::from(vec![Span::styled(
                    title,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::from(vec![
                    Span::raw(artist),
                    dim(" \u{2014} "),
                    Span::styled(album, Style::default().fg(Color::DarkGray)),
                ]),
            ];

            if let Some(c) = composer {
                lines.push(Line::from(vec![dim(format!("Composer: {c}"))]));
            }

            if let Some(n) = t.track_number {
                let dur = t
                    .duration_secs
                    .map(|d| format!("  |  {}", format_time(d)))
                    .unwrap_or_default();
                lines.push(Line::from(vec![dim(format!("Track {n}")), Span::raw(dur)]));
            } else if let Some(d) = t.duration_secs {
                lines.push(Line::from(vec![dim(format!(
                    "Duration: {}",
                    format_time(d)
                ))]));
            }

            lines
        })
        .unwrap_or_else(|| {
            vec![Line::from(Span::styled(
                "(no track)",
                Style::default().fg(Color::DarkGray),
            ))]
        });

    let content = Paragraph::new(lines);
    f.render_widget(content, area);
}

fn render_progress(f: &mut Frame, area: Rect, node: Option<&Node>, state: &PlaybackState) {
    let (duration, position) = node
        .and_then(|n| {
            let d = state.current_track()?.duration_secs?;
            Some((d, n.position_secs))
        })
        .unwrap_or((0.0, 0.0));

    let ratio = if duration > 0.0 && position > 0.0 {
        position / duration
    } else {
        0.0
    };
    let ratio = ratio.clamp(0.0, 1.0);

    let progress_text = format!("{} / {}", format_time(position), format_time(duration));

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .ratio(ratio)
        .label(Span::styled(
            progress_text,
            Style::default().fg(Color::White),
        ));

    f.render_widget(gauge, area);
}

fn render_status(f: &mut Frame, area: Rect, node: Option<&Node>, state: &PlaybackState) {
    let (status_str, volume, repeat_str, shuffle_str) = node
        .map(|n| {
            let s = match n.status {
                PlaybackStatus::Playing => "> Playing",
                PlaybackStatus::Paused => "|| Paused",
                PlaybackStatus::Stopped => "[] Stopped",
                PlaybackStatus::Loading => "~ Loading",
            };
            let r = match state.repeat {
                RepeatMode::Off => "",
                RepeatMode::One => " [Repeat One]",
                RepeatMode::All => " [Repeat All]",
            };
            let sh = if state.shuffle { " [Shuffle]" } else { "" };
            (s, n.volume, r, sh)
        })
        .unwrap_or(("[] Stopped", 0, "", ""));

    let line = Line::from(vec![
        Span::styled(status_str, Style::default().fg(Color::Green)),
        Span::raw("  "),
        dim(format!("Vol: {}%", volume)),
        Span::styled(shuffle_str, Style::default().fg(Color::Magenta)),
        Span::styled(repeat_str, Style::default().fg(Color::Yellow)),
    ]);

    let content = Paragraph::new(line);
    f.render_widget(content, area);
}

fn render_details(f: &mut Frame, area: Rect, node: Option<&Node>, state: &PlaybackState) {
    let current = state.current_track();

    let mut spans: Vec<Span<'static>> = Vec::new();

    // Technical info
    if let Some(t) = current {
        let fmt = t.format.as_deref().unwrap_or("-");
        let sr = t
            .sample_rate
            .map(|s| format!("{s} Hz"))
            .unwrap_or_else(|| "-".to_string());

        spans.push(dim(format!(" {}  |  {}", fmt, sr)));
    }

    // Node info
    if let Some(n) = node {
        spans.push(Span::styled(
            format!("  |  Node: {}", n.name),
            Style::default().fg(Color::DarkGray),
        ));
        spans.push(dim(format!(
            "  |  Queue: {}/{}",
            state.current_index.map(|i| i + 1).unwrap_or(0),
            state.queue.len()
        )));
    }

    let line = Line::from(spans);
    let content = Paragraph::new(line);
    f.render_widget(content, area);
}

fn format_time(secs: f64) -> String {
    let total = secs as u64;
    let mins = total / 60;
    let secs = total % 60;
    format!("{mins}:{secs:02}")
}
