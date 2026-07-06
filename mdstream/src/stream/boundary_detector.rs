use super::footnotes::{is_footnote_continuation, is_footnote_definition_start};
use super::mode::BlockMode;
use crate::syntax::facts::{
    fence_start, is_atx_heading_start as is_heading, is_blank_line as is_empty_line,
    is_blockquote_start, is_list_continuation, is_list_item_start, is_list_item_start_prefix,
    is_thematic_break, setext_underline_char,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BoundaryDecision {
    SameBlock,
    StartNewBlock,
}

impl BoundaryDecision {
    pub(super) fn starts_new_block(self) -> bool {
        matches!(self, Self::StartNewBlock)
    }
}

#[derive(Debug, Clone)]
pub(super) struct BoundaryDetector<'a> {
    current_mode: &'a BlockMode,
    block_start_mode: BlockMode,
    current_block_start_line: usize,
    curr_has_newline: bool,
    custom_boundary_starts: bool,
}

impl<'a> BoundaryDetector<'a> {
    pub(super) fn new(
        current_mode: &'a BlockMode,
        block_start_mode: BlockMode,
        current_block_start_line: usize,
        curr_has_newline: bool,
        custom_boundary_starts: bool,
    ) -> Self {
        Self {
            current_mode,
            block_start_mode,
            current_block_start_line,
            curr_has_newline,
            custom_boundary_starts,
        }
    }

    pub(super) fn detect(
        &self,
        prev: &str,
        curr: &str,
        curr_line_index: usize,
    ) -> BoundaryDecision {
        if self.must_stay_in_current_block(curr) {
            return BoundaryDecision::SameBlock;
        }

        if is_empty_line(prev) && !is_empty_line(curr) {
            return self.after_blank_line_decision(curr);
        }

        if matches!(self.current_mode, BlockMode::Paragraph | BlockMode::Unknown)
            && setext_underline_char(curr).is_some()
            && !is_empty_line(prev)
            && self.current_block_start_line + 1 == curr_line_index
        {
            return BoundaryDecision::SameBlock;
        }

        if self.current_line_can_interrupt(prev, curr, curr_line_index) {
            return BoundaryDecision::StartNewBlock;
        }

        BoundaryDecision::SameBlock
    }

    fn must_stay_in_current_block(&self, curr: &str) -> bool {
        match self.current_mode {
            BlockMode::CodeFence { .. } | BlockMode::CustomBoundary { .. } => true,
            BlockMode::MathBlock { open_count } => open_count % 2 == 1,
            BlockMode::HtmlBlock { stack, in_comment } => *in_comment || !stack.is_empty(),
            BlockMode::FootnoteDefinition => is_empty_line(curr) || is_footnote_continuation(curr),
            _ => false,
        }
    }

    fn after_blank_line_decision(&self, curr: &str) -> BoundaryDecision {
        let in_list = matches!(self.current_mode, BlockMode::List)
            || matches!(self.block_start_mode, BlockMode::List);
        let in_blockquote = matches!(self.current_mode, BlockMode::BlockQuote)
            || matches!(self.block_start_mode, BlockMode::BlockQuote);

        if in_list && (is_list_continuation(curr) || is_list_item_start_prefix(curr)) {
            return BoundaryDecision::SameBlock;
        }
        if in_blockquote && is_blockquote_start(curr) {
            return BoundaryDecision::SameBlock;
        }

        BoundaryDecision::StartNewBlock
    }

    fn current_line_can_interrupt(&self, prev: &str, curr: &str, curr_line_index: usize) -> bool {
        if is_heading(curr) || (self.curr_has_newline && is_thematic_break(curr)) {
            return true;
        }
        if fence_start(curr).is_some() {
            return true;
        }
        if self.custom_boundary_starts {
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

        matches!(self.current_mode, BlockMode::Paragraph | BlockMode::Unknown)
            && self.curr_has_newline
            && is_table_delimiter(curr)
            && prev.contains('|')
            && curr_line_index >= 1
            && self.current_block_start_line < curr_line_index - 1
    }
}

pub(super) fn is_table_delimiter(line: &str) -> bool {
    let s = line.trim();
    if s.is_empty() {
        return false;
    }
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
