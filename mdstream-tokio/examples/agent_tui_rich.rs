use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use mdstream::StreamEngine;
use mdstream_protocol::{
    ContentKind, ContentNode, Document, DocumentLifecycle, NodeId, NodeVersion, SemanticText,
    TransitionFacts, TransitionReducer,
};
use mdstream_tokio::{
    ActorBatch, ActorCommand, ActorExit, CoalesceOptions, StreamEngineActor,
    spawn_stream_engine_actor,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const INPUT_CAPACITY: usize = 8;
const GRAPHEMES_PER_TICK: usize = 4;
const MAX_HIGHLIGHT_BYTES: usize = 64 * 1024;
const MAX_HIGHLIGHT_CACHE_BYTES: usize = 256 * 1024;
const EVENT_QUEUE_CAPACITY: usize = 64;
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "function",
    "function.builtin",
    "keyword",
    "module",
    "number",
    "operator",
    "property",
    "punctuation",
    "string",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

pub(crate) const DEMO_MARKDOWN: &str = r#"# mdstream agent workbench

I am streaming a typed answer through a bounded Tokio actor. The host renders
mdstream Content IR directly, so timing and terminal presentation do not become
another Markdown parser.

> Canonical content belongs to mdstream. This workbench owns only layout,
> animation, scrolling, highlighting, and the activity timeline.

## Plan

- [x] Read the actor contract.
- [x] Render stable semantic blocks.
- [ ] Settle host-local text animation.

## Streaming implementation

```rust
pub fn present(change_sets: usize) -> String {
    format!("{change_sets} atomic updates")
}
```

The code fence is a typed `CodeBlock`; this example applies Tree-sitter only to
its already-normalized Rust body. It never asks Tree-sitter to parse Markdown.

```json
{
  "renderer": "host-owned",
  "animation": "grapheme-paced",
  "final_state": "canonical"
}
```

## Extension handoff

```mermaid
flowchart LR
  Tokens --> mdstream
  mdstream --> ContentIR
  ContentIR --> Host
  Host --> Merman
```

The Mermaid fence remains a typed node until a host chooses an artifact
processor such as Merman. See [the processor recipe][merman].

[merman]: https://github.com/Latias94/mdstream/tree/main/mdstream-merman
"#;

type UiLine = Line<'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ToolState {
    Queued,
    Running,
    Complete,
}

impl ToolState {
    const fn label(self) -> &'static str {
        match self {
            Self::Queued => "wait",
            Self::Running => "run ",
            Self::Complete => "done",
        }
    }

    fn style(self) -> Style {
        match self {
            Self::Queued => Style::default().fg(Color::DarkGray),
            Self::Running => Style::default().fg(Color::Yellow),
            Self::Complete => Style::default().fg(Color::Green),
        }
    }
}

#[derive(Debug, Clone)]
struct ToolActivity {
    name: &'static str,
    detail: &'static str,
    start_at_percent: usize,
    complete_at_percent: usize,
    state: ToolState,
}

impl ToolActivity {
    fn update(&mut self, progress_percent: usize) {
        let next = if progress_percent >= self.complete_at_percent {
            ToolState::Complete
        } else if progress_percent >= self.start_at_percent {
            ToolState::Running
        } else {
            ToolState::Queued
        };
        self.state = self.state.max(next);
    }
}

fn demo_activities() -> Vec<ToolActivity> {
    vec![
        ToolActivity {
            name: "read",
            detail: "actor contract",
            start_at_percent: 4,
            complete_at_percent: 24,
            state: ToolState::Queued,
        },
        ToolActivity {
            name: "render",
            detail: "semantic blocks",
            start_at_percent: 25,
            complete_at_percent: 63,
            state: ToolState::Queued,
        },
        ToolActivity {
            name: "check",
            detail: "settled state",
            start_at_percent: 64,
            complete_at_percent: 100,
            state: ToolState::Queued,
        },
    ]
}

struct RichApp {
    reducer: TransitionReducer,
    actor_open: bool,
    follow_tail: bool,
    scroll_y: u16,
    wrap: bool,
    paused: bool,
    reduced_motion: bool,
    visible_source_end: usize,
    animation_ticks: u64,
    batches: u64,
    changes: u64,
    errors: u64,
    last_error: Option<String>,
    activities: Vec<ToolActivity>,
    syntax: Option<SyntaxHighlighter>,
}

impl Default for RichApp {
    fn default() -> Self {
        Self {
            reducer: TransitionReducer::new(),
            actor_open: true,
            follow_tail: true,
            scroll_y: 0,
            wrap: true,
            paused: false,
            reduced_motion: false,
            visible_source_end: 0,
            animation_ticks: 0,
            batches: 0,
            changes: 0,
            errors: 0,
            last_error: None,
            activities: demo_activities(),
            syntax: SyntaxHighlighter::new(),
        }
    }
}

impl RichApp {
    fn apply_actor_batch(&mut self, batch: ActorBatch) {
        self.batches = self.batches.saturating_add(1);
        self.changes = self.changes.saturating_add(batch.change_count() as u64);

        for change in batch.changes().cloned() {
            match self.reducer.apply(change) {
                Ok(outcome) => self.observe_transition(outcome.facts.as_ref()),
                Err(error) => {
                    self.errors = self.errors.saturating_add(1);
                    self.last_error = Some(error.to_string());
                    break;
                }
            }
        }
        self.update_activities();
    }

