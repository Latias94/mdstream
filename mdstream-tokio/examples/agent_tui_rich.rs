#[path = "agent_tui_rich/presentation.rs"]
mod presentation;
#[path = "agent_tui_rich/render.rs"]
mod render;

use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{
    DocumentLifecycle, TransitionChildListOwner, TransitionFacts, TransitionReducer,
};
use mdstream_tokio::{
    ActorBatch, ActorCommand, ActorExit, ActorJoinOutcome, CoalesceOptions, StreamEngineActor,
    spawn_stream_engine_actor,
};
use presentation::{LineStage, PresentationState, RootKey, TickResult};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::CellWidth;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, StyledGrapheme, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use render::{ProjectionCache, SyntaxHighlighter, flatten_projections, project_document};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

pub(crate) const INPUT_CAPACITY: usize = 8;
const EVENT_QUEUE_CAPACITY: usize = 64;
const PRESENTATION_TICK: Duration = Duration::from_millis(32);

pub(crate) const DEMO_MARKDOWN: &str = r#"# mdstream agent workbench

I am streaming a typed answer through a bounded Tokio actor. The host renders
mdstream Content IR directly, so timing and terminal presentation do not become
another Markdown parser.

> Canonical content belongs to mdstream. This workbench owns only layout,
> animation, scrolling, highlighting, and the activity timeline.

## Plan

- [x] Read the actor contract.
- [x] Render stable semantic blocks.
- [x] Reconcile a committed-line frontier.

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
  "animation": "line-frontier",
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
            detail: "canonical convergence",
            start_at_percent: 64,
            complete_at_percent: 100,
            state: ToolState::Queued,
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActorState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LayoutKey {
    answer_revision: u64,
    width: u16,
    wrap: bool,
}

#[derive(Debug, Default)]
struct RenderMetrics {
    layout_builds: u64,
    viewport_rows_materialized: u64,
}

#[derive(Debug, Default)]
struct RuntimeMetrics {
    draws: u64,
    presentation_tick_wakeups: u64,
}

impl ActorState {
    const fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "complete",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

struct RichApp {
    reducer: TransitionReducer,
    presentation: PresentationState,
    actor_state: ActorState,
    scroll: ScrollState,
    last_layout: VisualLayout,
    layout_key: Option<LayoutKey>,
    answer_revision: u64,
    wrap: bool,
    batches: u64,
    changes: u64,
    errors: u64,
    last_error: Option<String>,
    activities: Vec<ToolActivity>,
    projection_cache: ProjectionCache,
    syntax: Option<SyntaxHighlighter>,
    last_tick_lines: usize,
    last_tick_catch_up: bool,
    render_metrics: RenderMetrics,
    runtime_metrics: RuntimeMetrics,
}

impl Default for RichApp {
    fn default() -> Self {
        Self::new(false)
    }
}

impl RichApp {
    fn new(reduced_motion: bool) -> Self {
        let mut presentation = PresentationState::new();
        let _ = presentation.set_reduced_motion(reduced_motion);
        Self {
            reducer: TransitionReducer::new(),
            presentation,
            actor_state: ActorState::Running,
            scroll: ScrollState::default(),
            last_layout: VisualLayout::default(),
            layout_key: None,
            answer_revision: 0,
            wrap: true,
            batches: 0,
            changes: 0,
            errors: 0,
            last_error: None,
            activities: demo_activities(),
            projection_cache: ProjectionCache::default(),
            syntax: SyntaxHighlighter::new(),
            last_tick_lines: 0,
            last_tick_catch_up: false,
            render_metrics: RenderMetrics::default(),
            runtime_metrics: RuntimeMetrics::default(),
        }
    }

    fn apply_actor_batch(&mut self, batch: ActorBatch, now: Instant) -> bool {
        self.apply_engine_outputs(batch.into_transitions(), now)
    }

    fn apply_engine_outputs(&mut self, outputs: Vec<EngineOutput>, now: Instant) -> bool {
        if outputs.is_empty() {
            return false;
        }
        self.batches = self.batches.saturating_add(1);
        self.changes = self.changes.saturating_add(
            outputs
                .iter()
                .map(|output| output.changes().len() as u64)
                .sum::<u64>(),
        );

        let mut had_success = false;
        let mut full_replace = false;
        'outputs: for output in outputs {
            for change in output.into_changes() {
                match self.reducer.apply(change) {
                    Ok(outcome) => {
                        if let Some(facts) = outcome.facts.as_ref() {
                            had_success = true;
                            full_replace |= matches!(facts, TransitionFacts::FullReplace { .. });
                            self.observe_facts(facts);
                        }
                    }
                    Err(error) => {
                        self.record_error(error.to_string());
                        break 'outputs;
                    }
                }
            }
        }

        if full_replace {
            self.reset_host_continuity();
        }
        let reconciled = had_success && self.reconcile_presentation(now, full_replace);
        if had_success {
            self.mark_answer_changed();
        }
        self.update_activities();
        reconciled
    }

    fn observe_facts(&mut self, facts: &TransitionFacts) {
        self.projection_cache.observe_facts(
            facts,
            self.reducer.document(),
            self.reducer.continuity_generation(),
        );
        let Some(syntax) = self.syntax.as_mut() else {
            return;
        };
        match facts {
            TransitionFacts::Continuous {
                nodes,
                structures,
                resources,
                ..
            } => {
                for node in nodes {
                    syntax.invalidate_key(node.key);
                }
                for structure in structures {
                    if let TransitionChildListOwner::Node { key } = structure.owner {
                        syntax.invalidate_key(key);
                    }
                    for key in structure.removed.iter().chain(&structure.inserted) {
                        syntax.invalidate_key(*key);
                    }
                }
                for resource in resources {
                    for key in &resource.affected_nodes {
                        syntax.invalidate_key(*key);
                    }
                }
            }
            TransitionFacts::FullReplace { .. } => syntax.clear(),
        }
    }

    fn reconcile_presentation(&mut self, now: Instant, full_replace: bool) -> bool {
        let Some(document) = self.reducer.document() else {
            return false;
        };
        let projections = project_document(
            document,
            self.reducer.continuity_generation(),
            &mut self.projection_cache,
            self.syntax.as_mut(),
        );
        self.presentation
            .reconcile(projections.stable, projections.mutable, now, full_replace)
    }

    fn reset_host_continuity(&mut self) {
        self.activities = demo_activities();
        self.last_tick_lines = 0;
        self.last_tick_catch_up = false;
        self.scroll.clear_content_identity();
        self.last_layout = VisualLayout::default();
        self.layout_key = None;
    }

    fn mark_answer_changed(&mut self) {
        self.answer_revision = self.answer_revision.saturating_add(1);
        self.layout_key = None;
    }

    fn apply_tick(&mut self, now: Instant) -> bool {
        let result = self.presentation.tick(now);
        self.apply_tick_result(result)
    }

    fn apply_tick_result(&mut self, result: TickResult) -> bool {
        if result.changed {
            self.last_tick_lines = result.committed_lines;
            self.last_tick_catch_up = result.catch_up;
            self.update_activities();
        }
        result.changed
    }

    fn toggle_paused(&mut self) -> bool {
        let result = self.presentation.set_paused(!self.presentation.is_paused());
        self.apply_tick_result(result)
    }

    fn toggle_reduced_motion(&mut self) -> bool {
        let result = self
            .presentation
            .set_reduced_motion(!self.presentation.is_reduced_motion());
        self.apply_tick_result(result)
    }

    fn update_activities(&mut self) {
        let total_lines = self.presentation.line_count();
        let committed_lines = self.presentation.committed_line_count();
        let progress = if self.is_settled() {
            100
        } else if total_lines == 0 {
            0
        } else {
            committed_lines.saturating_mul(100) / total_lines
        };
        for activity in &mut self.activities {
            activity.update(progress);
        }
    }

    fn is_settled(&self) -> bool {
        self.actor_state == ActorState::Completed
            && self.errors == 0
            && self
                .reducer
                .document()
                .is_some_and(|document| document.lifecycle() == DocumentLifecycle::Finalized)
            && self.pending_bytes() == 0
            && self.presentation.is_idle()
            && self.presentation.mutable_root_count() == 0
    }

