use std::collections::HashSet;

use super::footnotes::{is_footnote_continuation, is_footnote_definition_start};
use super::html::{html_block_start_state, update_html_block_state};
use super::mode::BlockMode;
use super::refs::extract_reference_usages;
use super::{AppendCtx, MdStream};
use crate::boundary::BoundaryUpdate;
use crate::options::{FootnotesMode, ReferenceDefinitionsMode};
use crate::reference::extract_reference_definition_label;
use crate::syntax::facts::{
    count_double_dollars, fence_end, fence_start, is_atx_heading_start as is_heading,
    is_blank_line as is_empty_line, is_blockquote_start, is_list_continuation, is_list_item_start,
    is_list_item_start_prefix, is_thematic_break, setext_underline_char,
};
use crate::types::{Block, BlockId, BlockKind, BlockStatus};

impl MdStream {
    pub(super) fn start_mode_for_line(&self, line: &str) -> BlockMode {
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

    pub(super) fn commit_block(&mut self, end_line_inclusive: usize, ctx: &mut AppendCtx<'_>) {
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
            kind: self.current_mode.kind(),
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

    pub(super) fn push_committed_block(&mut self, block: Block, ctx: &mut AppendCtx<'_>) {
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

    pub(super) fn line_str(&self, line_index: usize) -> &str {
        self.lines[line_index].as_str(&self.buffer)
    }

    pub(super) fn process_line(&mut self, line_index: usize, ctx: &mut AppendCtx<'_>) {
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

    pub(super) fn process_incomplete_tail_boundary(&mut self, ctx: &mut AppendCtx<'_>) {
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
}
