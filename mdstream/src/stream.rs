use std::collections::{HashMap, HashSet};

mod compaction;
mod footnotes;
mod html;
mod lines;
mod refs;

use self::footnotes::{detect_footnotes, is_footnote_continuation, is_footnote_definition_start};
use self::html::{html_block_start_state, update_html_block_state};
use self::lines::{Line, take_prefix_at_char_boundary, update_tail};
use self::refs::extract_reference_usages;

use crate::boundary::{BoundaryPlugin, BoundaryUpdate};
use crate::options::{FootnotesMode, Options, ReferenceDefinitionsMode};
use crate::pending::terminate_markdown;
use crate::reference::extract_reference_definition_label;
use crate::transform::{PendingTransformInput, PendingTransformer};
use crate::types::{Block, BlockId, BlockKind, BlockStatus, PendingBlockRef, Update, UpdateRef};

#[derive(Debug, Clone)]
enum BlockMode {
    Unknown,
    Paragraph,
    Heading,
    ThematicBreak,
    CodeFence {
        fence_char: char,
        fence_len: usize,
    },
    CustomBoundary {
        plugin_index: usize,
        started: bool,
    },
    List,
    BlockQuote,
    HtmlBlock {
        stack: Vec<String>,
        in_comment: bool,
    },
    Table,
    MathBlock {
        open_count: usize,
    },
    FootnoteDefinition,
}

fn is_empty_line(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#') && trimmed[1..].starts_with([' ', '\t', '#'])
}

fn thematic_break_char(line: &str) -> Option<char> {
    // CommonMark-like thematic break:
    // - up to 3 leading spaces
    // - one of '-', '*', '_' repeated >= 3
    // - spaces/tabs may appear between markers
    // - no other characters
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let s = s.trim_end_matches([' ', '\t']);
    let mut it = s.chars();
    let first = it.next()?;
    if first != '-' && first != '*' && first != '_' {
        return None;
    }
    let mut count = 1usize;
    for c in it {
        if c == first {
            count += 1;
            continue;
        }
        if c == ' ' || c == '\t' {
            continue;
        }
        return None;
    }
    if count >= 3 { Some(first) } else { None }
}

fn is_thematic_break(line: &str) -> bool {
    thematic_break_char(line).is_some()
}

fn setext_underline_char(line: &str) -> Option<char> {
    // Best-effort setext underline:
    // - up to 3 leading spaces
    // - '=' or '-' repeated >= 2
    // - spaces/tabs may appear between markers
    // - no other characters
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let s = s.trim_end_matches([' ', '\t']);
    let mut it = s.chars();
    let first = it.next()?;
    if first != '=' && first != '-' {
        return None;
    }
    let mut count = 1usize;
    for c in it {
        if c == first {
            count += 1;
            continue;
        }
        if c == ' ' || c == '\t' {
            continue;
        }
        return None;
    }
    if count >= 2 { Some(first) } else { None }
}

fn fence_start(line: &str) -> Option<(char, usize)> {
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let bytes = s.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let ch = bytes[0] as char;
    if ch != '`' && ch != '~' {
        return None;
    }
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == bytes[0] {
        len += 1;
    }
    if len < 3 {
        return None;
    }
    Some((ch, len))
}

fn fence_end(line: &str, fence_char: char, fence_len: usize) -> bool {
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let trimmed = s.trim_end();
    trimmed.chars().all(|c| c == fence_char) && trimmed.chars().count() >= fence_len
}

fn code_fence_suffix(raw_ended_with_newline: bool, fence_char: char, fence_len: usize) -> String {
    let mut out = String::new();
    if !raw_ended_with_newline {
        out.push('\n');
    }
    for _ in 0..fence_len {
        out.push(fence_char);
    }
    out.push('\n');
    out
}

fn is_blockquote_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('>')
}

fn is_list_item_start(line: &str) -> bool {
    let s = line.trim_start();
    if s.len() < 2 {
        return false;
    }
    let bytes = s.as_bytes();
    match bytes[0] {
        b'-' | b'+' | b'*' => bytes[1] == b' ' || bytes[1] == b'\t',
        b'0'..=b'9' => {
            let mut i = 0usize;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == 0 || i + 1 >= bytes.len() {
                return false;
            }
            (bytes[i] == b'.' || bytes[i] == b')')
                && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
        }
        _ => false,
    }
}

fn is_list_continuation(line: &str) -> bool {
    // Best-effort continuation line for lists:
    // - indented content (>=2 spaces or a tab)
    // - or a nested list item starter
    if is_list_item_start(line) {
        return true;
    }
    let bytes = line.as_bytes();
    if bytes.first() == Some(&b'\t') {
        return true;
    }
    let mut spaces = 0usize;
    for &b in bytes {
        if b == b' ' {
            spaces += 1;
            if spaces >= 2 {
                return true;
            }
            continue;
        }
        break;
    }
    false
}