    fn pending_bytes(&self) -> usize {
        self.reducer
            .document()
            .map_or(0, |document| document.pending_source().len())
    }

    fn highlighted_segments(&self) -> usize {
        self.syntax
            .as_ref()
            .map_or(0, SyntaxHighlighter::render_segments)
    }

    fn render_answer(&self) -> RenderedAnswer {
        let presentation_lines = self.presentation.lines();
        let mut rows = Vec::with_capacity(presentation_lines.len().saturating_add(2));
        for presented in presentation_lines {
            let anchor = LineAnchor {
                owner: presented.owner,
                row: presented.row,
            };
            rows.push(RenderedLine {
                line: presented.line,
                anchor: Some(anchor),
            });
        }

        let pending_bytes = self.pending_bytes();
        if pending_bytes != 0 {
            rows.push(RenderedLine::unanchored(Line::from(Span::styled(
                format!("streaming | pending {pending_bytes} source bytes"),
                Style::default().fg(Color::Yellow),
            ))));
        } else if self.actor_state == ActorState::Running && rows.is_empty() {
            rows.push(RenderedLine::unanchored(Line::from(Span::styled(
                "Waiting for projected Content IR...",
                Style::default().fg(Color::DarkGray),
            ))));
        } else if self.actor_state == ActorState::Completed && rows.is_empty() {
            rows.push(RenderedLine::unanchored(Line::from(Span::styled(
                "Completed with no content.",
                Style::default().fg(Color::DarkGray),
            ))));
        }

        if let Some(error) = &self.last_error {
            rows.push(RenderedLine::unanchored(Line::from(Span::styled(
                format!("stream error | {error}"),
                Style::default().fg(Color::Red),
            ))));
        }

        RenderedAnswer { rows }
    }

