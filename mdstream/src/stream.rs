mod block_machine;
mod boundary_detector;
mod compaction;
mod footnotes;
mod html;
mod input;
mod machine;
mod mode;

use self::block_machine::BlockMachine;
use self::input::LineBuffer;
use self::mode::BlockMode;

use crate::boundary::BoundaryPlugin;
use crate::extensions::{BoundaryRegistry, PendingTransformers};
use crate::options::{FootnotesMode, Options};
use crate::pending::{PendingDisplayPipeline, render_pending_display};
use crate::semantics::DocumentSemantics;
use crate::transform::PendingTransformer;
use crate::types::{Block, BlockId, BlockKind, BlockStatus, PendingBlockRef, Update, UpdateRef};

pub struct MdStream {
    opts: Options,
    input: LineBuffer,

    committed: Vec<Block>,
    block_machine: BlockMachine,

    pending_display: PendingDisplayPipeline,
    pending_transformers: PendingTransformers,
    boundaries: BoundaryRegistry,
    semantics: DocumentSemantics,
    last_finalized_buffer_len: usize,
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
            .field("buffer_len", &self.input.len())
            .field("lines_len", &self.input.line_count())
            .field("committed_len", &self.committed.len())
            .field("processed_line", &self.block_machine.processed_line)
            .field(
                "current_block_start_line",
                &self.block_machine.current_block_start_line,
            )
            .field("current_block_id", &self.block_machine.current_block_id)
            .field("next_block_id", &self.block_machine.next_block_id)
            .field("pending_display", &self.pending_display)
            .field("pending_transformers", &self.pending_transformers)
            .field("boundaries", &self.boundaries)
            .field("semantics", &self.semantics)
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
            input: LineBuffer::new(),
            committed: Vec::new(),
            block_machine: BlockMachine::new(),
            pending_display: PendingDisplayPipeline::default(),
            pending_transformers: PendingTransformers::default(),
            boundaries: BoundaryRegistry::default(),
            semantics: DocumentSemantics::default(),
            last_finalized_buffer_len: 0,
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
        self.pending_transformers.push(transformer);
        self.pending_display.clear();
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
        self.boundaries.push(plugin);
        self.pending_display.clear();
    }

    pub fn with_boundary_plugin<T>(mut self, plugin: T) -> Self
    where
        T: BoundaryPlugin + 'static,
    {
        self.push_boundary_plugin(plugin);
        self
    }

    pub fn buffer(&self) -> &str {
        self.input.as_str()
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
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.semantics.footnotes_detected()
        {
            if self.input.is_empty() {
                return None;
            }
            return Some(PendingInfo {
                id: BlockId(1),
                kind: BlockKind::Unknown,
                raw_start: 0,
            });
        }

        if self.block_machine.current_block_start_line >= self.input.line_count() {
            return None;
        }
        let start_off = self
            .input
            .line_start(self.block_machine.current_block_start_line)?;
        if start_off >= self.input.len() {
            return None;
        }
        if self.input.as_str()[start_off..].is_empty() {
            return None;
        }

        let kind = if matches!(self.block_machine.current_mode, BlockMode::Unknown) {
            let mode = self
                .start_mode_for_line(self.line_str(self.block_machine.current_block_start_line));
            mode.kind()
        } else {
            self.block_machine.current_mode.kind()
        };

        Some(PendingInfo {
            id: self.block_machine.current_block_id,
            kind,
            raw_start: start_off,
        })
    }

    fn ensure_current_pending_display(&mut self) {
        let Some(info) = self.current_pending_info() else {
            self.pending_display.clear();
            return;
        };
        let raw = &self.input.as_str()[info.raw_start..];
        let code_fence = self.current_code_fence_mode();
        self.pending_display.ensure_for(
            info.kind,
            raw,
            code_fence,
            &self.opts.terminator,
            self.pending_transformers.as_mut_slice(),
        );
    }

    fn current_pending_ref_readonly(&self) -> Option<PendingBlockRef<'_>> {
        let info = self.current_pending_info()?;
        let raw = &self.input.as_str()[info.raw_start..];
        Some(PendingBlockRef {
            id: info.id,
            kind: info.kind,
            raw,
            display: self.pending_display.display(),
        })
    }

    fn current_code_fence_mode(&self) -> Option<(char, usize)> {
        if let BlockMode::CodeFence {
            fence_char,
            fence_len,
        } = self.block_machine.current_mode
        {
            Some((fence_char, fence_len))
        } else {
            None
        }
    }

    fn pending_block_snapshot(&mut self) -> Option<Block> {
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.semantics.footnotes_detected()
        {
            let raw = self.input.as_str().to_string();
            if raw.is_empty() {
                return None;
            }
            let kind = BlockKind::Unknown;
            let display = render_pending_display(
                kind,
                &raw,
                &self.opts.terminator,
                self.pending_transformers.as_mut_slice(),
            );
            return Some(Block {
                id: BlockId(1),
                status: BlockStatus::Pending,
                kind,
                raw,
                display: Some(display),
            });
        }

        if self.block_machine.current_block_start_line >= self.input.line_count() {
            return None;
        }
        let start_off = self
            .input
            .line_start(self.block_machine.current_block_start_line)?;
        if start_off >= self.input.len() {
            return None;
        }
        let raw = self.input.as_str()[start_off..].to_string();
        if raw.is_empty() {
            return None;
        }
        let kind = if matches!(self.block_machine.current_mode, BlockMode::Unknown) {
            let mode = self
                .start_mode_for_line(self.line_str(self.block_machine.current_block_start_line));
            mode.kind()
        } else {
            self.block_machine.current_mode.kind()
        };
        let display = render_pending_display(
            kind,
            &raw,
            &self.opts.terminator,
            self.pending_transformers.as_mut_slice(),
        );
        Some(Block {
            id: self.block_machine.current_block_id,
            status: BlockStatus::Pending,
            kind,
            raw,
            display: Some(display),
        })
    }

    fn current_pending_block(&mut self) -> Option<Block> {
        if let Some(cached) = self.pending_display.display() {
            let info = self.current_pending_info()?;
            let raw = self.input.as_str()[info.raw_start..].to_string();
            if raw.is_empty() {
                return None;
            }
            return Some(Block {
                id: info.id,
                status: BlockStatus::Pending,
                kind: info.kind,
                raw,
                display: Some(cached.to_string()),
            });
        }

        let p = self.pending_block_snapshot();
        if let Some(p) = &p {
            if let Some(d) = &p.display {
                self.pending_display.set_owned_display(d.clone());
            }
        }
        p
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

        // Process newly completed lines.
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

        // Even if the current last line has no newline yet, we may have enough information to
        // commit the previous block (eg after a blank line).
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
                // Commit the remaining pending block.
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
                // Reset to empty.
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

    pub fn reset(&mut self) {
        self.input.reset();
        self.committed.clear();
        self.block_machine.reset();
        self.pending_display.clear();
        self.pending_transformers.reset_all();
        self.boundaries.reset_all();
        self.semantics.reset();
        self.last_finalized_buffer_len = 0;
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