    fn observe_transition(&mut self, facts: Option<&TransitionFacts>) {
        match facts {
            Some(TransitionFacts::FullReplace { .. }) => {
                self.visible_source_end = 0;
                self.activities = demo_activities();
                if let Some(syntax) = &mut self.syntax {
                    syntax.clear();
                }
            }
            Some(TransitionFacts::Continuous { nodes, .. }) => {
                if let Some(syntax) = &mut self.syntax {
                    for node in nodes {
                        if node.after.is_none() {
                            syntax.remove(node.key.node_id);
                        }
                    }
                }
            }
            None => {}
        }
        if self.reduced_motion {
            self.settle_presentation();
        }
    }

    fn advance_presentation(&mut self) {
        if self.paused {
            return;
        }
        let Some(source) = self.reducer.document().map(Document::source) else {
            return;
        };

        let limit = presentation_limit(source, self.actor_open);
        self.visible_source_end = self.visible_source_end.min(limit);
        if self.reduced_motion {
            self.visible_source_end = limit;
        } else {
            for _ in 0..GRAPHEMES_PER_TICK {
                if self.visible_source_end >= limit {
                    break;
                }
                let remaining = source
                    .get(self.visible_source_end..limit)
                    .expect("presentation limits are UTF-8 boundaries");
                let Some(grapheme) = remaining.graphemes(true).next() else {
                    break;
                };
                self.visible_source_end = self
                    .visible_source_end
                    .saturating_add(grapheme.len())
                    .min(limit);
            }
        }
        self.animation_ticks = self.animation_ticks.saturating_add(1);
        self.update_activities();
    }

    fn settle_presentation(&mut self) {
        let Some(source) = self.reducer.document().map(Document::source) else {
            return;
        };
        let limit = presentation_limit(source, self.actor_open);
        self.visible_source_end = self.visible_source_end.min(limit);
        while self.visible_source_end < limit {
            let remaining = source
                .get(self.visible_source_end..limit)
                .expect("grapheme cursor is always a UTF-8 boundary");
            let grapheme = remaining
                .graphemes(true)
                .next()
                .expect("remaining source is non-empty");
            self.visible_source_end = self
                .visible_source_end
                .saturating_add(grapheme.len())
                .min(limit);
            self.animation_ticks = self.animation_ticks.saturating_add(1);
        }
        self.update_activities();
    }

    fn update_activities(&mut self) {
        let (visible, total) = self.reducer.document().map_or((0, 0), |document| {
            (
                self.visible_source_end.min(document.source().len()),
                document.source().len(),
            )
        });
        let progress = if total == 0 {
            0
        } else {
            visible.saturating_mul(100) / total
        };
        for activity in &mut self.activities {
            activity.update(progress);
        }
    }

    fn is_settled(&self) -> bool {
        !self.actor_open
            && self
                .reducer
                .document()
                .is_some_and(|document| self.visible_source_end >= document.source().len())
    }