fn is_list_item_start_prefix(line: &str) -> bool {
    // Streaming-only heuristic: treat certain "prefix" lines as potential list item starters.
    // This prevents premature block commits when the marker is split across chunks (e.g. "-" then " item").
    let s = line.trim_start();
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    match bytes[0] {
        b'-' | b'+' | b'*' => s.len() == 1,
        b'0'..=b'9' => {
            let mut i = 0usize;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == 0 {
                return false;
            }
            if i == bytes.len() {
                // Digits only: could become "1." / "1)" with more input.
                return true;
            }
            if bytes[i] != b'.' && bytes[i] != b')' {
                return false;
            }
            if i + 1 == bytes.len() {
                // "1." or "1)" without the required whitespace yet.
                return true;
            }
            false
        }
        _ => false,
    }
}

fn count_double_dollars(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'$' {
            if i > 0 && bytes[i - 1] == b'\\' {
                i += 2;
                continue;
            }
            count += 1;
            i += 2;
            continue;
        }
        i += 1;
    }
    count
}

pub struct MdStream {
    opts: Options,
    buffer: String,
    lines: Vec<Line>,

    committed: Vec<Block>,
    processed_line: usize,
    current_block_start_line: usize,
    current_block_id: BlockId,
    next_block_id: u64,
    current_mode: BlockMode,

    pending_display_cache: Option<String>,
    pending_display_cache_suffix: Option<String>,
    pending_transformers: Vec<Box<dyn PendingTransformer>>,
    boundary_plugins: Vec<Box<dyn BoundaryPlugin>>,
    active_boundary_plugin: Option<usize>,
    footnotes_detected: bool,
    footnote_scan_tail: String,
    pending_cr: bool,
    last_finalized_buffer_len: usize,

    reference_usage_index: HashMap<String, HashSet<BlockId>>,
}

struct AppendCtx<'a> {
    committed_out: Option<&'a mut Vec<Block>>,
    invalidated: Vec<BlockId>,
    reset: bool,
}

impl<'a> AppendCtx<'a> {
    fn new(committed_out: Option<&'a mut Vec<Block>>) -> Self {
        Self {
            committed_out,
            invalidated: Vec::new(),
            reset: false,
        }
    }

    fn push_committed_clone(&mut self, block: &Block) {
        if let Some(out) = self.committed_out.as_deref_mut() {
            out.push(block.clone());
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingInfo {
    id: BlockId,
    kind: BlockKind,
    raw_start: usize,
}

impl std::fmt::Debug for MdStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdStream")
            .field("buffer_len", &self.buffer.len())
            .field("lines_len", &self.lines.len())
            .field("committed_len", &self.committed.len())
            .field("processed_line", &self.processed_line)
            .field("current_block_start_line", &self.current_block_start_line)
            .field("current_block_id", &self.current_block_id)
            .field("next_block_id", &self.next_block_id)
            .field(
                "pending_display_cache",
                &self.pending_display_cache.is_some(),
            )
            .field(
                "pending_display_cache_suffix",
                &self.pending_display_cache_suffix.is_some(),
            )
            .field("pending_transformers_len", &self.pending_transformers.len())
            .field("boundary_plugins_len", &self.boundary_plugins.len())
            .field("active_boundary_plugin", &self.active_boundary_plugin)
            .field("footnotes_detected", &self.footnotes_detected)
            .field("last_finalized_buffer_len", &self.last_finalized_buffer_len)
            .finish()
    }
}

impl MdStream {
    pub fn new(opts: Options) -> Self {
        let mut opts = opts;
        // Keep the window in one place: Options and TerminatorOptions should agree.
        opts.terminator.window_bytes = opts.terminator_window_bytes;
        Self {
            opts,
            buffer: String::new(),
            lines: vec![Line {
                start: 0,
                end: 0,
                has_newline: false,
            }],
            committed: Vec::new(),
            processed_line: 0,
            current_block_start_line: 0,
            current_block_id: BlockId(1),
            next_block_id: 2,
            current_mode: BlockMode::Unknown,
            pending_display_cache: None,
            pending_display_cache_suffix: None,
            pending_transformers: Vec::new(),
            boundary_plugins: Vec::new(),
            active_boundary_plugin: None,
            footnotes_detected: false,
            footnote_scan_tail: String::new(),
            pending_cr: false,
            last_finalized_buffer_len: 0,
            reference_usage_index: HashMap::new(),
        }
    }

