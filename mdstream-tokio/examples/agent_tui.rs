use std::io;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use mdstream::StreamEngine;
use mdstream_protocol::{DocumentLifecycle, Reducer};
use mdstream_tokio::{ActorCommand, CoalescePreset, StreamEngineActor, spawn_stream_engine_actor};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::mpsc;

struct App {
    reducer: Reducer,
    actor_open: bool,
    follow_tail: bool,
    scroll_y: u16,
    batches: u64,
    changes: u64,
    last_error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            reducer: Reducer::new(),
            actor_open: true,
            follow_tail: true,
            scroll_y: 0,
            batches: 0,
            changes: 0,
            last_error: None,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (input, input_rx) = mpsc::channel(64);
    let mut actor = spawn_stream_engine_actor(
        StreamEngine::new(),
        input_rx,
        CoalescePreset::Balanced.options(),
    );
    tokio::spawn(demo_stream(input));

    let (events, mut event_rx) = mpsc::channel(64);
    std::thread::spawn(move || {
        loop {
            if let Ok(true) = crossterm::event::poll(Duration::from_millis(50))
                && let Ok(event) = crossterm::event::read()
                && events.blocking_send(event).is_err()
            {
                break;
            }
        }
    });

    let result = run(
        &mut terminal,
        &mut App::default(),
        &mut actor,
        &mut event_rx,
    )
    .await;
    actor.close_output();
    if let Ok(Ok(unread)) = tokio::time::timeout(Duration::from_secs(1), actor.join()).await {
        drop(unread);
    }

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

async fn run<B>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    actor: &mut StreamEngineActor,
    events: &mut mpsc::Receiver<Event>,
) -> io::Result<()>
where
    B: ratatui::backend::Backend,
    io::Error: From<B::Error>,
{
    loop {
        terminal.draw(|frame| {
            let [main, status] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .areas(frame.area());
            let block = Block::default()
                .title("mdstream change-set actor")
                .borders(Borders::ALL);
            let inner = block.inner(main);
            frame.render_widget(block, main);

            let source = app
                .reducer
                .document()
                .map_or(String::new(), |document| document.source().to_string());
            let line_count = source.lines().count().max(1) as u16;
            if app.follow_tail {
                app.scroll_y = line_count.saturating_sub(inner.height);
            }
            frame.render_widget(
                Paragraph::new(source)
                    .wrap(Wrap { trim: false })
                    .scroll((app.scroll_y, 0)),
                inner,
            );
            frame.render_widget(Paragraph::new(status_line(app)), status);
        })?;

        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { return Ok(()); };
                if handle_event(app, event) {
                    return Ok(());
                }
            }
            result = actor.recv(), if app.actor_open => {
                match result {
                    Some(Ok(batch)) => {
                        app.batches = app.batches.saturating_add(1);
                        app.changes = app.changes.saturating_add(batch.len() as u64);
                        for change in batch {
                            if let Err(error) = app.reducer.apply(change) {
                                app.last_error = Some(error.to_string());
                                break;
                            }
                        }
                    }
                    Some(Err(error)) => app.last_error = Some(error.to_string()),
                    None => app.actor_open = false,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(16)) => {}
        }
    }
}

fn handle_event(app: &mut App, event: Event) -> bool {
    let Event::Key(key) = event else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }

    match key.code {
        KeyCode::Char('q') => true,
        KeyCode::Char('f') => {
            app.follow_tail = !app.follow_tail;
            false
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.follow_tail = false;
            app.scroll_y = app.scroll_y.saturating_add(1);
            false
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.follow_tail = false;
            app.scroll_y = app.scroll_y.saturating_sub(1);
            false
        }
        KeyCode::Char('g') | KeyCode::Home => {
            app.follow_tail = false;
            app.scroll_y = 0;
            false
        }
        KeyCode::Char('G') | KeyCode::End => {
            app.follow_tail = true;
            false
        }
        _ => false,
    }
}

fn status_line(app: &App) -> String {
    let (lifecycle, epoch, sequence, nodes) = app.reducer.document().map_or_else(
        || (DocumentLifecycle::Open, 0, 0, 0),
        |document| {
            (
                document.lifecycle(),
                document.coordinate().epoch.get(),
                document.coordinate().sequence.get(),
                document.nodes().len(),
            )
        },
    );
    let error = app.last_error.as_deref().unwrap_or("-");
    format!(
        "q quit | j/k scroll | g/G top/bottom | f follow={} | actor={} | {:?} epoch={} seq={} nodes={} | batches={} changes={} | error={}",
        app.follow_tail,
        if app.actor_open { "open" } else { "closed" },
        lifecycle,
        epoch,
        sequence,
        nodes,
        app.batches,
        app.changes,
        error,
    )
}

async fn demo_stream(input: mpsc::Sender<ActorCommand>) {
    let markdown = r#"# mdstream actor demo

This example sends token chunks through a bounded Tokio channel.

- Adjacent chunks are coalesced without loss.
- Each output item is one atomic change-set batch.
- Closing input finalizes the canonical document exactly once.

```rust
let change_sets = engine.append(chunk)?;
```

Done.
"#;

    for character in markdown.chars() {
        if input
            .send(ActorCommand::Append(character.to_string()))
            .await
            .is_err()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(4)).await;
    }
}
