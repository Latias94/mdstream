use super::LegacyFramer;
use super::mode::BlockMode;
use crate::options::FootnotesMode;
use crate::types::{Block, BlockId, BlockKind, BlockStatus, Update};

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

impl LegacyFramer {
    pub(crate) fn append_normalized(&mut self, suffix: &str) -> Update {
        let mut update = Update::empty();
        let mut ctx = AppendCtx::new(Some(&mut update.committed));
        self.append_core(suffix, &mut ctx);
        update.reset = ctx.reset;
        update.invalidated = ctx.invalidated;
        self.ensure_current_pending_display();
        update.pending = self.current_pending_block();
        update
    }

    fn append_core(&mut self, suffix: &str, ctx: &mut AppendCtx<'_>) {
        if suffix.is_empty() {
            return;
        }

        let footnotes_before = self.semantics.footnotes_detected();

        // Best-effort incremental update for code-fence pending display.
        let code_fence = self.current_code_fence_mode();
        let pending_display_kept = self
            .pending_display
            .try_incremental_code_fence_append(suffix, code_fence);
        if !pending_display_kept {
            self.pending_display.clear();
        }

        self.semantics.observe_chunk_for_footnotes(suffix);

        let enter_single_block_footnotes = !footnotes_before
            && self.semantics.footnotes_detected()
            && self.opts.footnotes == FootnotesMode::SingleBlock;

        self.input.append_normalized(suffix);

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

    pub(crate) fn finish(&mut self, eof_suffix: &str) -> Update {
        let mut update = Update::empty();
        let mut ctx = AppendCtx::new(Some(&mut update.committed));

        if !eof_suffix.is_empty() {
            self.append_core(eof_suffix, &mut ctx);
        }

        if self.opts.footnotes == FootnotesMode::SingleBlock && self.semantics.footnotes_detected()
        {
            if !self.input.is_empty() {
                if self.input.as_str().trim().is_empty() {
                    update.pending = None;
                    update.invalidated = ctx.invalidated;
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
                    update.invalidated = ctx.invalidated;
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
        update.invalidated = ctx.invalidated;
        update
    }
}