    /// Construct a stream with Streamdown-compatible defaults for incomplete links/images.
    ///
    /// This keeps the built-in terminator for emphasis/inline code/etc, but delegates incomplete
    /// link/image handling to the built-in pending transformers.
    pub fn streamdown_defaults() -> Self {
        // Use the transformers for link/image behavior so consumers can swap them out.
        let opts = Options {
            terminator: crate::pending::TerminatorOptions {
                links: false,
                images: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut s = MdStream::new(opts.clone());
        s.push_pending_transformer(crate::transform::IncompleteLinkPlaceholderTransformer {
            incomplete_link_url: opts.terminator.incomplete_link_url,
            window_bytes: opts.terminator_window_bytes,
        });
        s.push_pending_transformer(crate::transform::IncompleteImageDropTransformer {
            window_bytes: opts.terminator_window_bytes,
        });
        s
    }

    pub fn push_pending_transformer<T>(&mut self, transformer: T)
    where
        T: PendingTransformer + 'static,
    {
        self.pending_transformers.push(Box::new(transformer));
        self.pending_display_cache = None;
        self.pending_display_cache_suffix = None;
    }

    pub fn with_pending_transformer<T>(mut self, transformer: T) -> Self
    where
        T: PendingTransformer + 'static,
    {
        self.push_pending_transformer(transformer);
        self
    }

    pub fn push_boundary_plugin<T>(&mut self, plugin: T)
    where
        T: BoundaryPlugin + 'static,
    {
        self.boundary_plugins.push(Box::new(plugin));
        self.pending_display_cache = None;
        self.pending_display_cache_suffix = None;
    }

    pub fn with_boundary_plugin<T>(mut self, plugin: T) -> Self
    where
        T: BoundaryPlugin + 'static,
    {
        self.push_boundary_plugin(plugin);
        self
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn snapshot_blocks(&mut self) -> Vec<Block> {
        let mut blocks = self.committed.clone();
        // Pending is computed without mutating structural state, but pending transformers may
        // choose to keep internal state.
        if let Some(p) = self.pending_block_snapshot() {
            blocks.push(p);
        }
        blocks
    }

    fn start_mode_for_line(&self, line: &str) -> BlockMode {
        if let Some(idx) = self
            .boundary_plugins
            .iter()
            .position(|p| p.matches_start(line))
        {
            return BlockMode::CustomBoundary {
                plugin_index: idx,
                started: false,
            };
        }
        if is_heading(line) {
            return BlockMode::Heading;
        }
        if is_thematic_break(line) {
            return BlockMode::ThematicBreak;
        }
        if let Some((ch, len)) = fence_start(line) {
            return BlockMode::CodeFence {
                fence_char: ch,
                fence_len: len,
            };
        }
        if is_footnote_definition_start(line) {
            return BlockMode::FootnoteDefinition;
        }
        if is_blockquote_start(line) {
            return BlockMode::BlockQuote;
        }
        if is_list_item_start(line) {
            return BlockMode::List;
        }
        if let Some((stack, in_comment)) = html_block_start_state(line) {
            return BlockMode::HtmlBlock { stack, in_comment };
        }
        let dollars = count_double_dollars(line);
        if dollars % 2 == 1 && line.trim_start().starts_with("$$") {
            // `open_count` is tracked via `update_mode_with_line`, including the opening line.
            return BlockMode::MathBlock { open_count: 0 };
        }
        BlockMode::Paragraph
    }

    fn kind_for_mode(mode: &BlockMode) -> BlockKind {
        match mode {
            BlockMode::Paragraph => BlockKind::Paragraph,
            BlockMode::Heading => BlockKind::Heading,
            BlockMode::ThematicBreak => BlockKind::ThematicBreak,
            BlockMode::CodeFence { .. } => BlockKind::CodeFence,
            BlockMode::CustomBoundary { .. } => BlockKind::Unknown,
            BlockMode::List => BlockKind::List,
            BlockMode::BlockQuote => BlockKind::BlockQuote,
            BlockMode::HtmlBlock { .. } => BlockKind::HtmlBlock,
            BlockMode::Table => BlockKind::Table,
            BlockMode::MathBlock { .. } => BlockKind::MathBlock,
            BlockMode::FootnoteDefinition => BlockKind::FootnoteDefinition,
            BlockMode::Unknown => BlockKind::Unknown,
        }
    }

    fn commit_block(&mut self, end_line_inclusive: usize, ctx: &mut AppendCtx<'_>) {
        if self.current_block_start_line >= self.lines.len() {
            return;
        }
        if end_line_inclusive < self.current_block_start_line {
            return;
        }
        let start_off = self.lines[self.current_block_start_line].start;
        let end_off = self.lines[end_line_inclusive].end_with_newline();
        if end_off <= start_off {
            return;
        }

        let raw = self.buffer[start_off..end_off].to_string();
        if raw.trim().is_empty() {
            // Never emit whitespace-only blocks. Keep stable behavior by advancing the block cursor.
            self.current_block_start_line = end_line_inclusive + 1;
            self.current_block_id = BlockId(self.next_block_id);
            self.next_block_id += 1;
            self.current_mode = BlockMode::Unknown;
            self.active_boundary_plugin = None;
            self.pending_display_cache = None;
            self.pending_display_cache_suffix = None;
            return;
        }
        let block = Block {
            id: self.current_block_id,
            status: BlockStatus::Committed,
            kind: Self::kind_for_mode(&self.current_mode),
            raw,
            display: None,
        };
        self.push_committed_block(block, ctx);

        self.current_block_start_line = end_line_inclusive + 1;
        self.current_block_id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        self.current_mode = BlockMode::Unknown;
        self.active_boundary_plugin = None;
        self.pending_display_cache = None;
        self.pending_display_cache_suffix = None;
    }

    fn push_committed_block(&mut self, block: Block, ctx: &mut AppendCtx<'_>) {
        // Index usages for invalidation-based adapters.
        if block.kind != BlockKind::CodeFence && block.raw.contains('[') {
            let used = extract_reference_usages(&block.raw);
            if !used.is_empty() {
                for label in used {
                    self.reference_usage_index
                        .entry(label)
                        .or_default()
                        .insert(block.id);
                }
            }
        }

        // Emit invalidations when new reference definitions arrive.
        if self.opts.reference_definitions == ReferenceDefinitionsMode::Invalidate
            && block.kind != BlockKind::CodeFence
            && block.raw.contains("]:")
        {
            let mut invalidated = HashSet::new();
            for line in block.raw.split('\n') {
                let Some(label) = extract_reference_definition_label(line) else {
                    continue;
                };
                if let Some(ids) = self.reference_usage_index.get(&label) {
                    for id in ids {
                        if *id != block.id {
                            invalidated.insert(*id);
                        }
                    }
                }
            }
            if !invalidated.is_empty() {
                let mut ids: Vec<BlockId> = invalidated.into_iter().collect();
                ids.sort_by_key(|id| id.0);
                ctx.invalidated.extend(ids);
            }
        }

        self.committed.push(block);
        let block = self
            .committed
            .last()
            .expect("committed block must exist after push");
        ctx.push_committed_clone(block);
    }

    fn maybe_commit_single_line(&mut self, line_index: usize, ctx: &mut AppendCtx<'_>) {
        match self.current_mode {
            BlockMode::Heading | BlockMode::ThematicBreak => {
                self.commit_block(line_index, ctx);
            }
            _ => {}
        }
    }

    fn line_str(&self, line_index: usize) -> &str {
        self.lines[line_index].as_str(&self.buffer)
    }

    fn process_line(&mut self, line_index: usize, ctx: &mut AppendCtx<'_>) {
        // Skip if this line does not yet end with newline; we can't do stable boundary checks.
        if !self.lines[line_index].has_newline {
            return;
        }

        // If we're in SingleBlock footnote mode, we bypass block splitting.
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            return;
        }

        if line_index == self.current_block_start_line {
            // Defensive: the first line of a block is the single source of truth for the block mode.
            // This avoids stale-mode edge cases where `current_mode` is not `Unknown` at a new start.
            self.current_mode = self.start_mode_for_line(self.line_str(line_index));
            self.maybe_commit_single_line(line_index, ctx);
            // Even on the first line, some modes need to update internal state (e.g. HTML tag stack).
            self.update_mode_with_line(line_index, ctx);
            return;
        }

        let (boundary, next_mode) = {
            let prev = self.line_str(line_index - 1);
            let curr = self.line_str(line_index);
            let boundary = self.is_new_block_boundary(prev, curr, line_index);
            let next_mode = if boundary {
                Some(self.start_mode_for_line(curr))
            } else {
                None
            };
            (boundary, next_mode)
        };

        // Decide if current line starts a new block; if so, commit the previous block at prev line.
        if boundary {
            self.commit_block(line_index - 1, ctx);
            if let Some(m) = next_mode {
                self.current_mode = m;
            }
            self.maybe_commit_single_line(line_index, ctx);
            // If we started a new mode on this line, we must also update its per-line state.
            // This is required for modes like HTML/math where the opening line affects context.
            self.update_mode_with_line(line_index, ctx);
            return;
        }

        // Update per-block mode state transitions.
        self.update_mode_with_line(line_index, ctx);
    }

    fn process_incomplete_tail_boundary(&mut self, ctx: &mut AppendCtx<'_>) {
        if self.lines.len() < 2 {
            return;
        }
        let last = self.lines.len() - 1;
        if self.lines[last].has_newline {
            return;
        }
        if !self.lines[last - 1].has_newline {
            return;
        }

        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            return;
        }

        let boundary = {
            let prev = self.line_str(last - 1);
            let curr = self.line_str(last);
            self.is_new_block_boundary(prev, curr, last)
        };

        if boundary {
            self.commit_block(last - 1, ctx);
            self.current_mode = self.start_mode_for_line(self.line_str(last));
        }
    }

    fn is_new_block_boundary(&self, prev: &str, curr: &str, curr_line_index: usize) -> bool {
        // Never split inside fenced code blocks.
        if let BlockMode::CodeFence { .. } = self.current_mode {
            return false;
        }
        if let BlockMode::CustomBoundary { .. } = self.current_mode {
            return false;
        }
        if let BlockMode::MathBlock { open_count } = self.current_mode {
            if open_count % 2 == 1 {
                return false;
            }
        }
        if let BlockMode::HtmlBlock { stack, in_comment } = &self.current_mode {
            if *in_comment || !stack.is_empty() {
                return false;
            }
        }

        // Footnote definition: continuation lines should remain in the same block.
        if let BlockMode::FootnoteDefinition = self.current_mode {
            if is_empty_line(curr) || is_footnote_continuation(curr) {
                return false;
            }
            // A non-indented, non-empty line ends the footnote definition even without a blank line.
            return true;
        }

        // A new block can start after an empty line.
        if is_empty_line(prev) && !is_empty_line(curr) {
            // Be robust against mode drift in streaming scenarios: the current block's "start line"
            // is the source of truth for whether we're inside a list/quote container.
            let block_start_mode =
                self.start_mode_for_line(self.line_str(self.current_block_start_line));
            let in_list = matches!(self.current_mode, BlockMode::List)
                || matches!(block_start_mode, BlockMode::List);
            let in_blockquote = matches!(self.current_mode, BlockMode::BlockQuote)
                || matches!(block_start_mode, BlockMode::BlockQuote);
            // Lists can legally contain blank lines between items and within an item's continuation.
            if in_list && (is_list_continuation(curr) || is_list_item_start_prefix(curr)) {
                return false;
            }
            // Blockquotes can continue after blank lines only if the marker is present.
            if in_blockquote && is_blockquote_start(curr) {
                return false;
            }
            return true;
        }

        // Setext heading underline is part of the current paragraph block, not a new block boundary.
        if matches!(self.current_mode, BlockMode::Paragraph | BlockMode::Unknown)
            && setext_underline_char(curr).is_some()
            && !is_empty_line(prev)
            && self.current_block_start_line + 1 == curr_line_index
        {
            return false;
        }

        // Certain block starters can interrupt paragraphs/lists/quotes.
        if is_heading(curr) || is_thematic_break(curr) {
            return true;
        }
        if fence_start(curr).is_some() {
            return true;
        }
        if self.boundary_plugins.iter().any(|p| p.matches_start(curr)) {
            return true;
        }
        if is_footnote_definition_start(curr) {
            return true;
        }
        if is_blockquote_start(curr)
            && !is_blockquote_start(prev)
            && !matches!(self.current_mode, BlockMode::BlockQuote)
        {
            return true;
        }
        if is_list_item_start(curr)
            && !is_list_item_start(prev)
            && !matches!(self.current_mode, BlockMode::List)
        {
            return true;
        }

        // Table detection: if current line is a delimiter and previous line contains pipes,
        // consider starting a table block at the previous line.
        if matches!(self.current_mode, BlockMode::Paragraph | BlockMode::Unknown)
            && self.is_table_delimiter(curr)
            && prev.contains('|')
            // table starts at prev line, so boundary at prev-1 if block started earlier.
            && curr_line_index >= 1
            && self.current_block_start_line < curr_line_index - 1
        {
            return true;
        }

        false
    }

    fn is_table_delimiter(&self, line: &str) -> bool {
        let s = line.trim();
        if s.is_empty() {
            return false;
        }
        // Simple delimiter pattern: contains '-' and optional pipes/colons.
        let mut has_dash = false;
        for c in s.chars() {
            match c {
                '|' | ':' | ' ' | '\t' => {}
                '-' => has_dash = true,
                _ => return false,
            }
        }
        has_dash
    }

    fn update_mode_with_line(&mut self, line_index: usize, ctx: &mut AppendCtx<'_>) {
        let (start, end) = {
            let l = &self.lines[line_index];
            (l.start, l.end)
        };
        let line = &self.buffer[start..end];
        match &mut self.current_mode {
            BlockMode::Unknown => {
                self.current_mode = self.start_mode_for_line(line);
                self.maybe_commit_single_line(line_index, ctx);
            }
            BlockMode::CodeFence {
                fence_char,
                fence_len,
            } => {
                // Opening fence matches `fence_end()` pattern but must not close itself.
                if line_index > self.current_block_start_line
                    && fence_end(line, *fence_char, *fence_len)
                {
                    self.commit_block(line_index, ctx);
                }
            }
            BlockMode::CustomBoundary {
                plugin_index,
                started,
            } => {
                let idx = *plugin_index;
                if idx >= self.boundary_plugins.len() {
                    return;
                }
                self.active_boundary_plugin = Some(idx);
                if !*started {
                    self.boundary_plugins[idx].start(line);
                    *started = true;
                }
                if self.boundary_plugins[idx].update(line) == BoundaryUpdate::Close {
                    self.active_boundary_plugin = None;
                    self.commit_block(line_index, ctx);
                }
            }
            BlockMode::MathBlock { open_count } => {
                *open_count += count_double_dollars(line);
                if *open_count % 2 == 0 {
                    self.commit_block(line_index, ctx);
                }
            }
            BlockMode::Paragraph => {
                // Upgrade to setext heading if underline appears right after a single paragraph line.
                if setext_underline_char(line).is_some()
                    && self.current_block_start_line + 1 == line_index
                    && line_index > 0
                {
                    let prev = self.lines[line_index - 1].as_str(&self.buffer);
                    if !is_empty_line(prev) {
                        self.current_mode = BlockMode::Heading;
                        self.commit_block(line_index, ctx);
                        return;
                    }
                }
                // Upgrade to table mode if delimiter row appears.
                if self.is_table_delimiter(line) && line_index > 0 {
                    let prev = self.lines[line_index - 1].as_str(&self.buffer);
                    if prev.contains('|') {
                        self.current_mode = BlockMode::Table;
                    }
                }
            }
            BlockMode::Table => {
                // End table when an empty line is followed by a non-table line.
                // This is handled by boundary detection on next line arrival.
            }
            BlockMode::HtmlBlock { stack, in_comment } => {
                update_html_block_state(line, stack, in_comment);
                if !*in_comment && stack.is_empty() {
                    self.commit_block(line_index, ctx);
                }
            }
            BlockMode::FootnoteDefinition => {
                // Continuation handled by boundary logic.
            }
            BlockMode::List | BlockMode::BlockQuote => {
                // Conservative: rely on boundary logic on next line arrival.
            }
            BlockMode::Heading | BlockMode::ThematicBreak => {}
        }
    }

    fn current_pending_info(&self) -> Option<PendingInfo> {
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            if self.buffer.is_empty() {
                return None;
            }
            return Some(PendingInfo {
                id: BlockId(1),
                kind: BlockKind::Unknown,
                raw_start: 0,
            });
        }

        if self.current_block_start_line >= self.lines.len() {
            return None;
        }
        let start_off = self.lines[self.current_block_start_line].start;
        if start_off >= self.buffer.len() {
            return None;
        }
        if self.buffer[start_off..].is_empty() {
            return None;
        }

        let kind = if matches!(self.current_mode, BlockMode::Unknown) {
            let mode = self.start_mode_for_line(self.line_str(self.current_block_start_line));
            Self::kind_for_mode(&mode)
        } else {
            Self::kind_for_mode(&self.current_mode)
        };

        Some(PendingInfo {
            id: self.current_block_id,
            kind,
            raw_start: start_off,
        })
    }

