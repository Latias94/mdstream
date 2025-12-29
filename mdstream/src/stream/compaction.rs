use super::MdStream;
use crate::options::FootnotesMode;

impl MdStream {
    pub(super) fn maybe_compact_buffer(&mut self) {
        let Some(max) = self.opts.max_buffer_bytes else {
            return;
        };
        if self.buffer.len() <= max {
            return;
        }

        // In single-block footnote mode we must keep the entire buffer until finalize, since we
        // intentionally avoid incremental committing.
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            return;
        }

        let old_line_count = self.lines.len();
        let old_block_start_line = self.current_block_start_line;
        let old_processed_line = self.processed_line;

        let keep_from = if old_block_start_line < self.lines.len() {
            self.lines[old_block_start_line].start
        } else {
            self.buffer.len()
        };
        if keep_from == 0 {
            return;
        }
        if keep_from > self.buffer.len() {
            return;
        }

        let mut keep_from = keep_from;
        while keep_from < self.buffer.len() && !self.buffer.is_char_boundary(keep_from) {
            keep_from += 1;
        }
        if keep_from >= self.buffer.len() {
            self.buffer.clear();
        } else {
            self.buffer = self.buffer[keep_from..].to_string();
        }

        self.rebuild_lines_from_buffer();

        self.current_block_start_line = 0;
        self.processed_line = old_processed_line.saturating_sub(old_block_start_line);
        if self.processed_line > self.lines.len() {
            self.processed_line = self.lines.len();
        }

        self.pending_display_cache = None;
        self.pending_display_cache_suffix = None;
        self.last_finalized_buffer_len = self.last_finalized_buffer_len.saturating_sub(keep_from);

        // Best-effort sanity: avoid holding obviously wrong indices if something went off.
        debug_assert!(
            old_line_count == 0
                || old_block_start_line <= old_processed_line
                || old_block_start_line >= old_line_count
        );
    }
}
