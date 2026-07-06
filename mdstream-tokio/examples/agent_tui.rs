use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEventKind;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use mdstream::BlockKind;
use mdstream::DocumentState;
use mdstream::MdStream;
use mdstream::Options;
use mdstream_tokio::BackpressurePolicy;
use mdstream_tokio::CoalescePreset;
use mdstream_tokio::CoalescingReceiver;
use mdstream_tokio::DeltaSender;
use mdstream_tokio::FlushReason;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use std::io;
use std::time::Duration;
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthChar;

#[derive(Debug)]
struct App {
    stream: MdStream,
    state: DocumentState,
    cache: RenderCache,
    producer_policy: BackpressurePolicy,
    follow_tail: bool,
    scroll_y: u16,
    last_pending_kind: Option<BlockKind>,
    coalesce_preset: CoalescePreset,
    coalesce_dirty: bool,
    last_flush_reason: Option<FlushReason>,
    last_flush_merged: usize,
    last_flush_bytes: usize,
    total_in_messages: u64,
    total_out_chunks: u64,
    pending_code_fence_max_lines: usize,
}

#[derive(Debug, Default)]
struct RenderCache {
    width: u16,
    committed_lines: Vec<String>,
    committed_count: usize,
    rebuilt: bool,
}

impl RenderCache {
    fn invalidate(&mut self) {
        self.width = 0;
        self.committed_lines.clear();
        self.committed_count = 0;
        self.rebuilt = true;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> io::Result<()> {
    let producer_policy = parse_policy(std::env::args())?;
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    crossterm::execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Use a bounded channel so producers cannot allocate unbounded buffers.
    let (tx_delta, rx_delta) = mpsc::channel::<String>(64);
    let (tx_ev, mut rx_ev) = mpsc::channel::<Event>(64);

    std::thread::spawn(move || {
        loop {
            if let Ok(true) = crossterm::event::poll(Duration::from_millis(50))
                && let Ok(ev) = crossterm::event::read()
                && tx_ev.blocking_send(ev).is_err()
            {
                break;
            }
        }
    });

    tokio::spawn(async move {
        let mut sender = DeltaSender::new(tx_delta, producer_policy);
        sender.set_local_max_bytes(16 * 1024);
        demo_stream(sender).await;
    });

    let coalesce_preset = CoalescePreset::Balanced;
    let mut rx = CoalescingReceiver::new(rx_delta, coalesce_preset.options());

    let mut app = App {
        stream: MdStream::new(Options::default()),
        state: DocumentState::new(),
        cache: RenderCache::default(),
        producer_policy,
        follow_tail: true,
        scroll_y: 0,
        last_pending_kind: None,
        coalesce_preset,
        coalesce_dirty: false,
        last_flush_reason: None,
        last_flush_merged: 0,
        last_flush_bytes: 0,
        total_in_messages: 0,
        total_out_chunks: 0,
        pending_code_fence_max_lines: 40,
    };

    let res = run(&mut terminal, &mut app, &mut rx, &mut rx_ev).await;

    disable_raw_mode()?;
    crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

async fn run<B>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rx: &mut CoalescingReceiver,
    rx_ev: &mut mpsc::Receiver<Event>,
) -> io::Result<()>
where
    B: ratatui::backend::Backend,
    io::Error: From<B::Error>,
{
    let mut last_area_w: u16 = 0;
    let mut last_area_h: u16 = 0;

    loop {
        terminal.draw(|f| {
            let area = f.area();
            let [main, status] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .areas(area);

            let title = "mdstream-tokio demo (newline/time coalescing, follow-tail)";
            let block = Block::default().title(title).borders(Borders::ALL);
            let inner = block.inner(main);
            f.render_widget(block, main);

            if inner.width != last_area_w {
                last_area_w = inner.width;
                app.cache.invalidate();
            }
            if inner.height != last_area_h {
                last_area_h = inner.height;
            }

            let (lines, total_lines) = build_lines(
                &app.state,
                &mut app.cache,
                inner.width,
                app.pending_code_fence_max_lines,
            );
            if app.follow_tail {
                app.scroll_y = max_scroll(total_lines as u16, inner.height);
            } else {
                app.scroll_y = app
                    .scroll_y
                    .min(max_scroll(total_lines as u16, inner.height));
            }

            let paragraph = Paragraph::new(Text::from(
                lines
                    .into_iter()
                    .map(|s| Line::from(Span::raw(s)))
                    .collect::<Vec<_>>(),
            ))
            .scroll((app.scroll_y, 0));
            f.render_widget(paragraph, inner);

            let status_text = status_line(app, total_lines as u32, inner.height);
            let status_widget =
                Paragraph::new(Text::from(Line::styled(status_text, Style::default())));
            f.render_widget(status_widget, status);
        })?;

        tokio::select! {
            maybe_ev = rx_ev.recv() => {
                let Some(ev) = maybe_ev else { return Ok(()); };
                if handle_event(app, ev) {
                    return Ok(());
                }
                if app.coalesce_dirty {
                    rx.set_options(app.coalesce_preset.options());
                    app.coalesce_dirty = false;
                }
            }
            maybe_chunk = rx.recv_with_meta() => {
                if let Some(chunk) = maybe_chunk {
                    app.last_flush_reason = Some(chunk.reason);
                    app.last_flush_merged = chunk.merged_messages;
                    app.last_flush_bytes = chunk.text.len();
                    app.total_in_messages = app
                        .total_in_messages
                        .saturating_add(chunk.merged_messages as u64);
                    app.total_out_chunks = app.total_out_chunks.saturating_add(1);

                    let u = app.stream.append(&chunk.text);
                    let applied = app.state.apply(u);
                    if applied.reset {
                        app.cache.invalidate();
                        app.scroll_y = 0;
                    }
                    // Best-effort: track pending kind for UI hints.
                    app.last_pending_kind = app.state.pending().map(|p| p.kind);
                } else {
                    // Stream ended: finalize once and keep UI interactive.
                    let u = app.stream.finalize();
                    let applied = app.state.apply(u);
                    if applied.reset {
                        app.cache.invalidate();
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(16)) => {}
        }
    }
}

fn handle_event(app: &mut App, ev: Event) -> bool {
    let Event::Key(key) = ev else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }

    match key.code {
        KeyCode::Char('q') => true,
        KeyCode::Char('f') => {
            app.follow_tail = !app.follow_tail;
            true
        }
        KeyCode::Char('c') => {
            app.coalesce_preset = app.coalesce_preset.next();
            app.coalesce_dirty = true;
            true
        }
        KeyCode::Char(']') => {
            app.pending_code_fence_max_lines = (app.pending_code_fence_max_lines + 10).min(400);
            true
        }
        KeyCode::Char('[') => {
            app.pending_code_fence_max_lines =
                app.pending_code_fence_max_lines.saturating_sub(10).max(10);
            true
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.follow_tail = false;
            app.scroll_y = app.scroll_y.saturating_add(1);
            true
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.follow_tail = false;
            app.scroll_y = app.scroll_y.saturating_sub(1);
            true
        }
        KeyCode::PageDown => {
            app.follow_tail = false;
            app.scroll_y = app.scroll_y.saturating_add(10);
            true
        }
        KeyCode::PageUp => {
            app.follow_tail = false;
            app.scroll_y = app.scroll_y.saturating_sub(10);
            true
        }
        KeyCode::Char('g') | KeyCode::Home => {
            app.follow_tail = false;
            app.scroll_y = 0;
            true
        }
        KeyCode::Char('G') | KeyCode::End => {
            app.follow_tail = true;
            true
        }
        _ => false,
    }
}

fn status_line(app: &App, total_lines: u32, viewport_h: u16) -> String {
    let committed = app.state.committed().len();
    let pending = app.state.pending().is_some();
    let pending_kind = app
        .last_pending_kind
        .map(|k| format!("{k:?}"))
        .unwrap_or("-".to_string());
    let reason = app
        .last_flush_reason
        .map(|r| format!("{r:?}"))
        .unwrap_or("-".to_string());
    format!(
        "q quit | j/k scroll | g/G top/bottom | f follow-tail={} | c coalesce={} | [/] code-tail={} | producer={:?} | committed={} pending={} kind={} | flush={} merged={} bytes={} | in_msgs={} out_chunks={} | lines={} y={} vh={}",
        app.follow_tail,
        app.coalesce_preset.label(),
        app.pending_code_fence_max_lines,
        app.producer_policy,
        committed,
        pending,
        pending_kind,
        reason,
        app.last_flush_merged,
        app.last_flush_bytes,
        app.total_in_messages,
        app.total_out_chunks,
        total_lines,
        app.scroll_y,
        viewport_h,
    )
}

fn parse_policy(mut args: impl Iterator<Item = String>) -> io::Result<BackpressurePolicy> {
    let _exe = args.next();
    let mut policy: Option<BackpressurePolicy> = None;

    while let Some(arg) = args.next() {
        if let Some(v) = arg.strip_prefix("--policy=") {
            policy = Some(parse_policy_value(v)?);
            continue;
        }
        if arg == "--policy" {
            let Some(v) = args.next() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--policy requires a value: block|drop-new|coalesce-local",
                ));
            };
            policy = Some(parse_policy_value(&v)?);
            continue;
        }
        if arg == "-h" || arg == "--help" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: agent_tui [--policy block|drop-new|coalesce-local]",
            ));
        }
    }

    Ok(policy.unwrap_or(BackpressurePolicy::CoalesceLocal))
}