    fn ensure_current_pending_display(&mut self) {
        let Some(info) = self.current_pending_info() else {
            self.pending_display_cache = None;
            self.pending_display_cache_suffix = None;
            return;
        };
        self.ensure_pending_display_for(info.kind, info.raw_start);
    }

    fn current_pending_ref_readonly(&self) -> Option<PendingBlockRef<'_>> {
        let info = self.current_pending_info()?;
        let raw = &self.buffer[info.raw_start..];
        Some(PendingBlockRef {
            id: info.id,
            kind: info.kind,
            raw,
            display: self.pending_display_cache.as_deref(),
        })
    }

    fn transform_pending_display_at(
        &mut self,
        kind: BlockKind,
        raw_start: usize,
        mut display: String,
    ) -> String {
        if self.pending_transformers.is_empty() {
            return display;
        }
        let raw = &self.buffer[raw_start..];
        for t in &mut self.pending_transformers {
            if let Some(next) = t.transform(PendingTransformInput {
                kind,
                raw,
                display: &display,
            }) {
                display = next;
            }
        }
        display
    }

    fn ensure_pending_display_for(&mut self, kind: BlockKind, raw_start: usize) {
        if matches!(kind, BlockKind::CodeFence) {
            if let BlockMode::CodeFence {
                fence_char,
                fence_len,
            } = self.current_mode
            {
                if self.pending_display_cache.is_some() && self.pending_display_cache_suffix.is_some()
                {
                    return;
                }
                let raw = &self.buffer[raw_start..];
                let suffix = code_fence_suffix(raw.ends_with('\n'), fence_char, fence_len);
                let mut display = String::with_capacity(raw.len() + suffix.len());
                display.push_str(raw);
                display.push_str(&suffix);
                self.pending_display_cache = Some(display);
                self.pending_display_cache_suffix = Some(suffix);
                return;
            }
        }

        if self.pending_display_cache.is_some() {
            return;
        }
        let display = {
            let raw = &self.buffer[raw_start..];
            terminate_markdown(raw, &self.opts.terminator)
        };
        let display = self.transform_pending_display_at(kind, raw_start, display);
        self.pending_display_cache = Some(display);
        self.pending_display_cache_suffix = None;
    }