    fn activity_lines(&self) -> Vec<UiLine> {
        let mut lines = vec![
            Line::from(Span::styled(
                "host side-channel activity",
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
        lines
    }

    fn inspector_lines(&self) -> Vec<UiLine> {
        let (lifecycle, epoch, sequence, nodes, source_len, projection_cursor) =
            self.reducer.document().map_or_else(
                || (DocumentLifecycle::Open, 0, 0, 0, 0, 0),
                |document| {
                    (
                        document.lifecycle(),
                        document.coordinate().epoch.get(),
                        document.coordinate().sequence.get(),
                        document.nodes().len(),
                        document.source().len(),
                        document.projection_cursor().get(),
                    )
                },
            );
        let metrics = self.presentation.metrics();
        let projection_metrics = self.projection_cache.metrics();
        vec![
            Line::from(format!("lifecycle   {lifecycle:?}")),
            Line::from(format!("epoch/seq   {epoch}/{sequence}")),
            Line::from(format!("nodes       {nodes}")),
            Line::from(format!("source      {source_len} bytes")),
            Line::from(format!("projection  {projection_cursor} bytes")),
            Line::from(format!("pending     {} bytes", self.pending_bytes())),
            Line::default(),
            Line::from(Span::styled(
                "presentation",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!(
                "lines       {} committed",
                self.presentation.committed_line_count()
            )),
            Line::from(format!(
                "roots       {}/{} mutable",
                self.presentation.presented_root_count(),
                self.presentation.mutable_root_count()
            )),
            Line::from(format!(
                "queue       {} lines",
                self.presentation.queue_len(),
            )),
            Line::from(format!(
                "mode        {}{}",
                motion_label(self.presentation.is_reduced_motion()),
                if self.presentation.is_paused() {
                    " / paused"
                } else {
                    ""
                }
            )),
            Line::from(format!(
                "last tick   {} lines{}",
                self.last_tick_lines,
                if self.last_tick_catch_up {
                    " / catch-up"
                } else {
                    ""
                }
            )),
            Line::from(format!("reconciles  {}", metrics.reconciliations)),
            Line::from(format!("corrections {}", metrics.corrections)),
            Line::from(format!(
                "root cache  {}/{} render/reuse",
                projection_metrics.stable_roots_rendered, projection_metrics.stable_roots_reused
            )),
            Line::default(),
            Line::from(Span::styled(
                "host adapters",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!(
                "Tree-sitter {} captures",
                self.highlighted_segments()
            )),
            Line::from(format!("actor       {}", self.actor_state.label())),
            Line::from(format!("batches     {}", self.batches)),
            Line::from(format!("changes     {}", self.changes)),
            Line::from(format!("errors      {}", self.errors)),
        ]
    }

    fn record_error(&mut self, error: String) {
        self.errors = self.errors.saturating_add(1);
        self.last_error = Some(error);
        self.mark_answer_changed();
    }

    fn apply_join_outcome(&mut self, outcome: ActorJoinOutcome, now: Instant) {
        for batch in outcome.unread {
            self.apply_actor_batch(batch, now);
        }
        match outcome.exit {
            ActorExit::Completed(_) => {
                self.actor_state = if self.errors == 0 {
                    ActorState::Completed
                } else {
                    ActorState::Failed
                };
            }
            ActorExit::Failed(failure) => {
                let error = failure.error.to_string();
                self.apply_engine_outputs(failure.completed, now);
                self.record_error(error);
                self.actor_state = ActorState::Failed;
            }
            ActorExit::Cancelled(cancellation) => {
                if let Some(unpublished) = cancellation.unpublished {
                    self.apply_actor_batch(unpublished, now);
                }
                self.actor_state = ActorState::Cancelled;
            }
        }
        self.mark_answer_changed();
        self.update_activities();
    }
}

fn motion_label(reduced_motion: bool) -> &'static str {
    if reduced_motion { "reduced" } else { "paced" }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct LineAnchor {
    owner: RootKey,
    row: usize,
}

struct RenderedLine {
    line: UiLine,
    anchor: Option<LineAnchor>,
}

impl RenderedLine {
    fn unanchored(line: UiLine) -> Self {
        Self { line, anchor: None }
    }
}

struct RenderedAnswer {
    rows: Vec<RenderedLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrollAnchor {
    Content {
        line: LineAnchor,
        wrapped_row: usize,
    },
    Trailing {
        row: usize,
    },
}

#[derive(Debug)]
struct VisualLine {
    anchor: LineAnchor,
    start: usize,
    height: usize,
}

#[derive(Debug)]
struct VisualRow {
    line: UiLine,
    anchor: Option<LineAnchor>,
    wrapped_row: usize,
}

#[derive(Debug, Default)]
struct VisualLayout {
    rows: Vec<VisualRow>,
    lines: Vec<VisualLine>,
    line_indexes: HashMap<LineAnchor, usize>,
    owner_line_ranges: HashMap<RootKey, std::ops::Range<usize>>,
    owner_order: Arc<[RootKey]>,
    trailing_start: usize,
    total_rows: usize,
}

impl VisualLayout {
    fn from_answer(answer: &RenderedAnswer, width: u16, wrap: bool) -> Self {
        let mut rows = Vec::new();
        let mut lines = Vec::with_capacity(answer.rows.len());
        let mut line_indexes = HashMap::new();
        let mut owner_line_ranges = HashMap::<RootKey, std::ops::Range<usize>>::new();
        let mut owner_order = Vec::new();
        let mut trailing_start = None;
        let mut start = 0;
        for rendered in &answer.rows {
            let wrapped = if wrap {
                wrap_ui_line(&rendered.line, width)
            } else {
                vec![rendered.line.clone()]
            };
            let height = wrapped.len();
            if let Some(anchor) = rendered.anchor {
                let line_index = lines.len();
                line_indexes.insert(anchor, line_index);
                lines.push(VisualLine {
                    anchor,
                    start,
                    height,
                });
                owner_line_ranges
                    .entry(anchor.owner)
                    .and_modify(|range| range.end = line_index.saturating_add(1))
                    .or_insert(line_index..line_index.saturating_add(1));
                if owner_order.last() != Some(&anchor.owner) {
                    owner_order.push(anchor.owner);
                }
            } else {
                trailing_start.get_or_insert(start);
            }
            rows.extend(
                wrapped
                    .into_iter()
                    .enumerate()
                    .map(|(wrapped_row, line)| VisualRow {
                        line,
                        anchor: rendered.anchor,
                        wrapped_row,
                    }),
            );
            start = start.saturating_add(height);
        }
        Self {
            rows,
            lines,
            line_indexes,
            owner_line_ranges,
            owner_order: owner_order.into(),
            trailing_start: trailing_start.unwrap_or(start),
            total_rows: start,
        }
    }

    fn row_for(&self, anchor: ScrollAnchor) -> Option<usize> {
        match anchor {
            ScrollAnchor::Content { line, wrapped_row } => {
                self.line_indexes.get(&line).map(|index| {
                    let visual = &self.lines[*index];
                    visual
                        .start
                        .saturating_add(wrapped_row.min(visual.height.saturating_sub(1)))
                })
            }
            ScrollAnchor::Trailing { row } if self.trailing_start < self.total_rows => Some(
                self.trailing_start
                    .saturating_add(row)
                    .min(self.total_rows.saturating_sub(1)),
            ),
            ScrollAnchor::Trailing { .. } => None,
        }
    }

    fn anchor_at(&self, visual_row: usize) -> Option<ScrollAnchor> {
        let row = self.rows.get(visual_row)?;
        row.anchor.map_or_else(
            || {
                Some(ScrollAnchor::Trailing {
                    row: visual_row.saturating_sub(self.trailing_start),
                })
            },
            |line| {
                Some(ScrollAnchor::Content {
                    line,
                    wrapped_row: row.wrapped_row,
                })
            },
        )
    }

    fn nearest_row_for_owner(&self, owner: RootKey, preferred_row: usize) -> Option<usize> {
        let range = self.owner_line_ranges.get(&owner)?.clone();
        let lines = self.lines.get(range)?;
        let index = match lines.binary_search_by_key(&preferred_row, |line| line.anchor.row) {
            Ok(index) => index,
            Err(0) => 0,
            Err(index) if index == lines.len() => index.saturating_sub(1),
            Err(index) => {
                let before = index.saturating_sub(1);
                if lines[before].anchor.row.abs_diff(preferred_row)
                    <= lines[index].anchor.row.abs_diff(preferred_row)
                {
                    before
                } else {
                    index
                }
            }
        };
        Some(lines[index].start)
    }

    fn has_owner(&self, owner: RootKey) -> bool {
        self.owner_line_ranges.contains_key(&owner)
    }

    fn visible_range(&self, scroll_y: usize, viewport_height: u16) -> std::ops::Range<usize> {
        let start = scroll_y.min(self.rows.len());
        let end = start
            .saturating_add(usize::from(viewport_height.max(1)))
            .min(self.rows.len());
        start..end
    }
}

#[derive(Debug, Clone)]
struct ScrollState {
    follow_tail: bool,
    scroll_y: usize,
    anchor: Option<ScrollAnchor>,
    previous_owner_order: Arc<[RootKey]>,
    viewport_height: usize,
}

impl Default for ScrollState {
    fn default() -> Self {
        Self {
            follow_tail: true,
            scroll_y: 0,
            anchor: None,
            previous_owner_order: Arc::default(),
            viewport_height: 1,
        }
    }
}

impl ScrollState {
    fn resolve(&mut self, layout: &VisualLayout, viewport_height: u16) -> usize {
        self.viewport_height = usize::from(viewport_height.max(1));
        let max_scroll = layout.total_rows.saturating_sub(self.viewport_height);
        let target = if self.follow_tail {
            max_scroll
        } else if let Some(anchor) = self.anchor {
            self.resolve_anchor(layout, anchor)
                .unwrap_or(self.scroll_y)
                .min(max_scroll)
        } else {
            self.scroll_y.min(max_scroll)
        };

        self.scroll_y = target;
        self.anchor = layout.anchor_at(target);
        Arc::clone_from(&mut self.previous_owner_order, &layout.owner_order);
        target
    }

    fn resolve_anchor(&self, layout: &VisualLayout, anchor: ScrollAnchor) -> Option<usize> {
        layout.row_for(anchor).or_else(|| {
            let ScrollAnchor::Content { line, .. } = anchor else {
                return None;
            };
            layout
                .nearest_row_for_owner(line.owner, line.row)
                .or_else(|| {
                    self.fallback_owner(layout, line.owner)
                        .and_then(|owner| layout.nearest_row_for_owner(owner, line.row))
                })
        })
    }

    fn clear_content_identity(&mut self) {
        self.scroll_y = 0;
        self.anchor = None;
        self.previous_owner_order = Arc::default();
    }

    fn fallback_owner(&self, layout: &VisualLayout, removed: RootKey) -> Option<RootKey> {
        let index = self
            .previous_owner_order
            .iter()
            .position(|owner| *owner == removed)?;
        self.previous_owner_order[..index]
            .iter()
            .rev()
            .copied()
            .find(|owner| layout.has_owner(*owner))
            .or_else(|| {
                self.previous_owner_order[index.saturating_add(1)..]
                    .iter()
                    .copied()
                    .find(|owner| layout.has_owner(*owner))
            })
            .or_else(|| layout.owner_order.first().copied())
    }

    fn scroll_by(&mut self, layout: &VisualLayout, delta: isize) {
        self.follow_tail = false;
        let max_scroll = layout.total_rows.saturating_sub(self.viewport_height);
        self.scroll_y = if delta.is_negative() {
            self.scroll_y.saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll_y.saturating_add(delta as usize).min(max_scroll)
        };
        self.anchor = layout.anchor_at(self.scroll_y);
    }

    fn top(&mut self, layout: &VisualLayout) {
        self.follow_tail = false;
        self.scroll_y = 0;
        self.anchor = layout.anchor_at(0);
    }

    fn toggle_follow(&mut self, layout: &VisualLayout) {
        self.follow_tail = !self.follow_tail;
        if !self.follow_tail {
            self.anchor = layout.anchor_at(self.scroll_y);
        }
    }
}

fn wrap_ui_line(line: &UiLine, width: u16) -> Vec<UiLine> {
    let width = width.max(1);
    let mut wrapped = Vec::new();
    let mut pending_line = Vec::<StyledGrapheme<'_>>::new();
    let mut pending_word = Vec::<StyledGrapheme<'_>>::new();
    let mut pending_whitespace = VecDeque::<StyledGrapheme<'_>>::new();
    let mut line_width = 0_u16;
    let mut word_width = 0_u16;
    let mut whitespace_width = 0_u16;
    let mut previous_was_non_whitespace = false;

    // Match Ratatui's WordWrapper with `trim: false` while retaining owned spans.
    for grapheme in line.styled_graphemes(Style::default()) {
        let is_whitespace = grapheme.is_whitespace();
        let symbol_width = grapheme.symbol.cell_width();
        if symbol_width > width {
            continue;
        }

        let word_found = previous_was_non_whitespace && is_whitespace;
        let segment_overflow = pending_line.is_empty()
            && word_width
                .saturating_add(whitespace_width)
                .saturating_add(symbol_width)
                > width;
        if word_found || segment_overflow {
            pending_line.extend(pending_whitespace.drain(..));
            line_width = line_width.saturating_add(whitespace_width);
            pending_line.append(&mut pending_word);
            line_width = line_width.saturating_add(word_width);
            whitespace_width = 0;
            word_width = 0;
        }

        let line_full = line_width >= width;
        let pending_word_overflow = symbol_width > 0
            && line_width
                .saturating_add(whitespace_width)
                .saturating_add(word_width)
                >= width;
        if line_full || pending_word_overflow {
            let mut remaining_width = width.saturating_sub(line_width);
            wrapped.push(std::mem::take(&mut pending_line));
            line_width = 0;

            while let Some(grapheme) = pending_whitespace.front() {
                let grapheme_width = grapheme.symbol.cell_width();
                if grapheme_width > remaining_width {
                    break;
                }
                whitespace_width = whitespace_width.saturating_sub(grapheme_width);
                remaining_width = remaining_width.saturating_sub(grapheme_width);
                pending_whitespace.pop_front();
            }
            if is_whitespace && pending_whitespace.is_empty() {
                continue;
            }
        }

        if is_whitespace {
            whitespace_width = whitespace_width.saturating_add(symbol_width);
            pending_whitespace.push_back(grapheme);
        } else {
            word_width = word_width.saturating_add(symbol_width);
            pending_word.push(grapheme);
        }
        previous_was_non_whitespace = !is_whitespace;
    }

    pending_line.extend(pending_whitespace);
    pending_line.append(&mut pending_word);
    if !pending_line.is_empty() {
        wrapped.push(pending_line);
    }
    if wrapped.is_empty() {
        wrapped.push(Vec::new());
    }

    wrapped
        .into_iter()
        .map(|graphemes| line_from_graphemes(graphemes, line.alignment))
        .collect()
}

fn line_from_graphemes(
    graphemes: Vec<StyledGrapheme<'_>>,
    alignment: Option<ratatui::layout::Alignment>,
) -> UiLine {
    let mut spans = Vec::<Span<'static>>::new();
    for grapheme in graphemes {
        if let Some(last) = spans.last_mut()
            && last.style == grapheme.style
        {
            last.content.to_mut().push_str(grapheme.symbol);
        } else {
            spans.push(Span::styled(grapheme.symbol.to_owned(), grapheme.style));
        }
    }
    Line {
        spans,
        alignment,
        ..Line::default()
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
    pub(crate) reduced_motion: bool,
    pub(crate) reconciliations: u64,
    pub(crate) enqueued_lines: u64,
    pub(crate) committed_lines: u64,
    pub(crate) queued_lines: usize,
    pub(crate) mutable_roots: usize,
    pub(crate) catch_up_entries: u64,
    pub(crate) max_queue_depth: usize,
    pub(crate) stable_roots_rendered: u64,
    pub(crate) stable_roots_reused: u64,
    pub(crate) canonical_render_equal: bool,
    pub(crate) idle_without_tick: bool,
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
    if summary.reconciliations == 0 || summary.enqueued_lines == 0 || summary.committed_lines == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "smoke presentation did not reconcile and commit canonical lines",
        ));
    }
    if summary.queued_lines != 0 || summary.mutable_roots != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "smoke presentation did not drain: queued={} mutable={}",
                summary.queued_lines, summary.mutable_roots
            ),
        ));
    }
    if !summary.reduced_motion && summary.catch_up_entries == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "paced smoke did not exercise catch-up",
        ));
    }
    if summary.max_queue_depth == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "smoke never observed a queued stable line",
        ));
    }
    if summary.stable_roots_rendered == 0 || summary.stable_roots_reused == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "smoke did not exercise stable-root projection reuse",
        ));
    }
    if !summary.canonical_render_equal {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settled presentation differs from a direct canonical render",
        ));
    }
    if !summary.idle_without_tick {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settled presentation still requests animation ticks",
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
    let (smoke, reduced_motion) = parse_args(std::env::args().skip(1))?;
    if smoke {
        let summary = if reduced_motion {
            run_smoke_with_motion(true).await?
        } else {
            run_smoke().await?
        };
        validate_smoke_summary(&summary)?;
        println!(
            "RICH_SMOKE_OK lifecycle={:?} input_capacity={} commands_sent={} batches={} changes={} errors={} motion={} reconciliations={} enqueued_lines={} committed_lines={} queued_lines={} mutable_roots={} catch_up_entries={} max_queue_depth={} stable_roots_rendered={} stable_roots_reused={} canonical_render_equal={} idle_without_tick={} semantic_lines={} highlighted_segments={} completed_activities={}",
            summary.lifecycle,
            summary.input_capacity,
            summary.commands_sent,
            summary.batches,
            summary.changes,
            summary.errors,
            motion_label(summary.reduced_motion),
            summary.reconciliations,
            summary.enqueued_lines,
            summary.committed_lines,
            summary.queued_lines,
            summary.mutable_roots,
            summary.catch_up_entries,
            summary.max_queue_depth,
            summary.stable_roots_rendered,
            summary.stable_roots_reused,
            summary.canonical_render_equal,
            summary.idle_without_tick,
            summary.semantic_lines,
            summary.highlighted_segments,
            summary.completed_activities,
        );
        return Ok(());
    }

    run_interactive(reduced_motion).await
}

