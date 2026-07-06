use std::collections::{HashMap, HashSet};

mod compaction;
mod footnotes;
mod html;
mod input;
mod machine;
mod mode;
mod refs;

use self::footnotes::detect_footnotes;
use self::input::{Line, take_prefix_at_char_boundary, update_tail};
use self::mode::BlockMode;

use crate::boundary::BoundaryPlugin;
use crate::options::{FootnotesMode, Options};
use crate::pending::terminate_markdown;
use crate::syntax::facts::code_fence_suffix;
use crate::transform::{PendingTransformInput, PendingTransformer};
use crate::types::{Block, BlockId, BlockKind, BlockStatus, PendingBlockRef, Update, UpdateRef};

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
            mode.kind()
        } else {
            self.current_mode.kind()
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
                if self.pending_display_cache.is_some()
                    && self.pending_display_cache_suffix.is_some()
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
            mode.kind()
        } else {
            self.current_mode.kind()
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
            let info = self.current_pending_info()?;
            let raw = self.buffer[info.raw_start..].to_string();
            if raw.is_empty() {
                return None;
            }
            return Some(Block {
                id: info.id,
                status: BlockStatus::Pending,
                kind: info.kind,
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
        self.ensure_current_pending_display();
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
                    kind: self.current_mode.kind(),
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
    use super::html::update_html_block_state;

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