    fn render_answer(&mut self) -> RenderedAnswer {
        let visible_source_end = self.visible_source_end;
        let settled = self.is_settled();
        let reducer = &self.reducer;
        let syntax = &mut self.syntax;
        let Some(document) = reducer.document() else {
            return RenderedAnswer {
                lines: vec![Line::from(Span::styled(
                    "Waiting for canonical content...",
                    Style::default().fg(Color::DarkGray),
                ))],
                highlighted_segments: 0,
            };
        };

        if let Some(syntax) = syntax {
            syntax.begin_render();
        }
        let mut lines = Vec::new();
        render_blocks(
            document,
            document.roots().as_slice(),
            visible_source_end.min(document.source().len()),
            syntax.as_mut(),
            &mut lines,
        );
        if lines.last().is_some_and(|line| line.spans.is_empty()) {
            lines.pop();
        }
        if !settled {
            lines.push(Line::from(Span::styled(
                "streaming |",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::SLOW_BLINK),
            )));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "Waiting for a stable Markdown block...",
                Style::default().fg(Color::DarkGray),
            )));
        }
        RenderedAnswer {
            lines,
            highlighted_segments: syntax.as_ref().map_or(0, |syntax| syntax.render_segments),
        }
    }

    fn activity_lines(&self) -> Vec<UiLine> {
        let mut lines = vec![
            Line::from(Span::styled(
                "example host activity",
                Style::default().fg(Color::DarkGray),
            )),
            Line::default(),
        ];
        for activity in &self.activities {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("[{}] ", activity.state.label()),
                    activity.state.style(),
                ),
                Span::styled(activity.name, Style::default().add_modifier(Modifier::BOLD)),
            ]));
            lines.push(Line::from(Span::styled(
                format!("      {}", activity.detail),
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            "These are host side-channel events.",
            Style::default().fg(Color::DarkGray),
        )));
        lines
    }

    fn inspector_lines(&self, highlighted_segments: usize) -> Vec<UiLine> {
        let (lifecycle, epoch, sequence, nodes, pending, source_len) =
            self.reducer.document().map_or_else(
                || (DocumentLifecycle::Open, 0, 0, 0, 0, 0),
                |document| {
                    (
                        document.lifecycle(),
                        document.coordinate().epoch.get(),
                        document.coordinate().sequence.get(),
                        document.nodes().len(),
                        document.pending_source().len(),
                        document.source().len(),
                    )
                },
            );
        let error = self.last_error.as_deref().unwrap_or("-");
        vec![
            Line::from(format!("lifecycle  {lifecycle:?}")),
            Line::from(format!("epoch      {epoch}")),
            Line::from(format!("sequence   {sequence}")),
            Line::from(format!("nodes      {nodes}")),
            Line::from(format!(
                "visible    {}/{} bytes",
                self.visible_source_end.min(source_len),
                source_len
            )),
            Line::from(format!("pending    {pending} bytes")),
            Line::default(),
            Line::from(Span::styled(
                "presentation",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("follow     {}", self.follow_tail)),
            Line::from(format!("paused     {}", self.paused)),
            Line::from(format!("motion     {}", motion_label(self.reduced_motion))),
            Line::from(format!("wrap       {}", self.wrap)),
            Line::default(),
            Line::from(Span::styled(
                "Tree-sitter",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("rust + json code fences"),
            Line::from(format!("captures   {highlighted_segments}")),
            Line::from("max code   64 KiB"),
            Line::default(),
            Line::from(Span::styled(
                "transport",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("actor      {}", actor_label(self.actor_open))),
            Line::from(format!("batches    {}", self.batches)),
            Line::from(format!("changes    {}", self.changes)),
            Line::from(format!("errors     {}", self.errors)),
            Line::from(Span::styled(
                format!("last error {error}"),
                Style::default().fg(Color::DarkGray),
            )),
        ]
    }
}

fn motion_label(reduced_motion: bool) -> &'static str {
    if reduced_motion { "reduced" } else { "paced" }
}

fn actor_label(actor_open: bool) -> &'static str {
    if actor_open { "open" } else { "closed" }
}

/// Holds the final grapheme while input can still append to it.
///
/// Unicode clusters such as `e` plus a combining accent and emoji ZWJ sequences
/// can become longer when a later chunk arrives. The host therefore presents
/// every complete prefix, but waits for the trailing cluster until actor input
/// has closed.
fn presentation_limit(source: &str, actor_open: bool) -> usize {
    if !actor_open {
        return source.len();
    }
    source
        .graphemes(true)
        .next_back()
        .map_or(0, |trailing| source.len().saturating_sub(trailing.len()))
}

struct RenderedAnswer {
    lines: Vec<UiLine>,
    highlighted_segments: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeLanguage {
    Rust,
    Json,
}

impl CodeLanguage {
    fn from_fence(language: Option<&str>) -> Option<Self> {
        match language?.to_ascii_lowercase().as_str() {
            "rust" | "rs" => Some(Self::Rust),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct CachedCode {
    version: NodeVersion,
    source: String,
    lines: Vec<UiLine>,
    segments: usize,
    bytes: usize,
}

struct SyntaxHighlighter {
    rust: HighlightConfiguration,
    json: HighlightConfiguration,
    highlighter: Highlighter,
    cache: BTreeMap<NodeId, CachedCode>,
    cache_bytes: usize,
    render_segments: usize,
}

impl SyntaxHighlighter {
    fn new() -> Option<Self> {
        let mut rust = HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        )
        .ok()?;
        rust.configure(HIGHLIGHT_NAMES);

        let mut json = HighlightConfiguration::new(
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .ok()?;
        json.configure(HIGHLIGHT_NAMES);

        Some(Self {
            rust,
            json,
            highlighter: Highlighter::new(),
            cache: BTreeMap::new(),
            cache_bytes: 0,
            render_segments: 0,
        })
    }

    fn clear(&mut self) {
        self.cache.clear();
        self.cache_bytes = 0;
    }

    fn remove(&mut self, node_id: NodeId) {
        if let Some(cached) = self.cache.remove(&node_id) {
            self.cache_bytes = self.cache_bytes.saturating_sub(cached.bytes);
        }
    }

    fn begin_render(&mut self) {
        self.render_segments = 0;
    }

    fn render(
        &mut self,
        node: &ContentNode,
        language: Option<&str>,
        source: &str,
        fully_visible: bool,
    ) -> Vec<UiLine> {
        let Some(language) = CodeLanguage::from_fence(language) else {
            return plain_code_lines(source);
        };
        if !fully_visible || source.len() > MAX_HIGHLIGHT_BYTES {
            return plain_code_lines(source);
        }
        if let Some(cached) = self.cache.get(&node.id)
            && cached.version == node.version
            && cached.source == source
        {
            self.render_segments = self.render_segments.saturating_add(cached.segments);
            return cached.lines.clone();
        }
        let Some((lines, segments)) = self.highlight(language, source) else {
            return plain_code_lines(source);
        };
        self.render_segments = self.render_segments.saturating_add(segments);
        self.insert_cache(
            node.id,
            CachedCode {
                version: node.version.clone(),
                source: source.to_string(),
                lines: lines.clone(),
                segments,
                bytes: source.len(),
            },
        );
        lines
    }

    fn insert_cache(&mut self, node_id: NodeId, cached: CachedCode) {
        self.remove(node_id);
        self.cache_bytes = self.cache_bytes.saturating_add(cached.bytes);
        self.cache.insert(node_id, cached);
        while self.cache_bytes > MAX_HIGHLIGHT_CACHE_BYTES {
            let Some(node_id) = self.cache.first_key_value().map(|(node_id, _)| *node_id) else {
                break;
            };
            self.remove(node_id);
        }
    }

    fn highlight(&mut self, language: CodeLanguage, source: &str) -> Option<(Vec<UiLine>, usize)> {
        let (highlighter, configuration) = match language {
            CodeLanguage::Rust => (&mut self.highlighter, &self.rust),
            CodeLanguage::Json => (&mut self.highlighter, &self.json),
        };
        let events = highlighter
            .highlight(configuration, source.as_bytes(), None, |_| None)
            .ok()?;
        let mut lines = vec![Vec::new()];
        let mut styles = vec![Style::default().fg(Color::White)];
        let mut segments: usize = 0;

        for event in events {
            match event.ok()? {
                HighlightEvent::HighlightStart(Highlight(index)) => {
                    styles.push(highlight_style(index));
                    segments = segments.saturating_add(1);
                }
                HighlightEvent::HighlightEnd => {
                    if styles.len() > 1 {
                        styles.pop();
                    }
                }
                HighlightEvent::Source { start, end } => {
                    let text = source.get(start..end)?;
                    append_styled_text(
                        &mut lines,
                        text,
                        styles.last().copied().unwrap_or_else(Style::default),
                    );
                }
            }
        }

        Some((lines.into_iter().map(Line::from).collect(), segments))
    }
}

fn highlight_style(index: usize) -> Style {
    let name = HIGHLIGHT_NAMES.get(index).copied().unwrap_or_default();
    match name {
        "comment" => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
        "keyword" | "operator" => Style::default().fg(Color::Magenta),
        "string" => Style::default().fg(Color::Green),
        "number" | "constant" | "constant.builtin" => Style::default().fg(Color::Yellow),
        "type" | "type.builtin" | "constructor" => Style::default().fg(Color::Cyan),
        "function" | "function.builtin" => Style::default().fg(Color::Blue),
        "variable.parameter" => Style::default().fg(Color::LightMagenta),
        "property" | "attribute" => Style::default().fg(Color::LightCyan),
        "punctuation" => Style::default().fg(Color::Gray),
        _ => Style::default().fg(Color::White),
    }
}

fn plain_code_lines(source: &str) -> Vec<UiLine> {
    source
        .split('\n')
        .map(|line| {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Gray),
            ))
        })
        .collect()
}

fn append_styled_text(lines: &mut Vec<Vec<Span<'static>>>, text: &str, style: Style) {
    for fragment in text.split_inclusive('\n') {
        let (content, newline) = match fragment.strip_suffix('\n') {
            Some(content) => (content, true),
            None => (fragment, false),
        };
        if !content.is_empty() {
            lines
                .last_mut()
                .expect("highlight output always has an active line")
                .push(Span::styled(content.to_string(), style));
        }
        if newline {
            lines.push(Vec::new());
        }
    }
}

fn render_blocks(
    document: &Document,
    ids: &[NodeId],
    visible_source_end: usize,
    syntax: Option<&mut SyntaxHighlighter>,
    lines: &mut Vec<UiLine>,
) {
    let mut syntax = syntax;
    for id in ids {
        let Some(node) = document.node(*id) else {
            continue;
        };
        match &node.content {
            ContentKind::Heading { level } => {
                let mut spans = vec![Span::styled(
                    format!("{} ", "#".repeat((*level).into())),
                    heading_style(*level),
                )];
                inline_spans(
                    document,
                    node.children.as_slice(),
                    visible_source_end,
                    heading_style(*level),
                    &mut spans,
                );
                push_nonempty_line(lines, spans);
                lines.push(Line::default());
            }
            ContentKind::Paragraph {} => {
                let mut spans = Vec::new();
                inline_spans(
                    document,
                    node.children.as_slice(),
                    visible_source_end,
                    Style::default().fg(Color::White),
                    &mut spans,
                );
                push_nonempty_line(lines, spans);
                lines.push(Line::default());
            }
            ContentKind::List { ordered, start, .. } => {
                render_list(
                    document,
                    node,
                    *ordered,
                    *start,
                    visible_source_end,
                    syntax.as_deref_mut(),
                    lines,
                );
            }
            ContentKind::BlockQuote { .. } => {
                let mut quoted = Vec::new();
                render_blocks(
                    document,
                    node.children.as_slice(),
                    visible_source_end,
                    syntax.as_deref_mut(),
                    &mut quoted,
                );
                for line in quoted {
                    lines.push(prefix_line(
                        Span::styled("│ ", Style::default().fg(Color::Cyan)),
                        line,
                    ));
                }
                lines.push(Line::default());
            }
            ContentKind::CodeBlock { text, .. } => {
                let language = node.content.code_language().unwrap_or("plain");
                let code = semantic_text(document, node, text, visible_source_end);
                let fully_visible = visible_source_end >= cursor_to_usize(node.body.end);
                lines.push(Line::from(Span::styled(
                    format!(" {language} "),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                let code_lines = match syntax.as_deref_mut() {
                    Some(syntax) => syntax.render(node, Some(language), &code, fully_visible),
                    None => plain_code_lines(&code),
                };
                for code_line in code_lines {
                    lines.push(prefix_line(
                        Span::styled("  ", Style::default().fg(Color::DarkGray)),
                        code_line,
                    ));
                }
                lines.push(Line::default());
            }
            ContentKind::ThematicBreak {} => {
                lines.push(Line::from(Span::styled(
                    "----------------------------------------",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::default());
            }
            ContentKind::Math { text, display } => {
                let value = semantic_text(document, node, text, visible_source_end);
                if !value.is_empty() {
                    let delimiters = if *display { ("$$", "$$") } else { ("$", "$") };
                    lines.push(Line::from(Span::styled(
                        format!("{}{}{}", delimiters.0, value, delimiters.1),
                        Style::default().fg(Color::Magenta),
                    )));
                    lines.push(Line::default());
                }
            }
            ContentKind::CitationDefinition { key, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("[citation definition: {key}]"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            ContentKind::FootnoteDefinition { label, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("[footnote definition: {label}]"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            _ if !node.children.is_empty() => {
                render_blocks(
                    document,
                    node.children.as_slice(),
                    visible_source_end,
                    syntax.as_deref_mut(),
                    lines,
                );
            }
            _ => {}
        }
    }
}

fn render_list(
    document: &Document,
    list: &ContentNode,
    ordered: bool,
    start: Option<u32>,
    visible_source_end: usize,
    syntax: Option<&mut SyntaxHighlighter>,
    lines: &mut Vec<UiLine>,
) {
    let mut syntax = syntax;
    for (index, item_id) in list.children.iter().enumerate() {
        let Some(item) = document.node(*item_id) else {
            continue;
        };
        let mut item_lines = Vec::new();
        if matches!(item.content, ContentKind::ListItem { .. }) {
            render_blocks(
                document,
                item.children.as_slice(),
                visible_source_end,
                syntax.as_deref_mut(),
                &mut item_lines,
            );
        } else {
            render_blocks(
                document,
                std::slice::from_ref(item_id),
                visible_source_end,
                syntax.as_deref_mut(),
                &mut item_lines,
            );
        }

        let marker = list_marker(item, ordered, start, index);
        let prefix = Span::styled(marker, Style::default().fg(Color::Cyan));
        if let Some(first) = item_lines.first_mut() {
            first.spans.insert(0, prefix);
        } else {
            item_lines.push(Line::from(prefix));
        }
        if item_lines.last().is_some_and(|line| line.spans.is_empty()) {
            item_lines.pop();
        }
        lines.append(&mut item_lines);
    }
    lines.push(Line::default());
}

fn list_marker(item: &ContentNode, ordered: bool, start: Option<u32>, index: usize) -> String {
    let task_prefix = match item.content {
        ContentKind::ListItem {
            checked: Some(true),
        } => "[x] ",
        ContentKind::ListItem {
            checked: Some(false),
        } => "[ ] ",
        _ => "",
    };
    if ordered {
        let number = start.unwrap_or(1).saturating_add(index as u32);
        format!("{number}. {task_prefix}")
    } else {
        format!("- {task_prefix}")
    }
}

fn inline_spans(
    document: &Document,
    ids: &[NodeId],
    visible_source_end: usize,
    inherited_style: Style,
    spans: &mut Vec<Span<'static>>,
) {
    for id in ids {
        let Some(node) = document.node(*id) else {
            continue;
        };
        match &node.content {
            ContentKind::Text { text } => {
                push_text_span(
                    spans,
                    semantic_text(document, node, text, visible_source_end),
                    inherited_style,
                );
            }
            ContentKind::Emphasis {} => inline_spans(
                document,
                node.children.as_slice(),
                visible_source_end,
                inherited_style.add_modifier(Modifier::ITALIC),
                spans,
            ),
            ContentKind::Strong {} => inline_spans(
                document,
                node.children.as_slice(),
                visible_source_end,
                inherited_style.add_modifier(Modifier::BOLD),
                spans,
            ),
            ContentKind::Strikethrough {} => inline_spans(
                document,
                node.children.as_slice(),
                visible_source_end,
                inherited_style.add_modifier(Modifier::CROSSED_OUT),
                spans,
            ),
            ContentKind::Link { .. } => inline_spans(
                document,
                node.children.as_slice(),
                visible_source_end,
                inherited_style
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
                spans,
            ),
            ContentKind::InlineCode { text } => push_text_span(
                spans,
                semantic_text(document, node, text, visible_source_end),
                inherited_style.fg(Color::Yellow).bg(Color::DarkGray),
            ),
            ContentKind::Image { alt, .. } => push_text_span(
                spans,
                format!(
                    "[image: {}]",
                    semantic_text(document, node, alt, visible_source_end)
                ),
                inherited_style.fg(Color::Cyan),
            ),
            ContentKind::CitationReference { key, .. } => push_text_span(
                spans,
                format!("[{key}]"),
                inherited_style.fg(Color::Magenta),
            ),
            ContentKind::FootnoteReference { label, .. } => push_text_span(
                spans,
                format!("[^{label}]"),
                inherited_style.fg(Color::Magenta),
            ),
            ContentKind::SoftBreak {} | ContentKind::HardBreak {} => {
                push_text_span(spans, " ".to_string(), inherited_style);
            }
            ContentKind::Html { text, .. } => push_text_span(
                spans,
                semantic_text(document, node, text, visible_source_end),
                inherited_style.fg(Color::DarkGray),
            ),
            ContentKind::Custom {
                namespace, name, ..
            } => {
                push_text_span(
                    spans,
                    format!("[{namespace}/{name}]"),
                    inherited_style.fg(Color::Blue),
                );
                inline_spans(
                    document,
                    node.children.as_slice(),
                    visible_source_end,
                    inherited_style,
                    spans,
                );
            }
            _ if !node.children.is_empty() => inline_spans(
                document,
                node.children.as_slice(),
                visible_source_end,
                inherited_style,
                spans,
            ),
            _ => {}
        }
    }
}

fn semantic_text(
    document: &Document,
    node: &ContentNode,
    text: &SemanticText,
    visible_source_end: usize,
) -> String {
    let body_start = cursor_to_usize(node.body.start);
    let body_end = cursor_to_usize(node.body.end);
    match text {
        SemanticText::Source {} => {
            let visible_end = visible_source_end.min(body_end);
            if visible_end <= body_start {
                String::new()
            } else {
                document
                    .source()
                    .get(body_start..visible_end)
                    .expect("canonical source ranges are UTF-8 boundaries")
                    .to_string()
            }
        }
        SemanticText::Normalized { value } if visible_source_end >= body_end => value.clone(),
        SemanticText::Normalized { .. } => String::new(),
    }
}

fn cursor_to_usize(cursor: mdstream_protocol::SourceCursor) -> usize {
    usize::try_from(cursor.get()).expect("canonical source cursors fit the process address space")
}

fn push_text_span(spans: &mut Vec<Span<'static>>, text: String, style: Style) {
    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }
}

fn push_nonempty_line(lines: &mut Vec<UiLine>, spans: Vec<Span<'static>>) {
    if spans.iter().any(|span| !span.content.is_empty()) {
        lines.push(Line::from(spans));
    }
}

fn prefix_line(prefix: Span<'static>, mut line: UiLine) -> UiLine {
    line.spans.insert(0, prefix);
    line
}

fn heading_style(level: u8) -> Style {
    match level {
        1 => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        2 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
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
    pub(crate) animation_ticks: u64,
    pub(crate) semantic_lines: usize,
    pub(crate) highlighted_segments: usize,
    pub(crate) completed_activities: usize,
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
            "smoke source does not match the rich fixture",
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
    if summary.batches == 0 || summary.changes < summary.batches {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "smoke batch accounting is invalid: batches={} changes={}",
                summary.batches, summary.changes
            ),
        ));
    }
    if summary.animation_ticks == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "smoke animation did not advance",
        ));
    }
    if summary.semantic_lines < 10 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "smoke rendered too few semantic lines={}",
                summary.semantic_lines
            ),
        ));
    }
    if summary.highlighted_segments == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "smoke Tree-sitter did not produce syntax captures",
        ));
    }
    if summary.completed_activities != demo_activities().len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "smoke activities incomplete: expected {} completed {}",
                demo_activities().len(),
                summary.completed_activities
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
                "RICH_SMOKE_OK lifecycle={:?} input_capacity={} commands_sent={} batches={} changes={} errors={} animation_ticks={} semantic_lines={} highlighted_segments={} completed_activities={}",
                summary.lifecycle,
                summary.input_capacity,
                summary.commands_sent,
                summary.batches,
                summary.changes,
                summary.errors,
                summary.animation_ticks,
                summary.semantic_lines,
                summary.highlighted_segments,
                summary.completed_activities,
            );
            return Ok(());
        }
        [] => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: cargo run -p mdstream-tokio --features rich-tui --example agent_tui_rich -- [--smoke]",
            ));
        }
    }

    run_interactive().await
}

