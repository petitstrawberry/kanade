use anyhow::Result;
use kanade_core::state::PlaybackState;
use kanade_adapter_ws::{ClientMessage, ServerMessage};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use crate::app::App;

pub mod app;
pub mod ui;
pub mod ws;

pub async fn run(
    mut ws_rx: mpsc::Receiver<ServerMessage>,
    ws_tx: mpsc::Sender<ClientMessage>,
) -> Result<()> {
    // Raw mode must be enabled BEFORE spawning the event thread,
    // because crossterm::event::read() requires raw mode on the same terminal.
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;

    // Now spawn the event poll thread (must be after raw mode).
    let mut event_rx = spawn_event_task();

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(ws_tx.clone());
    let mut state = PlaybackState { zones: vec![] };

    loop {
        terminal.draw(|f| ui::draw(f, &app, &state))?;

        let tick = tokio::time::sleep(std::time::Duration::from_millis(100));
        tokio::select! {
            _ = tick => {}
            Some(msg) = ws_rx.recv() => {
                match msg {
                    ServerMessage::State { state: new_state } => {
                        state = new_state;
                    }
                    ServerMessage::Response { req_id: _, data } => {
                        app.handle_response(data);
                    }
                }
            }
            Some(event) = event_rx.recv() => {
                app.handle_event(event, &state).await;
            }
        }

        if app.should_quit {
            break;
        }
    }

    crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}

#[derive(Debug)]
pub enum AppEvent {
    Key(crossterm::event::KeyEvent),
}

/// Must be called AFTER `enable_raw_mode()`.
fn spawn_event_task() -> mpsc::Receiver<AppEvent> {
    let (tx, rx) = mpsc::channel::<AppEvent>(32);
    std::thread::spawn(move || {
        loop {
            if crossterm::event::poll(std::time::Duration::from_millis(50)).unwrap_or(false) {
                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(key)) => {
                        if tx.blocking_send(AppEvent::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => {
                        break;
                    }
                }
            }
        }
    });
    rx
}
