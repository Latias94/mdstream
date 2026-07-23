use std::io;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use mdstream::StreamEngine;
use mdstream_protocol::{DocumentLifecycle, Reducer};
use mdstream_tokio::{
    ActorBatch, ActorCommand, ActorExit, CoalesceOptions, StreamEngineActor,
    spawn_stream_engine_actor,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub(crate) const INPUT_CAPACITY: usize = 8;

pub(crate) const DEMO_MARKDOWN: &str = r#"# mdstream actor demo

This example sends token chunks through a bounded Tokio channel.

- Adjacent chunks are coalesced without loss.
- Each output item is one atomic change-set batch.
- Closing input finalizes the canonical document exactly once.

```rust
let change_sets = engine.append(chunk)?;
```

Done.
"#;

struct App {
    reducer: Reducer,
    actor_open: bool,
    follow_tail: bool,
    scroll_y: u16,
    batches: u64,
    changes: u64,
    errors: u64,
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
            errors: 0,
            last_error: None,
        }
    }
}

impl App {
    fn apply_actor_batch(&mut self, batch: ActorBatch) {
        self.batches = self.batches.saturating_add(1);
        self.changes = self.changes.saturating_add(batch.change_count() as u64);
        for change in batch.changes().cloned() {
            if let Err(error) = self.reducer.apply(change) {
                self.errors = self.errors.saturating_add(1);
                self.last_error = Some(error.to_string());
                break;
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SmokeSummary {
    pub(crate) source: String,
    pub(crate) lifecycle: DocumentLifecycle,
    pub(crate) input_capacity: usize,
    pub(crate) commands_sent: u64,
    pub(crate) batches: u64,
    pub(crate) changes: u64,
    pub(crate) errors: u64,
}

pub(crate) fn validate_smoke_summary(summary: &SmokeSummary) -> io::Result<()> {
    if summary.errors != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("smoke actor reported errors={}", summary.errors),
        ));
    }
    if summary.lifecycle != DocumentLifecycle::Finalized {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("smoke document lifecycle={:?}", summary.lifecycle),
        ));
    }
    if summary.source != DEMO_MARKDOWN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "smoke source mismatch: expected {} bytes, received {}",
                DEMO_MARKDOWN.len(),
                summary.source.len()
            ),
        ));
    }
    let expected_commands = DEMO_MARKDOWN.chars().count() as u64;
    if summary.commands_sent != expected_commands {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "smoke commands_sent mismatch: expected {expected_commands}, received {}",
                summary.commands_sent
            ),
        ));
    }
    if summary.input_capacity != INPUT_CAPACITY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "smoke input_capacity mismatch: expected {INPUT_CAPACITY}, received {}",
                summary.input_capacity
            ),
        ));
    }
    if summary.batches == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "smoke actor reported batches=0",
        ));
    }
    if summary.changes < summary.batches {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "smoke counter mismatch: changes={} batches={}",
                summary.changes, summary.batches
            ),
        ));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ProducerCounters {
    commands_sent: u64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [flag] if flag == "--smoke" => {
            let summary = run_smoke().await?;
            validate_smoke_summary(&summary)?;
            println!(
                "SMOKE_OK lifecycle={:?} input_capacity={} commands_sent={} batches={} changes={} errors={}",
                summary.lifecycle,
                summary.input_capacity,
                summary.commands_sent,
                summary.batches,
                summary.changes,
                summary.errors,
            );
            return Ok(());
        }
        [] => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: cargo run -p mdstream-tokio --example agent_tui -- [--smoke]",
            ));
        }
    }

    run_interactive().await
}

async fn run_interactive() -> io::Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (mut actor, producer) = spawn_demo(Duration::from_millis(4));

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
    actor.begin_cancel();
    let actor_result = match tokio::time::timeout(Duration::from_secs(1), actor.join()).await {
        Ok(result) => result,
        Err(_) => actor.join().await,
    }
    .map_err(|error| io::Error::other(format!("actor task failed: {error}")));
    let producer_result = producer
        .await
        .map_err(|error| io::Error::other(format!("demo producer failed: {error}")));

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    producer_result?;
    drop(actor_result?);
    result
}

pub(crate) async fn run_smoke() -> io::Result<SmokeSummary> {
    let (mut actor, producer) = spawn_demo(Duration::ZERO);
    let mut app = App::default();
    while let Some(batch) = actor.recv().await {
        app.apply_actor_batch(batch);
    }
    let unread = actor
        .join()
        .await
        .map_err(|error| io::Error::other(format!("actor task failed: {error}")))?;
    assert!(unread.unread.is_empty());
    if !matches!(unread.exit, ActorExit::Completed(_)) {
        return Err(io::Error::other("actor did not complete normally"));
    }
    let producer = producer
        .await
        .map_err(|error| io::Error::other(format!("demo producer failed: {error}")))?;
    let document = app
        .reducer
        .document()
        .ok_or_else(|| io::Error::other("actor produced no canonical document"))?;

    Ok(SmokeSummary {
        source: document.source().to_string(),
        lifecycle: document.lifecycle(),
        input_capacity: INPUT_CAPACITY,
        commands_sent: producer.commands_sent,
        batches: app.batches,
        changes: app.changes,
        errors: app.errors,
    })
}

fn spawn_demo(delay: Duration) -> (StreamEngineActor, JoinHandle<ProducerCounters>) {
    let (input, input_rx) = mpsc::channel(INPUT_CAPACITY);
    let actor = spawn_stream_engine_actor(
        StreamEngine::new(),
        input_rx,
        CoalesceOptions::new(Duration::from_millis(80), 16 * 1024, 2048),
    );
    let producer = tokio::spawn(demo_stream(input, delay));
    (actor, producer)
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
            batch = actor.recv(), if app.actor_open => {
                match batch {
                    Some(batch) => app.apply_actor_batch(batch),
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
        "q quit | j/k scroll | g/G top/bottom | f follow={} | actor={} | {:?} epoch={} seq={} nodes={} | batches={} changes={} errors={} | error={}",
        app.follow_tail,
        if app.actor_open { "open" } else { "closed" },
        lifecycle,
        epoch,
        sequence,
        nodes,
        app.batches,
        app.changes,
        app.errors,
        error,
    )
}

async fn demo_stream(input: mpsc::Sender<ActorCommand>, delay: Duration) -> ProducerCounters {
    let mut commands_sent = 0_u64;
    for character in DEMO_MARKDOWN.chars() {
        if input
            .send(ActorCommand::Append(character.to_string()))
            .await
            .is_err()
        {
            return ProducerCounters { commands_sent };
        }
        commands_sent = commands_sent.saturating_add(1);
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
    ProducerCounters { commands_sent }
}