    fn try_incremental_pending_display_append(&mut self, appended: &str) -> bool {
        let Some(suffix) = self.pending_display_cache_suffix.as_ref() else {
            return false;
        };
        let Some(display) = self.pending_display_cache.as_mut() else {
            self.pending_display_cache_suffix = None;
            return false;
        };
        let BlockMode::CodeFence {
            fence_char,
            fence_len,
        } = self.current_mode
        else {
            self.pending_display_cache_suffix = None;
            self.pending_display_cache = None;
            return false;
        };

        let prev_raw_ended_with_nl = !suffix.starts_with('\n');
        let new_raw_ended_with_nl = if appended.is_empty() {
            prev_raw_ended_with_nl
        } else {
            appended.ends_with('\n')
        };

        let base_len = display.len().saturating_sub(suffix.len());
        display.truncate(base_len);
        display.push_str(appended);

        let new_suffix = code_fence_suffix(new_raw_ended_with_nl, fence_char, fence_len);
        display.push_str(&new_suffix);
        self.pending_display_cache_suffix = Some(new_suffix);
        true
    }

    fn pending_block_snapshot(&mut self) -> Option<Block> {
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            let raw = self.buffer.clone();
            if raw.is_empty() {
                return None;
            }
            let kind = BlockKind::Unknown;
            let display = self.transform_pending_display(
                kind,
                &raw,
                terminate_markdown(&raw, &self.opts.terminator),
            );
            return Some(Block {
                id: BlockId(1),
                status: BlockStatus::Pending,
                kind,
                raw,
                display: Some(display),
            });
        }

