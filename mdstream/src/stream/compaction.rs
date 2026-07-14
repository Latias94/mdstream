use super::LegacyFramer;
use crate::options::FootnotesMode;

impl LegacyFramer {
    pub(super) fn maybe_compact_buffer(&mut self) {
        let Some(max) = self.opts.max_buffer_bytes else {
            return;
        };
        if self.input.len() <= max {
            return;
        }

        // In single-block footnote mode we must keep the entire buffer until finalize, since we
        // intentionally avoid incremental committing.
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.semantics.footnotes_detected()
        {
            return;
        }

        let old_line_count = self.input.line_count();
        let old_block_start_line = self.block_machine.current_block_start_line;
        let old_processed_line = self.block_machine.processed_line;

        let keep_from = if old_block_start_line < self.input.line_count() {
            self.input
                .line_start(old_block_start_line)
                .unwrap_or(self.input.len())
        } else {
            self.input.len()
        };
        if keep_from == 0 {
            return;
        }
        if keep_from > self.input.len() {
            return;
        }

        let compacted_from = self.input.compact_from(keep_from);
        if compacted_from == 0 {
            return;
        }

        self.block_machine.current_block_start_line = 0;
        self.block_machine.processed_line = old_processed_line.saturating_sub(old_block_start_line);
        if self.block_machine.processed_line > self.input.line_count() {
            self.block_machine.processed_line = self.input.line_count();
        }

        self.pending_display.clear();
        // Best-effort sanity: avoid holding obviously wrong indices if something went off.
        debug_assert!(
            old_line_count == 0
                || old_block_start_line <= old_processed_line
                || old_block_start_line >= old_line_count
        );
    }
}