async fn run_interactive() -> io::Result<()> {
    let mut stdout = io::stdout();
    enable_raw_mode()?;
    if let Err(error) = crossterm::execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(error);
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            let mut cleanup_stdout = io::stdout();
            let _ = crossterm::execute!(cleanup_stdout, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error);
        }
    };

    let (mut actor, producer) = spawn_demo(Duration::from_millis(5));
    let (events, mut event_rx) = mpsc::channel(EVENT_QUEUE_CAPACITY);
    let event_loop_open = Arc::new(AtomicBool::new(true));
    let event_loop_open_for_thread = Arc::clone(&event_loop_open);
    let quit_requested = Arc::new(AtomicBool::new(false));
    let quit_requested_for_thread = Arc::clone(&quit_requested);
    let event_thread = std::thread::spawn(move || {
        while event_loop_open_for_thread.load(Ordering::Relaxed) {
            if let Ok(true) = crossterm::event::poll(Duration::from_millis(50))
                && let Ok(event) = crossterm::event::read()
            {
                forward_event(&events, &quit_requested_for_thread, event);
            }
        }
    });

    let result = run(
        &mut terminal,
        &mut RichApp::default(),
        &mut actor,
        &mut event_rx,
        &quit_requested,
    )
    .await;
    event_loop_open.store(false, Ordering::Relaxed);
    let _ = event_thread.join();
    actor.begin_cancel();
    let terminal_result = restore_terminal(&mut terminal);
    let actor_result = actor
        .join()
        .await
        .map_err(|error| io::Error::other(format!("actor task failed: {error}")));
    let producer_result = producer
        .await
        .map_err(|error| io::Error::other(format!("demo producer failed: {error}")));

    terminal_result?;
    producer_result?;
    drop(actor_result?);
    result
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    let mut first_error = None;
    remember_cleanup_error(&mut first_error, disable_raw_mode());
    remember_cleanup_error(
        &mut first_error,
        crossterm::execute!(terminal.backend_mut(), LeaveAlternateScreen),
    );
    remember_cleanup_error(&mut first_error, terminal.show_cursor());
    first_error.map_or(Ok(()), Err)
}