        if self.current_block_start_line >= self.lines.len() {
            return None;
        }
        let start_off = self.lines[self.current_block_start_line].start;
        if start_off >= self.buffer.len() {
            return None;
        }
        let raw = self.buffer[start_off..].to_string();
        if raw.is_empty() {
            return None;
        }
        let kind = if matches!(self.current_mode, BlockMode::Unknown) {
            let mode = self.start_mode_for_line(self.line_str(self.current_block_start_line));
            Self::kind_for_mode(&mode)
        } else {
            Self::kind_for_mode(&self.current_mode)
        };
        let mut display = terminate_markdown(&raw, &self.opts.terminator);
        display = self.transform_pending_display(kind, &raw, display);
        Some(Block {
            id: self.current_block_id,
            status: BlockStatus::Pending,
            kind,
            raw,
            display: Some(display),
        })
    }

    fn current_pending_block(&mut self) -> Option<Block> {
        if let Some(cached) = &self.pending_display_cache {
            // Fast path: pending raw still needs to be refreshed.
            if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
                let raw = self.buffer.clone();
                if raw.is_empty() {
                    return None;
                }
                return Some(Block {
                    id: BlockId(1),
                    status: BlockStatus::Pending,
                    kind: BlockKind::Unknown,
                    raw,
                    display: Some(cached.clone()),
                });
            }

            if self.current_block_start_line >= self.lines.len() {
                return None;
            }
            let start_off = self.lines[self.current_block_start_line].start;
            if start_off >= self.buffer.len() {
                return None;
            }
            let raw = self.buffer[start_off..].to_string();
            if raw.is_empty() {
                return None;
            }
            return Some(Block {
                id: self.current_block_id,
                status: BlockStatus::Pending,
                kind: Self::kind_for_mode(&self.current_mode),
                raw,
                display: Some(cached.clone()),
            });
        }

        let p = self.pending_block_snapshot();
        if let Some(p) = &p {
            if let Some(d) = &p.display {
                self.pending_display_cache = Some(d.clone());
                self.pending_display_cache_suffix = None;
            }
        }
        p
    }

    fn transform_pending_display(
        &mut self,
        kind: BlockKind,
        raw: &str,
        mut display: String,
    ) -> String {
        if self.pending_transformers.is_empty() {
            return display;
        }
        for t in &mut self.pending_transformers {
            if let Some(next) = t.transform(PendingTransformInput {
                kind,
                raw,
                display: &display,
            }) {
                display = next;
            }
        }
        display
    }

    pub fn append(&mut self, chunk: &str) -> Update {
        let mut update = Update::empty();
        let mut ctx = AppendCtx::new(Some(&mut update.committed));
        self.append_core(chunk, &mut ctx);
        update.reset = ctx.reset;
        update.invalidated = ctx.invalidated;
        update.pending = self.current_pending_block();
        update
    }

    pub fn append_ref(&mut self, chunk: &str) -> UpdateRef<'_> {
        let committed_start = self.committed.len();
        let mut ctx = AppendCtx::new(None);
        self.append_core(chunk, &mut ctx);
        let committed_start = if ctx.reset { 0 } else { committed_start };
        self.ensure_current_pending_display();
        let pending = self.current_pending_ref_readonly();
        let committed = &self.committed[committed_start..];
        UpdateRef {
            committed,
            pending,
            reset: ctx.reset,
            invalidated: ctx.invalidated,
        }
    }

    fn append_core(&mut self, chunk: &str, ctx: &mut AppendCtx<'_>) {
        if chunk.is_empty() && !self.pending_cr {
            return;
        }

        let footnotes_before = self.footnotes_detected;
        let chunk = self.normalize_newlines_cow(chunk);

        // Best-effort incremental update for code-fence pending display.
        let pending_display_kept = self.try_incremental_pending_display_append(chunk.as_ref());
        if !pending_display_kept {
            self.pending_display_cache = None;
            self.pending_display_cache_suffix = None;
        }

        if !self.footnotes_detected {
            if detect_footnotes(chunk.as_ref()) {
                self.footnotes_detected = true;
            } else {
                // Keep a small tail window to detect patterns across chunk boundaries.
                const MAX_TAIL: usize = 256;
                let chunk_prefix = take_prefix_at_char_boundary(chunk.as_ref(), MAX_TAIL);
                if !self.footnote_scan_tail.is_empty() && !chunk_prefix.is_empty() {
                    let mut combined =
                        String::with_capacity(self.footnote_scan_tail.len() + chunk_prefix.len());
                    combined.push_str(&self.footnote_scan_tail);
                    combined.push_str(chunk_prefix);
                    if detect_footnotes(&combined) {
                        self.footnotes_detected = true;
                    }
                }
                if !self.footnotes_detected {
                    update_tail(&mut self.footnote_scan_tail, chunk.as_ref(), MAX_TAIL);
                }
            }
        }

        let enter_single_block_footnotes = !footnotes_before
            && self.footnotes_detected
            && self.opts.footnotes == FootnotesMode::SingleBlock;

        self.append_to_lines(chunk.as_ref());

        if enter_single_block_footnotes {
            self.reset_for_single_block_footnotes(ctx);
            return;
        }

        // Process newly completed lines.
        while self.processed_line < self.lines.len() {
            if !self.lines[self.processed_line].has_newline {
                break;
            }
            self.process_line(self.processed_line, ctx);
            self.processed_line += 1;
        }

        // Even if the current last line has no newline yet, we may have enough information to
        // commit the previous block (eg after a blank line).
        self.process_incomplete_tail_boundary(ctx);

        self.maybe_compact_buffer();
    }

    fn reset_for_single_block_footnotes(&mut self, ctx: &mut AppendCtx<'_>) {
        ctx.reset = true;

        self.committed.clear();
        self.reference_usage_index.clear();
        self.pending_display_cache = None;
        self.pending_display_cache_suffix = None;
        self.active_boundary_plugin = None;

        // Re-start IDs so consumers can treat it as a new document.
        self.current_block_start_line = 0;
        self.current_block_id = BlockId(1);
        self.next_block_id = 2;
        self.current_mode = BlockMode::Unknown;

        // We intentionally stop line processing in this mode.
        self.processed_line = self.lines.len();
    }

    pub fn finalize(&mut self) -> Update {
        if !self.pending_cr && self.buffer.len() == self.last_finalized_buffer_len {
            return Update::empty();
        }

        let mut update = Update::empty();
        let mut ctx = AppendCtx::new(Some(&mut update.committed));

        if self.pending_cr {
            // Treat a trailing '\r' at EOF as a newline.
            self.append_to_lines("\n");
            self.pending_cr = false;
        }

        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            if !self.buffer.is_empty() {
                if self.buffer.trim().is_empty() {
                    update.pending = None;
                    return update;
                }
                let block = Block {
                    id: BlockId(1),
                    status: BlockStatus::Committed,
                    kind: BlockKind::Unknown,
                    raw: self.buffer.clone(),
                    display: None,
                };
                self.push_committed_block(block, &mut ctx);
            }
            update.pending = None;
            self.maybe_compact_buffer();
            self.last_finalized_buffer_len = self.buffer.len();
            update.invalidated = ctx.invalidated;
            return update;
        }

        if self.current_block_start_line < self.lines.len() {
            let end_line = self.lines.len() - 1;
            let start_off = self.lines[self.current_block_start_line].start;
            let end_off = self.buffer.len();
            if end_off > start_off {
                // Commit the remaining pending block.
                if matches!(self.current_mode, BlockMode::Unknown) {
                    self.current_mode =
                        self.start_mode_for_line(self.line_str(self.current_block_start_line));
                }
                let raw = self.buffer[start_off..end_off].to_string();
                if raw.trim().is_empty() {
                    update.pending = None;
                    return update;
                }
                let block = Block {
                    id: self.current_block_id,
                    status: BlockStatus::Committed,
                    kind: Self::kind_for_mode(&self.current_mode),
                    raw,
                    display: None,
                };
                self.push_committed_block(block, &mut ctx);
                // Reset to empty.
                self.current_block_start_line = end_line + 1;
            }
        }
        update.pending = None;
        self.maybe_compact_buffer();
        self.last_finalized_buffer_len = self.buffer.len();
        update.invalidated = ctx.invalidated;
        update
    }

    pub fn finalize_ref(&mut self) -> UpdateRef<'_> {
        let committed_start = self.committed.len();
        let update = self.finalize();
        let committed_start = if update.reset { 0 } else { committed_start };
        UpdateRef {
            committed: &self.committed[committed_start..],
            pending: None,
            reset: update.reset,
            invalidated: update.invalidated,
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.lines.clear();
        self.lines.push(Line {
            start: 0,
            end: 0,
            has_newline: false,
        });
        self.committed.clear();
        self.processed_line = 0;
        self.current_block_start_line = 0;
        self.current_block_id = BlockId(1);
        self.next_block_id = 2;
        self.current_mode = BlockMode::Unknown;
        self.pending_display_cache = None;
        self.pending_display_cache_suffix = None;
        for t in &mut self.pending_transformers {
            t.reset();
        }
        for p in self.boundary_plugins.iter_mut() {
            p.reset();
        }
        self.active_boundary_plugin = None;
        self.footnotes_detected = false;
        self.footnote_scan_tail.clear();
        self.pending_cr = false;
        self.last_finalized_buffer_len = 0;
        self.reference_usage_index.clear();
    }
}

impl Default for MdStream {
    fn default() -> Self {
        Self::new(Options::default())
    }
}

#[cfg(test)]
mod html_state_tests {
    use super::*;

    #[test]
    fn html_stack_tracks_section_with_nested_p() {
        let mut stack = Vec::<String>::new();
        let mut in_comment = false;
        update_html_block_state("<section>", &mut stack, &mut in_comment);
        assert_eq!(stack, vec!["section".to_string()]);
        update_html_block_state("  <p>Second block</p>", &mut stack, &mut in_comment);
        assert_eq!(stack, vec!["section".to_string()]);
        update_html_block_state("</section>", &mut stack, &mut in_comment);
        assert!(stack.is_empty());
    }
}
