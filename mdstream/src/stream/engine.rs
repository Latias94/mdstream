use super::MdStream;
use super::mode::BlockMode;
use crate::options::FootnotesMode;
use crate::types::{Block, BlockId, BlockKind, BlockStatus, Update, UpdateRef};

pub(super) struct AppendCtx<'a> {
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

    pub(super) fn extend_invalidated(&mut self, invalidated: impl IntoIterator<Item = BlockId>) {
        self.invalidated.extend(invalidated);
    }

    pub(super) fn push_committed_clone(&mut self, block: &Block) {
        if let Some(out) = self.committed_out.as_deref_mut() {
            out.push(block.clone());
        }
    }
}

impl MdStream {
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
        if chunk.is_empty() && !self.input.has_pending_cr() {
            return;
        }

        let footnotes_before = self.semantics.footnotes_detected();
        let chunk = self.input.normalize_newlines_cow(chunk);

        // Best-effort incremental update for code-fence pending display.
        let code_fence = self.current_code_fence_mode();
        let pending_display_kept = self
            .pending_display
            .try_incremental_code_fence_append(chunk.as_ref(), code_fence);
        if !pending_display_kept {
            self.pending_display.clear();
        }

        self.semantics.observe_chunk_for_footnotes(chunk.as_ref());

        let enter_single_block_footnotes = !footnotes_before
            && self.semantics.footnotes_detected()
            && self.opts.footnotes == FootnotesMode::SingleBlock;

        self.input.append_normalized(chunk.as_ref());

        if enter_single_block_footnotes {
            self.reset_for_single_block_footnotes(ctx);
            return;
        }

        while self.block_machine.processed_line < self.input.line_count() {
            if !self
                .input
                .line_has_newline(self.block_machine.processed_line)
            {
                break;
            }
            self.process_line(self.block_machine.processed_line, ctx);
            self.block_machine.processed_line += 1;
        }

        self.process_incomplete_tail_boundary(ctx);
        self.maybe_compact_buffer();
    }

    fn reset_for_single_block_footnotes(&mut self, ctx: &mut AppendCtx<'_>) {
        ctx.reset = true;

        self.committed.clear();
        self.semantics.clear_references();
        self.pending_display.clear();
        self.boundaries.clear_active();

        // Re-start IDs so consumers can treat it as a new document.
        // We intentionally stop line processing in this mode.
        self.block_machine
            .reset_for_single_block(self.input.line_count());
    }

    pub fn finalize(&mut self) -> Update {
        if !self.input.has_pending_cr() && self.input.len() == self.last_finalized_buffer_len {
            return Update::empty();
        }

        let mut update = Update::empty();
        let mut ctx = AppendCtx::new(Some(&mut update.committed));

        if self.input.has_pending_cr() {
            // Treat a trailing '\r' at EOF as a newline.
            self.input.flush_pending_cr_at_eof();
        }

        if self.opts.footnotes == FootnotesMode::SingleBlock && self.semantics.footnotes_detected()
        {
            if !self.input.is_empty() {
                if self.input.as_str().trim().is_empty() {
                    update.pending = None;
                    return update;
                }
                let block = Block {
                    id: BlockId(1),
                    status: BlockStatus::Committed,
                    kind: BlockKind::Unknown,
                    raw: self.input.as_str().to_string(),
                    display: None,
                };
                self.push_committed_block(block, &mut ctx);
            }
            update.pending = None;
            self.maybe_compact_buffer();
            self.last_finalized_buffer_len = self.input.len();
            update.invalidated = ctx.invalidated;
            return update;
        }

        if self.block_machine.current_block_start_line < self.input.line_count() {
            let end_line = self.input.line_count() - 1;
            let start_off = self
                .input
                .line_start(self.block_machine.current_block_start_line)
                .unwrap_or(self.input.len());
            let end_off = self.input.len();
            if end_off > start_off {
                if matches!(self.block_machine.current_mode, BlockMode::Unknown) {
                    self.block_machine.current_mode = self.start_mode_for_line(
                        self.line_str(self.block_machine.current_block_start_line),
                    );
                }
                let raw = self.input.as_str()[start_off..end_off].to_string();
                if raw.trim().is_empty() {
                    update.pending = None;
                    return update;
                }
                let block = Block {
                    id: self.block_machine.current_block_id,
                    status: BlockStatus::Committed,
                    kind: self.block_machine.current_mode.kind(),
                    raw,
                    display: None,
                };
                self.push_committed_block(block, &mut ctx);
                self.block_machine.current_block_start_line = end_line + 1;
            }
        }
        update.pending = None;
        self.maybe_compact_buffer();
        self.last_finalized_buffer_len = self.input.len();
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
}