fn parse_policy_value(v: &str) -> io::Result<BackpressurePolicy> {
    match v {
        "block" => Ok(BackpressurePolicy::Block),
        "drop-new" | "dropnew" => Ok(BackpressurePolicy::DropNew),
        "coalesce-local" | "coalescelocal" => Ok(BackpressurePolicy::CoalesceLocal),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown --policy={v} (expected: block|drop-new|coalesce-local)"),
        )),
    }
}

fn build_lines(
    state: &DocumentState,
    cache: &mut RenderCache,
    width: u16,
    pending_code_fence_max_lines: usize,
) -> (Vec<String>, usize) {
    let width = width.max(1);
    if cache.width != width {
        cache.width = width;
        cache.committed_lines.clear();
        cache.committed_count = 0;
        cache.rebuilt = true;
    }

    let committed = state.committed();
    if cache.committed_count > committed.len() {
        cache.invalidate();
    }

    if cache.committed_count < committed.len() {
        for b in &committed[cache.committed_count..] {
            cache
                .committed_lines
                .extend(render_block(b.kind, b.display_or_raw(), width, false));
        }
        cache.committed_count = committed.len();
    }

    let mut out = cache.committed_lines.clone();

    if let Some(p) = state.pending() {
        out.extend(render_pending(
            p.kind,
            p.display_or_raw(),
            width,
            pending_code_fence_max_lines,
        ));
    }

    let total = out.len();
    (out, total)
}

