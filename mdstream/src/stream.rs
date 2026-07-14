mod block_machine;
mod boundary_detector;
mod builder;
mod compaction;
mod engine;
mod footnotes;
mod html;
mod input;
mod machine;
mod mode;

pub use self::builder::MdStreamBuilder;
pub(crate) use self::input::NewlineNormalizer;

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

pub(crate) struct LegacyFramer {
    opts: Options,
    input: LineBuffer,

    committed: Vec<Block>,
    block_machine: BlockMachine,

    pending_display: PendingDisplayPipeline,
    pending_transformers: PendingTransformers,
    boundaries: BoundaryRegistry,
    semantics: DocumentSemantics,
}

#[derive(Debug, Clone, Copy)]
struct PendingInfo {
    id: BlockId,
    kind: BlockKind,
    raw_start: usize,
}

impl std::fmt::Debug for LegacyFramer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LegacyFramer")
            .field("buffer_len", &self.input.len())
            .field("retained_base", &self.input.retained_base())
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
            .finish()
    }
}

impl LegacyFramer {
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
        }
    }

    pub fn push_pending_transformer<T>(&mut self, transformer: T)
    where
        T: PendingTransformer + 'static,
    {
        self.pending_transformers.push(transformer);
        self.pending_display.clear();
    }

    pub fn push_boundary_plugin<T>(&mut self, plugin: T)
    where
        T: BoundaryPlugin + 'static,
    {
        self.boundaries.push(plugin);
        self.pending_display.clear();
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

    pub fn reset(&mut self) {
        self.input.reset();
        self.committed.clear();
        self.block_machine.reset();
        self.pending_display.clear();
        self.pending_transformers.reset_all();
        self.boundaries.reset_all();
        self.semantics.reset();
    }
}

impl Default for LegacyFramer {
    fn default() -> Self {
        Self::new(Options::default())
    }
}

/// Temporary 0.3 compatibility facade over the legacy block framer.
///
/// New integrations should consume [`crate::StreamEngine`] change sets
/// directly. This facade intentionally performs only newline normalization and
/// legacy block framing until the remaining Rust and Tokio consumers migrate.
pub struct MdStream {
    normalizer: NewlineNormalizer,
    framer: LegacyFramer,
    finalized: bool,
    borrowed_update: Update,
}

impl std::fmt::Debug for MdStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MdStream")
            .field("normalizer", &self.normalizer)
            .field("framer", &self.framer)
            .field("finalized", &self.finalized)
            .finish_non_exhaustive()
    }
}

impl MdStream {
    pub fn new(options: Options) -> Self {
        Self {
            normalizer: NewlineNormalizer::default(),
            framer: LegacyFramer::new(options),
            finalized: false,
            borrowed_update: Update::empty(),
        }
    }

    pub fn builder(options: Options) -> MdStreamBuilder {
        MdStreamBuilder::new(options)
    }

    pub fn streamdown_defaults() -> Self {
        MdStreamBuilder::streamdown_defaults().build()
    }

    pub fn append(&mut self, chunk: &str) -> Update {
        self.append_legacy(chunk)
    }

    pub fn append_ref(&mut self, chunk: &str) -> UpdateRef<'_> {
        self.borrowed_update = self.append_legacy(chunk);
        self.borrowed_update_ref()
    }

    pub fn finalize(&mut self) -> Update {
        self.finish_legacy()
    }

    pub fn finalize_ref(&mut self) -> UpdateRef<'_> {
        self.borrowed_update = self.finish_legacy();
        self.borrowed_update_ref()
    }

    pub fn reset(&mut self) {
        self.normalizer = NewlineNormalizer::default();
        self.framer.reset();
        self.finalized = false;
        self.borrowed_update = Update::empty();
    }

    pub fn push_pending_transformer<T>(&mut self, transformer: T)
    where
        T: PendingTransformer + 'static,
    {
        self.framer.push_pending_transformer(transformer);
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
        self.framer.push_boundary_plugin(plugin);
    }

    pub fn with_boundary_plugin<T>(mut self, plugin: T) -> Self
    where
        T: BoundaryPlugin + 'static,
    {
        self.push_boundary_plugin(plugin);
        self
    }

    pub fn buffer(&self) -> &str {
        self.framer.buffer()
    }

    pub fn snapshot_blocks(&mut self) -> Vec<Block> {
        self.framer.snapshot_blocks()
    }

    fn append_legacy(&mut self, chunk: &str) -> Update {
        if self.finalized {
            return Update::empty();
        }
        let (normalizer, suffix) = self.normalizer.append(chunk);
        self.normalizer = normalizer;
        if suffix.is_empty() {
            Update::empty()
        } else {
            self.framer.append_normalized(&suffix)
        }
    }

    fn finish_legacy(&mut self) -> Update {
        if self.finalized {
            return Update::empty();
        }
        let (normalizer, suffix) = self.normalizer.finish();
        self.normalizer = normalizer;
        self.finalized = true;
        self.framer.finish(&suffix)
    }

    fn borrowed_update_ref(&self) -> UpdateRef<'_> {
        let pending = self
            .borrowed_update
            .pending
            .as_ref()
            .map(|block| PendingBlockRef {
                id: block.id,
                kind: block.kind,
                raw: block.raw.as_str(),
                display: block.display.as_deref(),
            });
        UpdateRef {
            committed: self.borrowed_update.committed.as_slice(),
            pending,
            reset: self.borrowed_update.reset,
            invalidated: self.borrowed_update.invalidated.clone(),
        }
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