fn parse_args(args: impl IntoIterator<Item = String>) -> io::Result<(bool, bool)> {
    let mut smoke = false;
    let mut reduced_motion = false;
    for arg in args {
        match arg.as_str() {
            "--smoke" if !smoke => smoke = true,
            "--reduced-motion" if !reduced_motion => reduced_motion = true,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "usage: cargo run -p mdstream-tokio --features rich-tui --example agent_tui_rich -- [--smoke] [--reduced-motion]",
                ));
            }
        }
    }
    Ok((smoke, reduced_motion))
}

async fn run_interactive(reduced_motion: bool) -> io::Result<()> {
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

    let mut app = RichApp::new(reduced_motion);
    let result = run(
        &mut terminal,
        &mut app,
        &mut actor,
        &mut event_rx,
        &quit_requested,
    )
    .await;
    event_loop_open.store(false, Ordering::Relaxed);
    let _ = event_thread.join();
    let terminal_result = restore_terminal(&mut terminal);
    settle_actor_after_run(&mut app, &mut actor, Instant::now()).await;
    let producer_result = producer
        .await
        .map_err(|error| io::Error::other(format!("demo producer failed: {error}")));

    terminal_result?;
    producer_result?;
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
    run_smoke_with_motion(false).await
}

pub(crate) async fn run_smoke_with_motion(reduced_motion: bool) -> io::Result<SmokeSummary> {
    let (mut actor, producer) = spawn_demo(Duration::ZERO);
    let started = Instant::now();
    let mut app = RichApp::new(reduced_motion);
    while let Some(batch) = actor.recv().await {
        app.apply_actor_batch(batch, started);
    }
    finish_actor(&mut app, &mut actor, started).await;

    let producer = producer
        .await
        .map_err(|error| io::Error::other(format!("demo producer failed: {error}")))?;
    let mut tick_at = started + Duration::from_millis(121);
    while app.presentation.needs_tick() {
        app.apply_tick(tick_at);
        tick_at += PRESENTATION_TICK;
    }
    if !app.presentation.is_idle() {
        let drained = app.presentation.drain_all();
        app.apply_tick_result(drained);
    }

    let document = app
        .reducer
        .document()
        .ok_or_else(|| io::Error::other("actor produced no canonical document"))?;
    let source = document.source().to_string();
    let lifecycle = document.lifecycle();
    let presented_lines = app
        .presentation
        .lines()
        .into_iter()
        .map(|line| line.line)
        .collect::<Vec<_>>();
    let mut direct_syntax = SyntaxHighlighter::new();
    let mut direct_cache = ProjectionCache::default();
    let direct = project_document(
        document,
        app.reducer.continuity_generation(),
        &mut direct_cache,
        direct_syntax.as_mut(),
    );
    let direct_lines = flatten_projections(&direct);
    let highlighted_segments = direct_syntax
        .as_ref()
        .map_or(0, SyntaxHighlighter::render_segments);
    let completed_activities = app
        .activities
        .iter()
        .filter(|activity| activity.state == ToolState::Complete)
        .count();
    let metrics = app.presentation.metrics();
    let projection_metrics = app.projection_cache.metrics();

    Ok(SmokeSummary {
        source,
        lifecycle,
        input_capacity: INPUT_CAPACITY,
        commands_sent: producer.commands_sent,
        batches: app.batches,
        changes: app.changes,
        errors: app.errors,
        reduced_motion,
        reconciliations: metrics.reconciliations,
        enqueued_lines: metrics.enqueued_lines,
        committed_lines: metrics.committed_lines,
        queued_lines: app.presentation.queue_len(),
        mutable_roots: app.presentation.mutable_root_count(),
        catch_up_entries: metrics.catch_up_entries,
        max_queue_depth: metrics.max_queue_depth,
        stable_roots_rendered: projection_metrics.stable_roots_rendered,
        stable_roots_reused: projection_metrics.stable_roots_reused,
        canonical_render_equal: presented_lines == direct_lines,
        idle_without_tick: !app.presentation.needs_tick(),
        semantic_lines: direct_lines.len(),
        highlighted_segments,
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

async fn finish_actor(app: &mut RichApp, actor: &mut StreamEngineActor, now: Instant) {
    match actor.join().await {
        Ok(outcome) => app.apply_join_outcome(outcome, now),
        Err(error) => {
            for batch in error.unread().iter().cloned() {
                app.apply_actor_batch(batch, now);
            }
            app.record_error(error.to_string());
            app.actor_state = ActorState::Failed;
        }
    }
}

async fn settle_actor_after_run(app: &mut RichApp, actor: &mut StreamEngineActor, now: Instant) {
    if app.actor_state != ActorState::Running {
        return;
    }

    actor.begin_cancel();
    finish_actor(app, actor, now).await;
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
    B::Error: std::fmt::Display,
{
    let mut dirty = true;
    let first_tick = tokio::time::Instant::now() + PRESENTATION_TICK;
    let mut ticker = tokio::time::interval_at(first_tick, PRESENTATION_TICK);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        if quit_requested.swap(false, Ordering::Relaxed) {
            return Ok(());
        }
        if dirty {
            terminal
                .draw(|frame| draw_ui(frame, app))
                .map_err(|error| io::Error::other(error.to_string()))?;
            app.runtime_metrics.draws = app.runtime_metrics.draws.saturating_add(1);
            dirty = false;
        }

        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { return Ok(()); };
                match handle_event(app, event) {
                    UiAction::Quit => return Ok(()),
                    UiAction::Redraw => dirty = true,
                    UiAction::None => {}
                }
            }
            batch = actor.recv(), if app.actor_state == ActorState::Running => {
                match batch {
                    Some(batch) => {
                        app.apply_actor_batch(batch, Instant::now());
                        dirty = true;
                    }
                    None => {
                        finish_actor(app, actor, Instant::now()).await;
                        dirty = true;
                    }
                }
            }
            _ = ticker.tick(), if app.presentation.needs_tick() => {
                app.runtime_metrics.presentation_tick_wakeups = app
                    .runtime_metrics
                    .presentation_tick_wakeups
                    .saturating_add(1);
                dirty |= app.apply_tick(Instant::now());
            }
        }
    }
}

fn draw_ui(frame: &mut ratatui::Frame<'_>, app: &mut RichApp) {
    let area = frame.area();
    let footer_height = if area.width >= 80 { 2 } else { 3 };
    let [header, main, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(footer_height),
        ])
        .areas(area);
    let header_text = if area.width >= 72 {
        " mdstream agent workbench | typed Content IR, host-owned presentation"
    } else {
        " mdstream agent workbench"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            header_text,
            Style::default().fg(Color::Black).bg(Color::Cyan),
        )),
        header,
    );

    if area.width >= 100 && area.height >= 18 {
        let [activity, answer_area, inspector] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(28),
                Constraint::Min(38),
                Constraint::Length(30),
            ])
            .areas(main);
        frame.render_widget(
            Paragraph::new(Text::from(app.activity_lines()))
                .block(Block::default().title(" Activity ").borders(Borders::ALL))
                .wrap(Wrap { trim: false }),
            activity,
        );
        render_answer_panel(frame, app, answer_area);
        frame.render_widget(
            Paragraph::new(Text::from(app.inspector_lines()))
                .block(Block::default().title(" Inspector ").borders(Borders::ALL))
                .wrap(Wrap { trim: false }),
            inspector,
        );
    } else {
        render_answer_panel(frame, app, main);
    }
    frame.render_widget(
        Paragraph::new(status_line(app, area.width)).wrap(Wrap { trim: false }),
        footer,
    );
}