fn remember_cleanup_error(first_error: &mut Option<io::Error>, result: io::Result<()>) {
    if first_error.is_none() {
        if let Err(error) = result {
            *first_error = Some(error);
        }
    } else {
        let _ = result;
    }
}

pub(crate) async fn run_smoke() -> io::Result<SmokeSummary> {
    let (mut actor, producer) = spawn_demo(Duration::ZERO);
    let mut app = RichApp::default();
    while let Some(batch) = actor.recv().await {
        app.apply_actor_batch(batch);
    }
    app.actor_open = false;
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
    let source = document.source().to_string();
    let lifecycle = document.lifecycle();

    app.settle_presentation();
    let rendered = app.render_answer();
    let completed_activities = app
        .activities
        .iter()
        .filter(|activity| activity.state == ToolState::Complete)
        .count();

    Ok(SmokeSummary {
        source,
        lifecycle,
        input_capacity: INPUT_CAPACITY,
        commands_sent: producer.commands_sent,
        batches: app.batches,
        changes: app.changes,
        errors: app.errors,
        animation_ticks: app.animation_ticks,
        semantic_lines: rendered.lines.len(),
        highlighted_segments: rendered.highlighted_segments,
        completed_activities,
    })
}

fn spawn_demo(delay: Duration) -> (StreamEngineActor, JoinHandle<ProducerCounters>) {
    let (input, input_rx) = mpsc::channel(INPUT_CAPACITY);
    let actor = spawn_stream_engine_actor(
        StreamEngine::new(),
        input_rx,
        CoalesceOptions::new(Duration::from_millis(60), 16 * 1024, 2048),
    );
    let producer = tokio::spawn(demo_stream(input, delay));
    (actor, producer)
}