fn render_pending(
    kind: BlockKind,
    text: &str,
    width: u16,
    code_fence_max_lines: usize,
) -> Vec<String> {
    // Gemini CLI style: large pending code fences are truncated to reduce flicker/latency.
    if kind == BlockKind::CodeFence {
        return render_pending_code_fence(text, width, code_fence_max_lines);
    }
    render_block(kind, text, width, true)
}

fn render_pending_code_fence(text: &str, width: u16, max_lines: usize) -> Vec<String> {
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return render_block(BlockKind::CodeFence, text, width, true);
    }

    let total = lines.len();
    let mut kept: Vec<String> = Vec::new();
    if let Some(first) = lines.first().copied() {
        kept.push(first.to_string());
    }
    let hint = format!(
        "… generating more … (showing last {} of {} lines)",
        max_lines.saturating_sub(2),
        total.saturating_sub(1),
    );
    kept.push(hint);
    let tail = lines.split_off(lines.len().saturating_sub(max_lines.saturating_sub(2)));
    kept.extend(tail.into_iter().map(|s| s.to_string()));

    let joined = kept.join("\n");
    render_block(BlockKind::CodeFence, &joined, width, true)
}

fn render_block(kind: BlockKind, text: &str, width: u16, pending: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    let header = if pending {
        format!("--- pending {kind:?} ---")
    } else {
        format!("--- committed {kind:?} ---")
    };
    out.push(header);

    match kind {
        BlockKind::CodeFence | BlockKind::Table => {
            for line in text.lines() {
                out.push(line.to_string());
            }
        }
        _ => {
            for line in text.lines() {
                out.extend(wrap_chars(line, width));
            }
        }
    }

    out.push(String::new());
    out
}

fn wrap_chars(s: &str, width: u16) -> Vec<String> {
    let width = width as usize;
    if width == 0 {
        return vec![];
    }
    if s.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;

    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if cur_w + w > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(ch);
        cur_w += w;
    }

    if !cur.is_empty() {
        out.push(cur);
    }

    out
}

fn max_scroll(content_h: u16, viewport_h: u16) -> u16 {
    content_h.saturating_sub(viewport_h)
}

async fn demo_stream(mut tx: DeltaSender) {
    let md = r#"# mdstream demo

This is a **streaming** example:

- Flush strategy: newline-gated + time window (like Codex / Gemini CLI)
- UI strategy: follow-tail by default

```rs
fn main() {
    println!("hello");
}
```

Now we stream a *large* code block to show pending truncation:

```txt
"#;

    for chunk in chunk_by(md, 3) {
        let _ = tx.send(&chunk).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    for i in 1..=200 {
        let line = format!("{i:04} | The quick brown fox jumps over the lazy dog.\n");
        for chunk in chunk_by(&line, 2) {
            let _ = tx.send(&chunk).await;
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
    }

    let tail = "\n```\n\nDone.\n";
    for chunk in chunk_by(tail, 3) {
        let _ = tx.send(&chunk).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let _ = tx.flush().await;
}

fn chunk_by(s: &str, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        cur.push(ch);
        if cur.chars().count() >= n {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