fn render_answer_panel(
    frame: &mut ratatui::Frame<'_>,
    app: &mut RichApp,
    area: ratatui::layout::Rect,
) {
    let inner_width = area.width.saturating_sub(2).max(1);
    let inner_height = area.height.saturating_sub(2).max(1);
    let layout_key = LayoutKey {
        answer_revision: app.answer_revision,
        width: inner_width,
        wrap: app.wrap,
    };
    if app.layout_key != Some(layout_key) {
        let answer = app.render_answer();
        app.last_layout = VisualLayout::from_answer(&answer, inner_width, app.wrap);
        app.layout_key = Some(layout_key);
        app.render_metrics.layout_builds = app.render_metrics.layout_builds.saturating_add(1);
    }

    let scroll_y = app.scroll.resolve(&app.last_layout, inner_height);
    let visible_range = app.last_layout.visible_range(scroll_y, inner_height);
    let lines = app.last_layout.rows[visible_range]
        .iter()
        .map(|row| materialize_visual_row(row, &app.presentation))
        .collect::<Vec<_>>();
    app.render_metrics.viewport_rows_materialized = app
        .render_metrics
        .viewport_rows_materialized
        .saturating_add(u64::try_from(lines.len()).unwrap_or(u64::MAX));
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(Block::default().title(" Answer ").borders(Borders::ALL)),
        area,
    );
}

fn materialize_visual_row(row: &VisualRow, presentation: &PresentationState) -> UiLine {
    let line = row.line.clone();
    match row
        .anchor
        .and_then(|anchor| presentation.line_stage(anchor.owner, anchor.row))
    {
        Some(LineStage::Queued) => line.style(Style::default().add_modifier(Modifier::DIM)),
        Some(LineStage::Committed | LineStage::Mutable) | None => line,
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
            if key.kind == KeyEventKind::Press
                && (matches!(key.code, KeyCode::Char('q') | KeyCode::Esc)
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)))
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiAction {
    None,
    Redraw,
    Quit,
}

fn handle_event(app: &mut RichApp, event: Event) -> UiAction {
    if is_quit_event(&event) {
        return UiAction::Quit;
    }
    if matches!(event, Event::Resize(_, _)) {
        return UiAction::Redraw;
    }
    let Event::Key(key) = event else {
        return UiAction::None;
    };
    if key.kind != KeyEventKind::Press {
        return UiAction::None;
    }

    match key.code {
        KeyCode::Char('f') => app.scroll.toggle_follow(&app.last_layout),
        KeyCode::Char('p') => {
            app.toggle_paused();
        }
        KeyCode::Char('m') => {
            app.toggle_reduced_motion();
        }
        KeyCode::Char('w') => app.wrap = !app.wrap,
        KeyCode::Char('j') | KeyCode::Down => app.scroll.scroll_by(&app.last_layout, 1),
        KeyCode::Char('k') | KeyCode::Up => app.scroll.scroll_by(&app.last_layout, -1),
        KeyCode::Char('g') | KeyCode::Home => app.scroll.top(&app.last_layout),
        KeyCode::Char('G') | KeyCode::End => app.scroll.follow_tail = true,
        _ => return UiAction::None,
    }
    UiAction::Redraw
}