async fn run<B>(
    terminal: &mut Terminal<B>,
    app: &mut RichApp,
    actor: &mut StreamEngineActor,
    events: &mut mpsc::Receiver<Event>,
    quit_requested: &AtomicBool,
) -> io::Result<()>
where
    B: ratatui::backend::Backend,
    io::Error: From<B::Error>,
{
    loop {
        if quit_requested.swap(false, Ordering::Relaxed) {
            return Ok(());
        }
        terminal.draw(|frame| {
            let [header, main, footer] = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(2),
                ])
                .areas(frame.area());
            let [activity, answer, inspector] = if frame.area().width >= 100 {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(29),
                        Constraint::Min(40),
                        Constraint::Length(31),
                    ])
                    .areas(main)
            } else if frame.area().width >= 70 {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(23),
                        Constraint::Min(24),
                        Constraint::Length(23),
                    ])
                    .areas(main)
            } else {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(8),
                        Constraint::Min(6),
                        Constraint::Length(13),
                    ])
                    .areas(main)
            };
            frame.render_widget(
                Paragraph::new(Span::styled(
                    " mdstream agent workbench  |  typed content, host-owned motion, optional syntax analysis",
                    Style::default().fg(Color::Black).bg(Color::Cyan),
                )),
                header,
            );

            frame.render_widget(
                Paragraph::new(Text::from(app.activity_lines()))
                    .block(Block::default().title(" Activity ").borders(Borders::ALL))
                    .wrap(Wrap { trim: false }),
                activity,
            );

            let rendered = app.render_answer();
            let answer_inner_width = answer.width.saturating_sub(2);
            let answer_inner_height = answer.height.saturating_sub(2);
            if app.follow_tail {
                app.scroll_y = follow_tail_scroll(
                    answer_visual_line_count(&rendered.lines, answer_inner_width, app.wrap),
                    answer_inner_height,
                );
            }
            let mut answer_widget = Paragraph::new(Text::from(rendered.lines))
                .block(Block::default().title(" Answer ").borders(Borders::ALL))
                .scroll((app.scroll_y, 0));
            if app.wrap {
                answer_widget = answer_widget.wrap(Wrap { trim: false });
            }
            frame.render_widget(answer_widget, answer);

            frame.render_widget(
                Paragraph::new(Text::from(app.inspector_lines(rendered.highlighted_segments)))
                    .block(Block::default().title(" Inspector ").borders(Borders::ALL))
                    .wrap(Wrap { trim: false }),
                inspector,
            );
            frame.render_widget(
                Paragraph::new(status_line(app)).wrap(Wrap { trim: false }),
                footer,
            );
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
            _ = tokio::time::sleep(Duration::from_millis(16)) => app.advance_presentation(),
        }
    }
}

