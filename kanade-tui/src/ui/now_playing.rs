use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Gauge, Paragraph},
    Frame,
};

use kanade_core::{
    model::{PlaybackStatus, Zone},
    state::PlaybackState,
};

pub fn draw(f: &mut Frame, area: Rect, state: &PlaybackState) {
    let zone = state.zones.first();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    render_track_info(f, chunks[0], zone);
    render_progress(f, chunks[1], zone);
    render_status(f, chunks[2], zone);
    render_details(f, chunks[3], zone);
}

fn render_track_info(f: &mut Frame, area: Rect, zone: Option<&Zone>) {
    let current = zone.and_then(|z| z.current_track());

    let (title, artist, album) = current
        .map(|t| {
            (
                t.title.as_deref().unwrap_or("(no title)"),
                t.artist.as_deref().unwrap_or("(no artist)"),
                t.album_title.as_deref().unwrap_or(""),
            )
        })
        .unwrap_or(("(no track)", "(no artist)", ""));

    let title_line = Line::from(vec![Span::styled(
        title,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )]);
    let artist_line = Line::from(vec![
        Span::raw(artist),
        Span::styled(" - ", Style::default().fg(Color::DarkGray)),
        Span::styled(album, Style::default().fg(Color::DarkGray)),
    ]);

    let content = Paragraph::new(vec![title_line, artist_line]);
    f.render_widget(content, area);
}

fn render_progress(f: &mut Frame, area: Rect, zone: Option<&Zone>) {
    let (duration, position) = zone
        .and_then(|z| {
            let d = z.current_track()?.duration_secs?;
            Some((d, z.position_secs))
        })
        .unwrap_or((0.0, 0.0));

    let ratio = if duration > 0.0 && position > 0.0 {
        position / duration
    } else {
        0.0
    };
    let ratio = ratio.clamp(0.0, 1.0);

    let progress_text = format!("{} / {:.0}", format_time(position), duration);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .ratio(ratio)
        .label(Span::styled(
            progress_text,
            Style::default().fg(Color::White),
        ));

    f.render_widget(gauge, area);
}

fn render_status(f: &mut Frame, area: Rect, zone: Option<&Zone>) {
    let (status_str, volume, repeat_str) = zone
        .map(|z| {
            let s = match z.status {
                PlaybackStatus::Playing => "> Playing",
                PlaybackStatus::Paused => "|| Paused",
                PlaybackStatus::Stopped => "[] Stopped",
                PlaybackStatus::Loading => "~ Loading",
            };
            let r = match z.repeat {
                kanade_core::model::RepeatMode::Off => "",
                kanade_core::model::RepeatMode::One => " [Repeat One]",
                kanade_core::model::RepeatMode::All => " [Repeat All]",
            };
            (s, z.volume, r)
        })
        .unwrap_or(("[] Stopped", 0, ""));

    let vol_text = format!("Vol: {}%", volume);

    let line = Line::from(vec![
        Span::styled(status_str, Style::default().fg(Color::Green)),
        Span::raw("  "),
        Span::styled(&vol_text, Style::default().fg(Color::DarkGray)),
        Span::styled(repeat_str, Style::default().fg(Color::Yellow)),
    ]);

    let content = Paragraph::new(line);
    f.render_widget(content, area);
}

fn render_details(f: &mut Frame, area: Rect, zone: Option<&Zone>) {
    let (format, sample_rate) = zone
        .and_then(|z| {
            let t = z.current_track()?;
            Some((
                t.format.as_deref().unwrap_or("-"),
                t.sample_rate
                    .map(|s| format!("{s} Hz"))
                    .unwrap_or_else(|| "-".to_string()),
            ))
        })
        .unwrap_or(("-", "-".to_string()));

    let line = Line::from(vec![Span::raw(format!(" {}  |  {}", format, sample_rate))]);
    let content = Paragraph::new(line);
    f.render_widget(content, area);
}

fn format_time(secs: f64) -> String {
    let mins = (secs as u64) / 60;
    let secs = (secs as u64) % 60;
    format!("{mins}:{secs:02}")
}