fn status_line(app: &RichApp, width: u16) -> String {
    if width >= 100 {
        format!(
            "q/Esc quit | j/k scroll | g/G top/bottom | f follow={} | p paused={} | m motion={} | w wrap={} | actor={} | queue={} tail={} pending={} | batches={} changes={} errors={}",
            app.scroll.follow_tail,
            app.presentation.is_paused(),
            motion_label(app.presentation.is_reduced_motion()),
            app.wrap,
            app.actor_state.label(),
            app.presentation.queue_len(),
            app.presentation.mutable_root_count(),
            app.pending_bytes(),
            app.batches,
            app.changes,
            app.errors,
        )
    } else {
        format!(
            "q quit | j/k scroll | p paused={} | m {} | actor={} | queue={} tail={} pending={}",
            app.presentation.is_paused(),
            motion_label(app.presentation.is_reduced_motion()),
            app.actor_state.label(),
            app.presentation.queue_len(),
            app.presentation.mutable_root_count(),
            app.pending_bytes(),
        )
    }
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
    use crossterm::event::KeyEvent;
    use mdstream_protocol::{
        ChangeId, ChangeSet, ContinuityGeneration, Epoch, NodeId, ProtocolLimits, SourceCursor,
        SourceDelta, TransitionNodeKey,
    };
    use ratatui::backend::TestBackend;

    use super::*;

    fn key(node_id: u64) -> RootKey {
        TransitionNodeKey {
            continuity_generation: ContinuityGeneration::new(0),
            epoch: Epoch::new(0),
            node_id: NodeId::from(node_id),
        }
    }

    fn line_text(answer: &RenderedAnswer) -> String {
        answer
            .rows
            .iter()
            .flat_map(|row| &row.line.spans)
            .fold(String::new(), |mut text, span| {
                text.push_str(span.content.as_ref());
                text
            })
    }

    fn answer_with(owners: &[RootKey]) -> RenderedAnswer {
        RenderedAnswer {
            rows: owners
                .iter()
                .enumerate()
                .map(|(row, owner)| RenderedLine {
                    line: Line::from(format!("line {row}")),
                    anchor: Some(LineAnchor {
                        owner: *owner,
                        row: 0,
                    }),
                })
                .collect(),
        }
    }

    fn answer_rows_for_owner(owner: RootKey, count: usize) -> RenderedAnswer {
        RenderedAnswer {
            rows: (0..count)
                .map(|row| RenderedLine {
                    line: Line::from(format!("line {row}")),
                    anchor: Some(LineAnchor { owner, row }),
                })
                .collect(),
        }
    }

    fn terminal_text(app: &mut RichApp, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw_ui(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(usize::from(width))
            .fold(String::new(), |mut output, cells| {
                for cell in cells {
                    output.push_str(cell.symbol());
                }
                output.push('\n');
                output
            })
    }

    async fn render_chunk_schedule(chunks: Vec<String>, reduced_motion: bool) -> Vec<UiLine> {
        let expected_source = chunks.concat();
        let (input, input_rx) = mpsc::channel(INPUT_CAPACITY);
        let mut actor = spawn_stream_engine_actor(
            StreamEngine::new(),
            input_rx,
            CoalesceOptions::new(Duration::ZERO, 16 * 1024, 2048),
        );
        let producer = tokio::spawn(async move {
            for chunk in chunks {
                input.send(ActorCommand::Append(chunk)).await.unwrap();
            }
            input.send(ActorCommand::Finish).await.unwrap();
        });
        let now = Instant::now();
        let mut app = RichApp::new(reduced_motion);
        while let Some(batch) = actor.recv().await {
            app.apply_actor_batch(batch, now);
        }
        finish_actor(&mut app, &mut actor, now).await;
        producer.await.unwrap();

        let mut tick_at = now + CATCH_UP_TEST_AGE;
        while app.presentation.needs_tick() {
            app.apply_tick(tick_at);
            tick_at += PRESENTATION_TICK;
        }
        let presented = app
            .presentation
            .lines()
            .into_iter()
            .map(|line| line.line)
            .collect::<Vec<_>>();
        let document = app.reducer.document().expect("schedule creates a document");
        let mut direct_cache = ProjectionCache::default();
        let mut direct_syntax = SyntaxHighlighter::new();
        let direct = project_document(
            document,
            app.reducer.continuity_generation(),
            &mut direct_cache,
            direct_syntax.as_mut(),
        );

        assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
        assert_eq!(document.source(), expected_source);
        assert_eq!(presented, flatten_projections(&direct));
        assert!(app.presentation.is_idle());
        presented
    }

    const CATCH_UP_TEST_AGE: Duration = Duration::from_millis(121);
    const UTF8_SCHEDULE_MARKDOWN: &str = "# UTF-8 schedule\n\n你好, cafe\u{301} 👩‍💻.\n\n```rust\nfn main() { println!(\"你好\"); }\n```\n";

    fn uneven_character_chunks(source: &str) -> Vec<String> {
        let widths = [1, 3, 2, 5];
        let mut characters = source.chars();
        let mut chunks = Vec::new();
        for width in widths.into_iter().cycle() {
            let chunk = characters.by_ref().take(width).collect::<String>();
            if chunk.is_empty() {
                break;
            }
            chunks.push(chunk);
        }
        chunks
    }

    #[test]
    fn completed_activity_never_regresses_when_the_document_grows() {
        let mut activity = demo_activities().remove(0);

        activity.update(100);
        activity.update(0);

        assert_eq!(activity.state, ToolState::Complete);
    }

    #[test]
    fn pending_source_is_reported_but_never_rendered_as_transcript() {
        let mut engine = StreamEngine::new();
        let mut app = RichApp::default();
        let outputs = vec![engine.append("a *b").unwrap(), engine.append("**").unwrap()];
        app.apply_engine_outputs(outputs, Instant::now());
        let pending = app
            .reducer
            .document()
            .expect("fixture produces a canonical document")
            .pending_source()
            .to_string();
        assert!(!pending.is_empty(), "fixture must retain pending source");

        let rendered = app.render_answer();
        let text = line_text(&rendered);

        assert!(!text.contains(&pending));
        assert!(text.contains(&format!("pending {} source bytes", pending.len())));
    }

    #[test]
    fn one_engine_output_group_reconciles_only_its_tail_state() {
        let mut engine = StreamEngine::new();
        let mut app = RichApp::default();
        let outputs = vec![
            engine.append("title\n").unwrap(),
            engine.append("=====\n\n").unwrap(),
            engine.finish().unwrap(),
        ];

        app.apply_engine_outputs(outputs, Instant::now());

        let document = app.reducer.document().expect("batch creates a document");
        let mut direct_cache = ProjectionCache::default();
        let direct = project_document(
            document,
            app.reducer.continuity_generation(),
            &mut direct_cache,
            None,
        );
        let expected = flatten_projections(&direct);
        let actual = app
            .presentation
            .lines()
            .into_iter()
            .map(|line| line.line)
            .collect::<Vec<_>>();

        assert_eq!(app.presentation.metrics().reconciliations, 1);
        assert_eq!(app.batches, 1);
        assert_eq!(actual, expected);
        assert_eq!(app.presentation.queue_len(), expected.len());
        assert_eq!(
            actual
                .iter()
                .flat_map(|line| &line.spans)
                .filter(|span| span.content.contains("title"))
                .count(),
            1,
            "the superseded paragraph projection must never enter the queue"
        );
    }

    #[test]
    fn wrapped_lines_drive_visual_scroll_geometry() {
        let answer = RenderedAnswer {
            rows: vec![RenderedLine::unanchored(Line::from(
                "one two three four five",
            ))],
        };
        let visual_lines = VisualLayout::from_answer(&answer, 5, true).total_rows;

        assert!(visual_lines > answer.rows.len());
    }

    #[test]
    fn wrapped_lines_follow_ratatui_word_boundaries() {
        let line = Line::from("aaaaaa bbbbbb cccccc");
        let wrapped = wrap_ui_line(&line, 10);
        let expected_count = Paragraph::new(line)
            .wrap(Wrap { trim: false })
            .line_count(10);

        assert_eq!(wrapped.len(), expected_count);
        assert_eq!(
            wrapped
                .iter()
                .flat_map(|line| &line.spans)
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>(),
            vec!["aaaaaa", "bbbbbb", "cccccc"]
        );
    }

    #[test]
    fn removed_scroll_owner_falls_back_to_the_previous_survivor() {
        let first = key(1);
        let removed = key(2);
        let next = key(3);
        let old = VisualLayout::from_answer(&answer_with(&[first, removed, next]), 20, false);
        let mut scroll = ScrollState {
            follow_tail: false,
            ..ScrollState::default()
        };
        scroll.previous_owner_order.clone_from(&old.owner_order);
        scroll.anchor = Some(ScrollAnchor::Content {
            line: LineAnchor {
                owner: removed,
                row: 0,
            },
            wrapped_row: 0,
        });
        scroll.scroll_y = 1;
        let replacement = VisualLayout::from_answer(&answer_with(&[first, next]), 20, false);

        scroll.resolve(&replacement, 1);

        assert_eq!(
            scroll.anchor,
            Some(ScrollAnchor::Content {
                line: LineAnchor {
                    owner: first,
                    row: 0,
                },
                wrapped_row: 0,
            })
        );
        assert!(!scroll.follow_tail);
    }

    #[test]
    fn corrected_scroll_owner_keeps_its_nearest_surviving_row() {
        let previous = key(1);
        let corrected = key(2);
        let old = VisualLayout::from_answer(
            &RenderedAnswer {
                rows: answer_with(&[previous])
                    .rows
                    .into_iter()
                    .chain(answer_rows_for_owner(corrected, 4).rows)
                    .collect(),
            },
            20,
            false,
        );
        let mut scroll = ScrollState {
            follow_tail: false,
            scroll_y: 4,
            anchor: Some(ScrollAnchor::Content {
                line: LineAnchor {
                    owner: corrected,
                    row: 3,
                },
                wrapped_row: 0,
            }),
            ..ScrollState::default()
        };
        scroll.previous_owner_order.clone_from(&old.owner_order);
        let replacement = VisualLayout::from_answer(
            &RenderedAnswer {
                rows: answer_with(&[previous])
                    .rows
                    .into_iter()
                    .chain(answer_rows_for_owner(corrected, 1).rows)
                    .collect(),
            },
            20,
            false,
        );

        assert_eq!(scroll.resolve(&replacement, 1), 1);
        assert_eq!(
            scroll.anchor,
            Some(ScrollAnchor::Content {
                line: LineAnchor {
                    owner: corrected,
                    row: 0,
                },
                wrapped_row: 0,
            })
        );
    }

    #[test]
    fn follow_tail_scrolls_beyond_the_terminal_u16_range() {
        let layout = VisualLayout::from_answer(&answer_rows_for_owner(key(1), 70_000), 80, false);
        let mut scroll = ScrollState::default();

        let scroll_y = scroll.resolve(&layout, 20);

        assert_eq!(scroll_y, 69_980);
        assert_eq!(layout.visible_range(scroll_y, 20), 69_980..70_000);
    }

    #[test]
    fn manual_scroll_can_remain_on_an_unanchored_status_row() {
        let owner = key(1);
        let mut answer = answer_with(&[owner]);
        answer.rows.push(RenderedLine::unanchored(Line::from(
            "streaming | pending 4 source bytes",
        )));
        let layout = VisualLayout::from_answer(&answer, 40, false);
        let mut scroll = ScrollState {
            follow_tail: false,
            scroll_y: 1,
            viewport_height: 1,
            anchor: Some(ScrollAnchor::Trailing { row: 0 }),
            ..ScrollState::default()
        };

        assert_eq!(scroll.resolve(&layout, 1), 1);
        assert_eq!(scroll.anchor, Some(ScrollAnchor::Trailing { row: 0 }));
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
    fn argument_parser_accepts_reduced_motion_before_first_update() {
        assert_eq!(
            parse_args(["--reduced-motion".to_string()]).unwrap(),
            (false, true)
        );
        assert_eq!(
            parse_args(["--smoke".to_string(), "--reduced-motion".to_string()]).unwrap(),
            (true, true)
        );
    }

    #[test]
    fn presentation_ticks_reuse_layout_and_materialize_only_the_viewport() {
        let mut engine = StreamEngine::new();
        let mut app = RichApp::default();
        app.apply_engine_outputs(
            vec![
                engine.append("# title\n\nbody\n").unwrap(),
                engine.finish().unwrap(),
            ],
            Instant::now(),
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();

        terminal.draw(|frame| draw_ui(frame, &mut app)).unwrap();
        let first_layout_builds = app.render_metrics.layout_builds;
        let first_materialized = app.render_metrics.viewport_rows_materialized;
        assert!(app.apply_tick(Instant::now()));
        terminal.draw(|frame| draw_ui(frame, &mut app)).unwrap();

        assert_eq!(first_layout_builds, 1);
        assert_eq!(app.render_metrics.layout_builds, first_layout_builds);
        assert!(
            app.render_metrics
                .viewport_rows_materialized
                .saturating_sub(first_materialized)
                <= 21,
            "one presentation tick must materialize only the answer viewport"
        );
    }

    #[test]
    fn queued_style_changes_without_rebuilding_content_geometry() {
        let owner = key(1);
        let mut presentation = PresentationState::new();
        presentation.reconcile(
            vec![presentation::RootProjection::new(
                owner,
                vec![Line::from("queued")],
            )],
            Vec::new(),
            Instant::now(),
            false,
        );
        let row = VisualRow {
            line: Line::from("queued"),
            anchor: Some(LineAnchor { owner, row: 0 }),
            wrapped_row: 0,
        };

        let queued = materialize_visual_row(&row, &presentation);
        assert!(queued.style.add_modifier.contains(Modifier::DIM));
        presentation.tick(Instant::now());
        let committed = materialize_visual_row(&row, &presentation);
        assert!(!committed.style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn full_replace_resets_host_continuity_but_preserves_preferences() {
        let now = Instant::now();
        let mut engine = StreamEngine::new();
        let mut app = RichApp::default();
        app.apply_engine_outputs(
            vec![
                engine
                    .append("```rust\nfn old() { println!(\"old\"); }\n```\n\n")
                    .unwrap(),
                engine.finish().unwrap(),
            ],
            now,
        );
        let old_owners = app
            .presentation
            .lines()
            .into_iter()
            .map(|line| line.owner)
            .collect::<Vec<_>>();
        let old_owner = old_owners[0];
        assert!(
            app.syntax
                .as_ref()
                .is_some_and(|syntax| syntax.cache_len() > 0)
        );
        assert!(app.presentation.queue_len() > 1);
        assert!(app.apply_tick(now));
        assert!(app.presentation.queue_len() > 0);
        assert!(
            app.activities
                .iter()
                .any(|activity| activity.state != ToolState::Queued)
        );
        app.toggle_paused();
        app.toggle_reduced_motion();
        app.wrap = false;
        app.scroll.follow_tail = false;
        app.scroll.scroll_y = 7;
        app.scroll.anchor = Some(ScrollAnchor::Content {
            line: LineAnchor {
                owner: old_owner,
                row: 0,
            },
            wrapped_row: 0,
        });
        app.scroll.previous_owner_order = Arc::from([old_owner]);
        app.last_tick_lines = 9;
        app.last_tick_catch_up = true;

        app.apply_engine_outputs(
            vec![
                engine.reset().unwrap(),
                engine.append("# new\n\n").unwrap(),
                engine.finish().unwrap(),
            ],
            now + Duration::from_millis(1),
        );

        assert!(!app.scroll.follow_tail);
        assert_eq!(app.scroll.scroll_y, 0);
        assert_eq!(app.scroll.anchor, None);
        assert!(app.scroll.previous_owner_order.is_empty());
        assert!(app.presentation.is_paused());
        assert!(app.presentation.is_reduced_motion());
        assert!(!app.wrap);
        assert_eq!(app.last_tick_lines, 0);
        assert!(!app.last_tick_catch_up);
        assert!(
            app.activities
                .iter()
                .all(|activity| activity.state == ToolState::Queued)
        );
        assert_eq!(
            app.syntax.as_ref().map_or(0, SyntaxHighlighter::cache_len),
            0
        );
        assert!(
            app.presentation
                .lines()
                .iter()
                .all(|line| !old_owners.contains(&line.owner))
        );
    }

    #[test]
    fn paused_batches_refresh_canonical_tail_pending_and_activity_without_committing() {
        let now = Instant::now();
        let mut engine = StreamEngine::new();
        let mut app = RichApp::default();
        app.apply_engine_outputs(vec![engine.append("# committed\n\n").unwrap()], now);
        assert!(app.apply_tick(now));
        let committed_before = app.presentation.committed_line_count();
        assert!(committed_before > 0);
        assert!(app.toggle_paused());

        app.apply_engine_outputs(
            vec![engine.append("a *b").unwrap(), engine.append("*").unwrap()],
            now + Duration::from_millis(1),
        );

        let document = app.reducer.document().expect("batch creates a document");
        assert_eq!(document.source(), "# committed\n\na *b*");
        assert_eq!(document.pending_source(), "*");
        assert!(app.presentation.mutable_root_count() > 0);
        assert_eq!(app.presentation.committed_line_count(), committed_before);
        assert_eq!(
            app.activities
                .iter()
                .map(|activity| activity.state)
                .collect::<Vec<_>>(),
            vec![ToolState::Complete, ToolState::Running, ToolState::Queued]
        );
        assert!(line_text(&app.render_answer()).contains("pending 1 source bytes"));
    }

    #[tokio::test]
    async fn chunk_schedules_and_motion_policies_converge_to_identical_utf8_lines() {
        let schedules = [
            vec![UTF8_SCHEDULE_MARKDOWN.to_owned()],
            UTF8_SCHEDULE_MARKDOWN
                .chars()
                .map(|character| character.to_string())
                .collect(),
            uneven_character_chunks(UTF8_SCHEDULE_MARKDOWN),
        ];
        let mut expected = None;

        for reduced_motion in [false, true] {
            for chunks in schedules.iter().cloned() {
                let presented = render_chunk_schedule(chunks, reduced_motion).await;
                if let Some(expected) = &expected {
                    assert_eq!(&presented, expected);
                } else {
                    expected = Some(presented);
                }
            }
        }
    }

    #[test]
    fn narrow_terminal_covers_the_user_visible_state_matrix() {
        let now = Instant::now();

        let initial = RichApp::default();

        let mut pending = RichApp::default();
        let pending_change = ChangeSet::start_epoch(
            Epoch::new(0),
            ChangeId::new("fixture:pending-only").unwrap(),
            None,
            SourceDelta::append(SourceCursor::new(0), "pending"),
            Vec::new(),
        )
        .unwrap();
        let pending_facts = pending
            .reducer
            .apply(pending_change)
            .unwrap()
            .facts
            .expect("source-only change emits transition facts");
        let pending_full_replace = matches!(pending_facts, TransitionFacts::FullReplace { .. });
        pending.observe_facts(&pending_facts);
        pending.reconcile_presentation(now, pending_full_replace);
        pending.mark_answer_changed();
        pending.update_activities();
        assert_eq!(pending.presentation.line_count(), 0);
        assert!(pending.pending_bytes() > 0);

        let mut partial_engine = StreamEngine::new();
        let mut partial = RichApp::default();
        partial.apply_engine_outputs(
            vec![partial_engine.append("# partial\n\nbody").unwrap()],
            now,
        );

        let mut paused_engine = StreamEngine::new();
        let mut paused = RichApp::default();
        paused.apply_engine_outputs(
            vec![
                paused_engine.append("# queued\n\n").unwrap(),
                paused_engine.finish().unwrap(),
            ],
            now,
        );
        paused.toggle_paused();

        let mut failed = RichApp {
            actor_state: ActorState::Failed,
            ..RichApp::default()
        };
        failed.record_error("fixture failure".to_owned());

        let mut empty_engine = StreamEngine::new();
        let mut empty = RichApp::default();
        empty.apply_engine_outputs(vec![empty_engine.finish().unwrap()], now);
        empty.actor_state = ActorState::Completed;
        empty.mark_answer_changed();
        empty.update_activities();

        let mut complete_engine = StreamEngine::new();
        let mut complete = RichApp::default();
        complete.apply_engine_outputs(
            vec![
                complete_engine.append("# done\n\n").unwrap(),
                complete_engine.finish().unwrap(),
            ],
            now,
        );
        complete.actor_state = ActorState::Completed;
        let drained = complete.presentation.drain_all();
        complete.apply_tick_result(drained);
        complete.mark_answer_changed();
        complete.update_activities();

        let cases = [
            ("initial", initial, "Waiting", "actor=running"),
            ("pending", pending, "pending", "actor=running"),
            ("partial", partial, "# partial", "actor=running"),
            ("paused", paused, "# queued", "paused=true"),
            ("failed", failed, "stream error", "actor=failed"),
            (
                "empty complete",
                empty,
                "Completed with no content",
                "actor=complete",
            ),
            ("non-empty complete", complete, "# done", "actor=complete"),
        ];

        for (name, mut app, expected_answer, expected_status) in cases {
            let screen = terminal_text(&mut app, 48, 10);
            assert!(screen.contains("Answer"), "{name}: answer panel missing");
            assert!(screen.contains("q quit"), "{name}: exit control missing");
            assert!(
                screen.contains(expected_answer),
                "{name}: answer state missing from {screen:?}"
            );
            assert!(
                screen.contains(expected_status),
                "{name}: status missing from {screen:?}"
            );
            assert!(
                !screen.contains("Activity"),
                "{name}: narrow panels overlap"
            );
            assert!(
                !screen.contains("Inspector"),
                "{name}: narrow panels overlap"
            );

            let compact = terminal_text(&mut app, 32, 7);
            assert!(compact.contains("Answer"), "{name}: compact answer missing");
            assert!(compact.contains("q quit"), "{name}: compact exit missing");
            assert!(
                !compact.contains("Activity"),
                "{name}: compact panels overlap"
            );
            assert!(
                !compact.contains("Inspector"),
                "{name}: compact panels overlap"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn settled_event_loop_waits_without_periodic_ticks_or_draws() {
        let (input, input_rx) = mpsc::channel(4);
        let mut actor = spawn_stream_engine_actor(
            StreamEngine::new(),
            input_rx,
            CoalesceOptions::new(Duration::from_millis(20), 1024, 8),
        );
        input
            .send(ActorCommand::Append("# done\n\n".to_string()))
            .await
            .unwrap();
        input.send(ActorCommand::Finish).await.unwrap();
        drop(input);

        let mut app = RichApp::default();
        finish_actor(&mut app, &mut actor, Instant::now()).await;
        let drained = app.presentation.drain_all();
        app.apply_tick_result(drained);
        assert!(app.is_settled());
        let draws_before_wait = app.runtime_metrics.draws;
        let ticks_before_wait = app.runtime_metrics.presentation_tick_wakeups;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let (events, mut event_rx) = mpsc::channel(1);
        let started = tokio::time::Instant::now();
        let event_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            events
                .send(Event::Key(KeyEvent::new(
                    KeyCode::Char('q'),
                    KeyModifiers::NONE,
                )))
                .await
                .unwrap();
        });
        let quit_requested = AtomicBool::new(false);

        run(
            &mut terminal,
            &mut app,
            &mut actor,
            &mut event_rx,
            &quit_requested,
        )
        .await
        .unwrap();
        event_task.await.unwrap();

        assert!(tokio::time::Instant::now() >= started + Duration::from_secs(1));
        assert_eq!(app.actor_state, ActorState::Completed);
        assert!(app.is_settled());
        assert_eq!(
            app.runtime_metrics.presentation_tick_wakeups,
            ticks_before_wait
        );
        assert_eq!(app.runtime_metrics.draws, draws_before_wait + 1);
    }

    #[tokio::test]
    async fn interactive_cancellation_applies_unread_and_unpublished_batches_once() {
        const COMMAND_COUNT: usize = 65;

        let (input, input_rx) = mpsc::channel(COMMAND_COUNT);
        let mut actor = spawn_stream_engine_actor(
            StreamEngine::new(),
            input_rx,
            CoalesceOptions::new(Duration::from_secs(60), 1024, COMMAND_COUNT),
        );
        for _ in 0..COMMAND_COUNT {
            input.send(ActorCommand::Reset).await.unwrap();
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            while input.capacity() != COMMAND_COUNT {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actor must receive every queued reset");
        let mut app = RichApp::default();

        settle_actor_after_run(&mut app, &mut actor, Instant::now()).await;
        drop(input);

        assert_eq!(app.actor_state, ActorState::Cancelled);
        assert_eq!(app.errors, 0);
        assert_eq!(app.batches, COMMAND_COUNT as u64);
        assert_eq!(app.changes, COMMAND_COUNT as u64);
        assert_eq!(
            app.presentation.metrics().reconciliations,
            COMMAND_COUNT as u64
        );
    }

    #[tokio::test]
    async fn actor_partial_failure_reconciles_its_completed_prefix_before_error() {
        let engine = StreamEngine::builder()
            .protocol_limits(ProtocolLimits {
                max_source_bytes: 1,
                ..ProtocolLimits::default()
            })
            .build()
            .unwrap();
        let (input, input_rx) = mpsc::channel(8);
        let mut actor = spawn_stream_engine_actor(
            engine,
            input_rx,
            CoalesceOptions::new(Duration::from_millis(20), 1024, 8).with_newline_flush(false),
        );
        input
            .send(ActorCommand::Append("a".to_string()))
            .await
            .unwrap();
        input
            .send(ActorCommand::Append("b".to_string()))
            .await
            .unwrap();
        drop(input);
        let mut app = RichApp::default();

        finish_actor(&mut app, &mut actor, Instant::now()).await;

        assert_eq!(app.actor_state, ActorState::Failed);
        assert_eq!(app.errors, 1);
        assert_eq!(
            app.reducer
                .document()
                .expect("completed prefix creates a document")
                .source(),
            "a"
        );
        assert_eq!(
            app.reducer
                .document()
                .expect("completed prefix creates a document")
                .lifecycle(),
            DocumentLifecycle::Open
        );
        assert_eq!(app.presentation.metrics().reconciliations, 1);
        assert!(!app.is_settled());
    }
}