fn forward_event(events: &mpsc::Sender<Event>, quit_requested: &AtomicBool, event: Event) {
    if is_quit_event(&event) {
        quit_requested.store(true, Ordering::Relaxed);
    }
    let _ = events.try_send(event);
}

fn is_quit_event(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(key)
            if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q')
    )
}

fn answer_visual_line_count(lines: &[UiLine], width: u16, wrap: bool) -> usize {
    if wrap {
        Paragraph::new(Text::from(lines.to_vec()))
            .wrap(Wrap { trim: false })
            .line_count(width)
    } else {
        lines.len()
    }
}

fn follow_tail_scroll(line_count: usize, viewport_height: u16) -> u16 {
    u16::try_from(line_count)
        .unwrap_or(u16::MAX)
        .saturating_sub(viewport_height)
}

fn handle_event(app: &mut RichApp, event: Event) -> bool {
    if is_quit_event(&event) {
        return true;
    }
    let Event::Key(key) = event else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }

    match key.code {
        KeyCode::Char('f') => {
            app.follow_tail = !app.follow_tail;
            false
        }
        KeyCode::Char('p') => {
            app.paused = !app.paused;
            false
        }
        KeyCode::Char('m') => {
            app.reduced_motion = !app.reduced_motion;
            if app.reduced_motion {
                app.settle_presentation();
            }
            false
        }
        KeyCode::Char('w') => {
            app.wrap = !app.wrap;
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

fn status_line(app: &RichApp) -> String {
    format!(
        "q quit | j/k scroll | g/G top/bottom | f follow={} | p paused={} | m motion={} | w wrap={} | actor={} | batches={} changes={} errors={}",
        app.follow_tail,
        app.paused,
        motion_label(app.reduced_motion),
        app.wrap,
        actor_label(app.actor_open),
        app.batches,
        app.changes,
        app.errors,
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

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyEvent, KeyModifiers};

    use super::*;

    #[test]
    fn completed_activity_never_regresses_when_the_source_grows() {
        let mut activity = demo_activities().remove(0);

        activity.update(100);
        activity.update(0);

        assert_eq!(activity.state, ToolState::Complete);
    }

    #[test]
    fn presentation_holds_a_trailing_cluster_until_input_closes() {
        let accented = "e\u{301}";
        let source = format!("{accented}x");
        let zwj = "\u{1F469}\u{200D}\u{1F4BB}";
        let zwj_source = format!("{zwj}x");

        assert_eq!(presentation_limit("e", true), 0);
        assert_eq!(presentation_limit(accented, true), 0);
        assert_eq!(presentation_limit(&source, true), accented.len());
        assert_eq!(presentation_limit(accented, false), accented.len());
        assert_eq!(presentation_limit(zwj, true), 0);
        assert_eq!(presentation_limit(&zwj_source, true), zwj.len());
    }

    #[test]
    fn wrapped_follow_tail_uses_ratatuis_visual_line_count() {
        let lines = vec![Line::from("one two three four five")];
        let visual_lines = answer_visual_line_count(&lines, 5, true);

        assert!(visual_lines > lines.len());
        assert_eq!(
            follow_tail_scroll(visual_lines, 1),
            u16::try_from(visual_lines.saturating_sub(1)).unwrap()
        );
    }

    #[test]
    fn quit_signal_survives_a_full_nonblocking_event_queue() {
        let (events, _receiver) = mpsc::channel(1);
        events.try_send(Event::Resize(1, 1)).unwrap();
        let quit_requested = AtomicBool::new(false);

        forward_event(
            &events,
            &quit_requested,
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
        );

        assert!(quit_requested.load(Ordering::Relaxed));
    }

    #[test]
    fn removed_code_node_releases_its_cached_highlight() {
        let mut syntax = SyntaxHighlighter::new().expect("fixture grammars compile");
        let node_id = NodeId::from(7_u64);
        let source = "fn main() {}".to_string();
        let bytes = source.len();

        syntax.insert_cache(
            node_id,
            CachedCode {
                version: NodeVersion::digest(b"cached-code"),
                source,
                lines: vec![Line::from("fn main() {}")],
                segments: 1,
                bytes,
            },
        );
        syntax.remove(node_id);

        assert!(syntax.cache.is_empty());
        assert_eq!(syntax.cache_bytes, 0);
    }
}
