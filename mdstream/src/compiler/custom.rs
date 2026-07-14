use std::{collections::BTreeMap, ops::Range};

use mdstream_protocol::ProtocolLimits;

use super::CustomBlockSpec;
use crate::syntax::{
    containers::{names_match, parse_tag_name},
    facts::{fence_end, fence_start, strip_up_to_three_leading_spaces},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CustomBlockMatch<'source> {
    pub(super) spec_index: usize,
    pub(super) source: Range<usize>,
    pub(super) body: Range<usize>,
    pub(super) attributes: Option<&'source str>,
    pub(super) children: Vec<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct CustomScan<'source> {
    pub(super) blocks: Vec<CustomBlockMatch<'source>>,
    pub(super) roots: Vec<usize>,
    pub(super) pending_start: Option<usize>,
    pub(super) scan_source_bytes: usize,
    pub(super) pending: Option<PendingCustomState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CustomSyntaxError {
    AttributeName,
    AttributeValue,
    DuplicateAttribute,
    LimitExceeded {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    NumericOverflow(&'static str),
}

#[derive(Debug, Clone, Copy)]
struct OpeningTag<'a> {
    name: &'a str,
    attributes: Option<&'a str>,
    self_closing: bool,
    consumed: usize,
}

#[derive(Debug, Clone, Copy)]
struct ClosingTag<'a> {
    name: &'a str,
    consumed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingCustomLiteral {
    Fence {
        marker: char,
        length: usize,
    },
    RawText(RawTextElement),
    FixedHtml {
        kind: FixedHtmlLiteralKind,
        block: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FixedHtmlLiteralKind {
    Comment,
    Cdata,
    ProcessingInstruction,
    Declaration,
}

impl FixedHtmlLiteralKind {
    const fn closing_token(self) -> &'static str {
        match self {
            Self::Comment => "-->",
            Self::Cdata => "]]>",
            Self::ProcessingInstruction => "?>",
            Self::Declaration => ">",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingCustomState {
    TopLevel {
        literal: Option<PendingCustomLiteral>,
        tentative_line: bool,
    },
    Custom {
        spec_index: usize,
        opaque_balance: usize,
        literal: Option<PendingCustomLiteral>,
        tentative_line: bool,
    },
}

impl PendingCustomState {
    const fn literal(self) -> Option<PendingCustomLiteral> {
        match self {
            Self::TopLevel { literal, .. } => literal,
            Self::Custom { literal, .. } => literal,
        }
    }

    const fn has_tentative_line(self) -> bool {
        matches!(
            self,
            Self::TopLevel {
                tentative_line: true,
                ..
            } | Self::Custom {
                tentative_line: true,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CustomAppendScan {
    pub(super) reaches_boundary: bool,
    pub(super) source_bytes: usize,
    pub(super) pending: Option<PendingCustomState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CustomStartContext {
    DocumentStart,
    AfterBlankLine,
    AfterNonBlankLine,
}

impl CustomStartContext {
    const fn opening_allowed(self) -> bool {
        matches!(self, Self::DocumentStart | Self::AfterBlankLine)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RawTextElement {
    Pre,
    Style,
    Script,
    Textarea,
}

impl RawTextElement {
    fn from_type_one_start(candidate: &str) -> Option<Self> {
        let after_open = candidate.strip_prefix('<')?;
        for element in [Self::Pre, Self::Style, Self::Script, Self::Textarea] {
            let name = element.name();
            let Some(prefix) = after_open.get(..name.len()) else {
                continue;
            };
            if !prefix.eq_ignore_ascii_case(name) {
                continue;
            }
            match after_open.as_bytes().get(name.len()) {
                None | Some(b'>') => return Some(element),
                Some(byte) if byte.is_ascii_whitespace() => return Some(element),
                Some(_) => {}
            }
        }
        None
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Pre => "pre",
            Self::Style => "style",
            Self::Script => "script",
            Self::Textarea => "textarea",
        }
    }

    const fn closing_token(self) -> &'static str {
        match self {
            Self::Pre => "</pre>",
            Self::Style => "</style>",
            Self::Script => "</script>",
            Self::Textarea => "</textarea>",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct OpenCustom {
    block_index: usize,
    spec_index: usize,
    opaque_balance: usize,
    tentative_line: bool,
}

#[derive(Debug, Clone, Copy)]
enum CustomLine<'source> {
    Opening {
        spec_index: usize,
        opening_end: usize,
        attributes: Option<&'source str>,
    },
    Closing {
        spec_index: usize,
        closing_end: usize,
    },
}

#[cfg(test)]
pub(super) fn find_custom_blocks<'source>(
    source: &'source str,
    custom_blocks: &[CustomBlockSpec],
    limits: ProtocolLimits,
) -> Result<CustomScan<'source>, CustomSyntaxError> {
    find_custom_blocks_with_node_budget(
        source,
        custom_blocks,
        limits,
        0,
        limits.max_nodes,
        CustomStartContext::DocumentStart,
        true,
    )
}

#[cfg(test)]
pub(super) fn find_custom_blocks_with_node_limit<'source>(
    source: &'source str,
    custom_blocks: &[CustomBlockSpec],
    limits: ProtocolLimits,
    max_custom_nodes: usize,
) -> Result<CustomScan<'source>, CustomSyntaxError> {
    find_custom_blocks_with_node_budget(
        source,
        custom_blocks,
        limits,
        0,
        max_custom_nodes,
        CustomStartContext::DocumentStart,
        true,
    )
}

pub(super) fn find_custom_blocks_with_node_budget<'source>(
    source: &'source str,
    custom_blocks: &[CustomBlockSpec],
    limits: ProtocolLimits,
    baseline_nodes: usize,
    max_nodes: usize,
    start_context: CustomStartContext,
    confirm_eof: bool,
) -> Result<CustomScan<'source>, CustomSyntaxError> {
    if custom_blocks.is_empty() || source.is_empty() {
        return Ok(CustomScan::default());
    }

    let mut blocks = Vec::<CustomBlockMatch<'source>>::new();
    let mut roots = Vec::<usize>::new();
    let mut open = Vec::<OpenCustom>::new();
    let mut literal = None;
    let mut top_level_tentative_line = false;
    let mut cursor = 0usize;
    let mut previous_line_blank = start_context.opening_allowed();

    while cursor < source.len() {
        let line_end = source[cursor..]
            .find('\n')
            .map_or(source.len(), |offset| cursor + offset);
        let next_cursor = if line_end < source.len() {
            line_end + 1
        } else {
            line_end
        };
        let line_complete = line_end < source.len();
        let raw_line = source
            .get(cursor..line_end)
            .ok_or(CustomSyntaxError::NumericOverflow("custom line range"))?;
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let mut recognized_custom = false;

        if let Some(active) = literal {
            let next_literal = advance_literal_line(line, active);
            if !line_complete && !confirm_eof {
                if let Some(frame) = open.last_mut() {
                    frame.tentative_line = true;
                } else {
                    top_level_tentative_line = true;
                }
            }
            literal = next_literal;
        } else {
            if let Some(token) = parse_custom_line(line, custom_blocks, previous_line_blank) {
                match token {
                    CustomLine::Opening {
                        spec_index,
                        opening_end,
                        attributes,
                    } => {
                        if let Some(parent) = open.last_mut() {
                            if custom_blocks[parent.spec_index].is_opaque() {
                                if !line_complete && !confirm_eof {
                                    parent.tentative_line = true;
                                }
                                if parent.spec_index == spec_index {
                                    parent.opaque_balance = parent
                                        .opaque_balance
                                        .checked_add(1)
                                        .ok_or(CustomSyntaxError::NumericOverflow(
                                            "opaque custom balance",
                                        ))?;
                                } else {
                                    literal = literal_after_line(line);
                                }
                                previous_line_blank =
                                    line.bytes().all(|byte| matches!(byte, b' ' | b'\t'));
                                cursor = next_cursor;
                                continue;
                            }
                        }
                        let depth = open
                            .len()
                            .checked_add(1)
                            .ok_or(CustomSyntaxError::NumericOverflow("custom tree depth"))?;
                        check_limit("tree.depth", depth, limits.max_tree_depth)?;
                        let node_count = baseline_nodes
                            .checked_add(blocks.len())
                            .and_then(|count| count.checked_add(1))
                            .ok_or(CustomSyntaxError::NumericOverflow("custom node count"))?;
                        check_limit("nodes", node_count, max_nodes)?;

                        let parent_block = open.last().map(|parent| parent.block_index);
                        if let Some(parent_block) = parent_block {
                            let child_count =
                                blocks[parent_block].children.len().checked_add(1).ok_or(
                                    CustomSyntaxError::NumericOverflow("custom child count"),
                                )?;
                            check_limit("children", child_count, limits.max_children_per_list)?;
                        } else {
                            let root_count = roots
                                .len()
                                .checked_add(1)
                                .ok_or(CustomSyntaxError::NumericOverflow("custom root count"))?;
                            check_limit(
                                "document.roots",
                                root_count,
                                limits.max_children_per_list,
                            )?;
                        }

                        let block_index = blocks.len();
                        let opening_end = cursor
                            .checked_add(opening_end)
                            .ok_or(CustomSyntaxError::NumericOverflow("custom opening range"))?;
                        blocks.push(CustomBlockMatch {
                            spec_index,
                            source: cursor..source.len(),
                            body: opening_end..source.len(),
                            attributes,
                            children: Vec::new(),
                        });
                        if let Some(parent_block) = parent_block {
                            blocks[parent_block].children.push(block_index);
                        } else {
                            roots.push(block_index);
                        }
                        open.push(OpenCustom {
                            block_index,
                            spec_index,
                            opaque_balance: 0,
                            tentative_line: !line_complete && !confirm_eof,
                        });
                        recognized_custom = true;
                    }
                    CustomLine::Closing {
                        spec_index,
                        closing_end,
                    } if open
                        .last()
                        .is_some_and(|frame| frame.spec_index == spec_index) =>
                    {
                        if !line_complete && !confirm_eof {
                            let frame = open.last_mut().expect("matching close has an open frame");
                            frame.tentative_line = true;
                            if frame.opaque_balance == 0 {
                                let block = &mut blocks[frame.block_index];
                                block.source.end = cursor.checked_add(closing_end).ok_or(
                                    CustomSyntaxError::NumericOverflow("custom closing range"),
                                )?;
                                block.body.end = cursor;
                            }
                            previous_line_blank = false;
                            cursor = next_cursor;
                            continue;
                        }
                        if open.last().is_some_and(|frame| frame.opaque_balance > 0) {
                            open.last_mut()
                                .expect("matching close has an open frame")
                                .opaque_balance -= 1;
                            previous_line_blank =
                                line.bytes().all(|byte| matches!(byte, b' ' | b'\t'));
                            cursor = next_cursor;
                            continue;
                        }
                        let frame = open.pop().expect("matching close has an open frame");
                        let block = &mut blocks[frame.block_index];
                        block.source.end = cursor
                            .checked_add(closing_end)
                            .ok_or(CustomSyntaxError::NumericOverflow("custom closing range"))?;
                        block.body.end = cursor;
                        recognized_custom = true;
                    }
                    CustomLine::Closing { .. } => {}
                }
            }
            if !recognized_custom {
                literal = literal_after_line(line);
                if literal.is_some() && !line_complete && !confirm_eof {
                    if let Some(frame) = open.last_mut() {
                        frame.tentative_line = true;
                    } else {
                        top_level_tentative_line = true;
                    }
                }
            }
        }

        previous_line_blank = line.bytes().all(|byte| matches!(byte, b' ' | b'\t'));
        cursor = next_cursor;
    }

    let pending_start = open
        .first()
        .map(|frame| blocks[frame.block_index].source.start);
    let pending = open.last().map_or_else(
        || {
            (literal.is_some() || top_level_tentative_line).then_some(
                PendingCustomState::TopLevel {
                    literal,
                    tentative_line: top_level_tentative_line,
                },
            )
        },
        |frame| {
            Some(PendingCustomState::Custom {
                spec_index: frame.spec_index,
                opaque_balance: frame.opaque_balance,
                literal,
                tentative_line: frame.tentative_line,
            })
        },
    );
    Ok(CustomScan {
        blocks,
        roots,
        pending_start,
        scan_source_bytes: source.len(),
        pending,
    })
}

fn parse_custom_line<'source>(
    line: &'source str,
    custom_blocks: &[CustomBlockSpec],
    opening_allowed: bool,
) -> Option<CustomLine<'source>> {
    if !line.starts_with('<') {
        return None;
    }
    if let Some(opening) = parse_opening_tag(line) {
        if opening.self_closing
            || !opening_allowed
            || !line[opening.consumed..]
                .bytes()
                .all(|byte| matches!(byte, b' ' | b'\t'))
        {
            return None;
        }
        let spec_index = custom_spec_index(custom_blocks, opening.name)?;
        return Some(CustomLine::Opening {
            spec_index,
            opening_end: opening.consumed,
            attributes: opening.attributes,
        });
    }
    let closing = parse_closing_tag(line)?;
    if !line[closing.consumed..]
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
    {
        return None;
    }
    Some(CustomLine::Closing {
        spec_index: custom_spec_index(custom_blocks, closing.name)?,
        closing_end: closing.consumed,
    })
}

fn custom_spec_index(custom_blocks: &[CustomBlockSpec], name: &str) -> Option<usize> {
    custom_blocks
        .iter()
        .position(|spec| names_match(name, spec.name(), spec.is_case_insensitive()))
}

fn literal_after_line(line: &str) -> Option<PendingCustomLiteral> {
    if strip_up_to_three_leading_spaces(line).starts_with([' ', '\t']) {
        return None;
    }
    if let Some((marker, length)) = custom_fence_start(line) {
        return Some(PendingCustomLiteral::Fence { marker, length });
    }
    scan_html_literals(line, 0)
}

fn custom_fence_start(line: &str) -> Option<(char, usize)> {
    let (marker, length) = fence_start(line)?;
    if marker == '`'
        && strip_up_to_three_leading_spaces(line)
            .get(length..)?
            .contains('`')
    {
        return None;
    }
    Some((marker, length))
}

fn advance_literal_line(line: &str, literal: PendingCustomLiteral) -> Option<PendingCustomLiteral> {
    match literal {
        PendingCustomLiteral::Fence { marker, length } => (!fence_end(line, marker, length))
            .then_some(PendingCustomLiteral::Fence { marker, length }),
        PendingCustomLiteral::RawText(element) => (!line.contains(element.closing_token()))
            .then_some(PendingCustomLiteral::RawText(element)),
        PendingCustomLiteral::FixedHtml { kind, block } => {
            continue_fixed_literal(line, kind, block)
        }
    }
}

fn continue_fixed_literal(
    line: &str,
    kind: FixedHtmlLiteralKind,
    block: bool,
) -> Option<PendingCustomLiteral> {
    let pending = PendingCustomLiteral::FixedHtml { kind, block };
    let closing = kind.closing_token();
    line.find(closing).map_or(Some(pending), |offset| {
        if block {
            None
        } else {
            offset
                .checked_add(closing.len())
                .and_then(|end| scan_html_literals(line, end))
        }
    })
}

fn scan_html_literals(line: &str, mut cursor: usize) -> Option<PendingCustomLiteral> {
    let protected_code_spans = code_span_ranges(line);
    let mut protected_index = 0usize;
    while cursor < line.len() {
        let token_start =
            next_unprotected_html_start(line, cursor, &protected_code_spans, &mut protected_index)?;
        let candidate = line.get(token_start..)?;
        let (opening_len, kind) = if candidate.starts_with("<!--") {
            (4, FixedHtmlLiteralKind::Comment)
        } else if candidate.starts_with("<![CDATA[") {
            (9, FixedHtmlLiteralKind::Cdata)
        } else if candidate.starts_with("<?") {
            (2, FixedHtmlLiteralKind::ProcessingInstruction)
        } else if candidate.as_bytes().get(1) == Some(&b'!')
            && candidate
                .as_bytes()
                .get(2)
                .is_some_and(u8::is_ascii_alphabetic)
        {
            (2, FixedHtmlLiteralKind::Declaration)
        } else if let Some(element) = RawTextElement::from_type_one_start(candidate) {
            if starts_raw_text_html_block(line, token_start) {
                return (!candidate.contains(element.closing_token()))
                    .then_some(PendingCustomLiteral::RawText(element));
            }
            cursor = parse_opening_tag(candidate).map_or_else(
                || token_start.checked_add(1),
                |opening| token_start.checked_add(opening.consumed.max(1)),
            )?;
            continue;
        } else if let Some(opening) = parse_opening_tag(candidate) {
            cursor = token_start.checked_add(opening.consumed.max(1))?;
            continue;
        } else {
            cursor = token_start.checked_add(1)?;
            continue;
        };

        let block = starts_html_block(line, token_start);
        let closing = kind.closing_token();
        let search_start = token_start.checked_add(opening_len)?;
        if let Some(offset) = line.get(search_start..)?.find(closing) {
            if block {
                return None;
            }
            cursor = search_start
                .checked_add(offset)?
                .checked_add(closing.len())?;
            continue;
        }
        return Some(PendingCustomLiteral::FixedHtml { kind, block });
    }
    None
}

fn starts_raw_text_html_block(line: &str, token_start: usize) -> bool {
    starts_html_block(line, token_start)
}

fn starts_html_block(line: &str, token_start: usize) -> bool {
    line.get(..token_start)
        .is_some_and(|prefix| prefix.len() <= 3 && prefix.bytes().all(|byte| byte == b' '))
}

fn next_unprotected_html_start(
    line: &str,
    mut cursor: usize,
    protected_code_spans: &[Range<usize>],
    protected_index: &mut usize,
) -> Option<usize> {
    while cursor < line.len() {
        let html_start = cursor.checked_add(line.get(cursor..)?.find('<')?)?;
        while protected_code_spans
            .get(*protected_index)
            .is_some_and(|range| range.end <= html_start)
        {
            *protected_index = (*protected_index).checked_add(1)?;
        }
        if protected_code_spans
            .get(*protected_index)
            .is_some_and(|range| range.start <= html_start)
        {
            let range = &protected_code_spans[*protected_index];
            cursor = range.end;
            *protected_index = (*protected_index).checked_add(1)?;
            continue;
        }
        if is_backslash_escaped(line, html_start) {
            cursor = html_start.checked_add(1)?;
            continue;
        }
        return Some(html_start);
    }
    None
}

#[derive(Debug, Clone, Copy)]
struct BacktickRun {
    start: usize,
    end: usize,
    preceded_by_backslash: bool,
}

fn code_span_ranges(line: &str) -> Vec<Range<usize>> {
    let mut runs = Vec::<BacktickRun>::new();
    let mut cursor = 0usize;
    while cursor < line.len() {
        let Some(offset) = line.get(cursor..).and_then(|tail| tail.find('`')) else {
            break;
        };
        let Some(start) = cursor.checked_add(offset) else {
            break;
        };
        let Some(end) = backtick_run_end(line, start) else {
            break;
        };
        runs.push(BacktickRun {
            start,
            end,
            preceded_by_backslash: is_backslash_escaped(line, start),
        });
        cursor = end;
    }
    let max_length = runs
        .iter()
        .map(|run| run.end.saturating_sub(run.start))
        .max()
        .unwrap_or(0);
    let mut next_by_length = vec![None; max_length.saturating_add(1)];
    let mut closing_for_open = vec![None; runs.len()];
    for (index, run) in runs.iter().enumerate().rev() {
        let raw_length = run.end.saturating_sub(run.start);
        let opening_length = raw_length.saturating_sub(usize::from(run.preceded_by_backslash));
        if opening_length > 0 {
            closing_for_open[index] = next_by_length[opening_length];
        }
        next_by_length[raw_length] = Some(index);
    }

    let mut ranges = Vec::new();
    let mut index = 0usize;
    while index < runs.len() {
        let Some(closing_index) = closing_for_open[index] else {
            index += 1;
            continue;
        };
        let opening_start = runs[index]
            .start
            .saturating_add(usize::from(runs[index].preceded_by_backslash));
        ranges.push(opening_start..runs[closing_index].end);
        index = closing_index.saturating_add(1);
    }
    ranges
}

fn backtick_run_end(line: &str, start: usize) -> Option<usize> {
    let bytes = line.as_bytes();
    if bytes.get(start) != Some(&b'`') {
        return None;
    }
    let mut cursor = start;
    while bytes.get(cursor) == Some(&b'`') {
        cursor = cursor.checked_add(1)?;
    }
    Some(cursor)
}

fn is_backslash_escaped(line: &str, token_start: usize) -> bool {
    line.as_bytes()[..token_start]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn parse_opening_tag(input: &str) -> Option<OpeningTag<'_>> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'<') || matches!(bytes.get(1), Some(b'/') | Some(b'!') | Some(b'?'))
    {
        return None;
    }
    let name_start = 1;
    let (name, _) = parse_tag_name(input.get(name_start..)?)?;
    let name_end = name_start + name.len();
    if bytes
        .get(name_end)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(*byte, b'/' | b'>'))
    {
        return None;
    }

    let mut cursor = name_end;
    let mut quote = None;
    let closing = loop {
        let byte = *bytes.get(cursor)?;
        match quote {
            Some(expected) if byte == expected => quote = None,
            Some(_) => {}
            None if matches!(byte, b'\'' | b'"') => quote = Some(byte),
            None if byte == b'>' => break cursor,
            None if byte == b'<' => return None,
            None => {}
        }
        cursor += 1;
    };
    if quote.is_some() {
        return None;
    }

    let tail = &input[name_end..closing];
    let self_closing_marker = self_closing_marker_start(tail);
    let attributes = self_closing_marker.map_or(tail, |marker| &tail[..marker]);
    let attributes = attributes.trim();
    Some(OpeningTag {
        name: &input[name_start..name_end],
        attributes: (!attributes.is_empty()).then_some(attributes),
        self_closing: self_closing_marker.is_some(),
        consumed: closing + 1,
    })
}

#[derive(Debug, Clone, Copy)]
enum AttributeLexState {
    Between,
    Name,
    AfterName,
    BeforeValue,
    QuotedValue(u8),
    AfterQuotedValue,
    UnquotedValue,
}

fn self_closing_marker_start(tail: &str) -> Option<usize> {
    let bytes = tail.as_bytes();
    let marker = bytes.iter().rposition(|byte| !byte.is_ascii_whitespace())?;
    if bytes[marker] != b'/' {
        return None;
    }

    let mut state = AttributeLexState::Between;
    for byte in bytes[..marker].iter().copied() {
        state = match state {
            AttributeLexState::Between if byte.is_ascii_whitespace() => AttributeLexState::Between,
            AttributeLexState::Between if is_attribute_name_byte(byte) => AttributeLexState::Name,
            AttributeLexState::Name if is_attribute_name_byte(byte) => AttributeLexState::Name,
            AttributeLexState::Name if byte.is_ascii_whitespace() => AttributeLexState::AfterName,
            AttributeLexState::Name if byte == b'=' => AttributeLexState::BeforeValue,
            AttributeLexState::AfterName if byte.is_ascii_whitespace() => {
                AttributeLexState::AfterName
            }
            AttributeLexState::AfterName if byte == b'=' => AttributeLexState::BeforeValue,
            AttributeLexState::AfterName if is_attribute_name_byte(byte) => AttributeLexState::Name,
            AttributeLexState::BeforeValue if byte.is_ascii_whitespace() => {
                AttributeLexState::BeforeValue
            }
            AttributeLexState::BeforeValue if matches!(byte, b'\'' | b'"') => {
                AttributeLexState::QuotedValue(byte)
            }
            AttributeLexState::BeforeValue if is_unquoted_attribute_value_byte(byte) => {
                AttributeLexState::UnquotedValue
            }
            AttributeLexState::QuotedValue(quote) if byte == quote => {
                AttributeLexState::AfterQuotedValue
            }
            AttributeLexState::QuotedValue(quote) => AttributeLexState::QuotedValue(quote),
            AttributeLexState::AfterQuotedValue if byte.is_ascii_whitespace() => {
                AttributeLexState::Between
            }
            AttributeLexState::UnquotedValue if byte.is_ascii_whitespace() => {
                AttributeLexState::Between
            }
            AttributeLexState::UnquotedValue if is_unquoted_attribute_value_byte(byte) => {
                AttributeLexState::UnquotedValue
            }
            _ => return None,
        };
    }

    matches!(
        state,
        AttributeLexState::Between
            | AttributeLexState::Name
            | AttributeLexState::AfterName
            | AttributeLexState::AfterQuotedValue
    )
    .then_some(marker)
}

fn parse_closing_tag(input: &str) -> Option<ClosingTag<'_>> {
    let bytes = input.as_bytes();
    if !input.starts_with("</") || !bytes.get(2).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let name_start = 2;
    let (name, _) = parse_tag_name(input.get(name_start..)?)?;
    let name_end = name_start + name.len();
    let mut cursor = name_end;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'>') {
        return None;
    }
    Some(ClosingTag {
        name: &input[name_start..name_end],
        consumed: cursor + 1,
    })
}

pub(super) fn append_reaches_custom_boundary(
    prefix: &str,
    suffix: &str,
    custom_blocks: &[CustomBlockSpec],
    pending: Option<PendingCustomState>,
) -> CustomAppendScan {
    if custom_blocks.is_empty() {
        return CustomAppendScan {
            reaches_boundary: false,
            source_bytes: 0,
            pending,
        };
    }
    if !suffix.as_bytes().contains(&b'\n') {
        return CustomAppendScan {
            reaches_boundary: false,
            source_bytes: suffix.len(),
            pending,
        };
    }
    if pending.is_some_and(PendingCustomState::has_tentative_line) {
        return CustomAppendScan {
            reaches_boundary: true,
            source_bytes: suffix.len(),
            pending,
        };
    }

    let tail = prefix.rsplit_once('\n').map_or(prefix, |(_, tail)| tail);
    let pending_literal = pending.and_then(PendingCustomState::literal);
    let mut appended = String::with_capacity(tail.len().saturating_add(suffix.len()));
    appended.push_str(tail);
    appended.push_str(suffix);
    let scan = appended.as_str();
    let mut previous_line_blank = previous_physical_line_is_blank(prefix, tail.len());
    let source_bytes = suffix.len().saturating_add(tail.len());

    let mut next_pending = pending;
    let mut literal = pending_literal;
    for complete_line in scan.split_inclusive('\n') {
        if !complete_line.ends_with('\n') {
            break;
        }
        let line = complete_line
            .strip_suffix('\n')
            .expect("complete line has a line ending")
            .strip_suffix('\r')
            .unwrap_or_else(|| {
                complete_line
                    .strip_suffix('\n')
                    .expect("checked line ending")
            });

        if let Some(active) = literal {
            let next = advance_literal_line(line, active);
            match next_pending {
                Some(PendingCustomState::TopLevel { .. }) => {
                    literal = next;
                    next_pending = next.map(|literal| PendingCustomState::TopLevel {
                        literal: Some(literal),
                        tentative_line: false,
                    });
                }
                Some(PendingCustomState::Custom { spec_index, .. })
                    if custom_blocks[spec_index].is_opaque() =>
                {
                    literal = next;
                    let Some(PendingCustomState::Custom {
                        literal: persisted_literal,
                        ..
                    }) = next_pending.as_mut()
                    else {
                        unreachable!("opaque custom state remains pending");
                    };
                    *persisted_literal = next;
                }
                Some(PendingCustomState::Custom { .. }) => {
                    if next != Some(active) {
                        return CustomAppendScan {
                            reaches_boundary: true,
                            source_bytes,
                            pending: next_pending,
                        };
                    }
                    literal = next;
                }
                None => unreachable!("a pending literal has a persisted owner"),
            }
        } else if let Some(PendingCustomState::Custom {
            spec_index,
            opaque_balance,
            ..
        }) = next_pending
        {
            let spec = &custom_blocks[spec_index];
            if spec.is_opaque() {
                match parse_custom_line(line, custom_blocks, previous_line_blank) {
                    Some(CustomLine::Opening {
                        spec_index: opening_spec,
                        ..
                    }) if opening_spec == spec_index => {
                        let Some(balance) = opaque_balance.checked_add(1) else {
                            return CustomAppendScan {
                                reaches_boundary: true,
                                source_bytes,
                                pending: next_pending,
                            };
                        };
                        let Some(PendingCustomState::Custom { opaque_balance, .. }) =
                            next_pending.as_mut()
                        else {
                            unreachable!("opaque custom state remains pending");
                        };
                        *opaque_balance = balance;
                    }
                    Some(CustomLine::Closing {
                        spec_index: closing_spec,
                        ..
                    }) if closing_spec == spec_index && opaque_balance == 0 => {
                        return CustomAppendScan {
                            reaches_boundary: true,
                            source_bytes,
                            pending: next_pending,
                        };
                    }
                    Some(CustomLine::Closing {
                        spec_index: closing_spec,
                        ..
                    }) if closing_spec == spec_index => {
                        let Some(PendingCustomState::Custom { opaque_balance, .. }) =
                            next_pending.as_mut()
                        else {
                            unreachable!("opaque custom state remains pending");
                        };
                        *opaque_balance -= 1;
                    }
                    _ => {
                        if let Some(next_literal) = literal_after_line(line) {
                            literal = Some(next_literal);
                            let Some(PendingCustomState::Custom {
                                literal: persisted_literal,
                                ..
                            }) = next_pending.as_mut()
                            else {
                                unreachable!("opaque custom state remains pending");
                            };
                            *persisted_literal = literal;
                        }
                    }
                }
                previous_line_blank = line.bytes().all(|byte| matches!(byte, b' ' | b'\t'));
                continue;
            }
            match parse_custom_line(line, custom_blocks, previous_line_blank) {
                Some(CustomLine::Opening {
                    spec_index: opening_spec,
                    ..
                }) if !spec.is_opaque() || opening_spec == spec_index => {
                    return CustomAppendScan {
                        reaches_boundary: true,
                        source_bytes,
                        pending: next_pending,
                    };
                }
                Some(CustomLine::Closing {
                    spec_index: closing_spec,
                    ..
                }) if closing_spec == spec_index => {
                    return CustomAppendScan {
                        reaches_boundary: true,
                        source_bytes,
                        pending: next_pending,
                    };
                }
                _ => {}
            }
            if literal_after_line(line).is_some() {
                return CustomAppendScan {
                    reaches_boundary: true,
                    source_bytes,
                    pending: next_pending,
                };
            }
        } else if matches!(
            parse_custom_line(line, custom_blocks, previous_line_blank),
            Some(CustomLine::Opening { .. })
        ) {
            return CustomAppendScan {
                reaches_boundary: true,
                source_bytes,
                pending: next_pending,
            };
        }

        previous_line_blank = line.bytes().all(|byte| matches!(byte, b' ' | b'\t'));
    }
    CustomAppendScan {
        reaches_boundary: false,
        source_bytes,
        pending: next_pending,
    }
}

fn previous_physical_line_is_blank(prefix: &str, tail_len: usize) -> bool {
    let tail_start = prefix.len().saturating_sub(tail_len);
    if tail_start == 0 {
        return true;
    }
    let before_tail = &prefix[..tail_start - 1];
    let previous = before_tail
        .rsplit_once('\n')
        .map_or(before_tail, |(_, line)| line)
        .strip_suffix('\r')
        .unwrap_or_else(|| {
            before_tail
                .rsplit_once('\n')
                .map_or(before_tail, |(_, line)| line)
        });
    previous.bytes().all(|byte| matches!(byte, b' ' | b'\t'))
}

pub(super) fn parse_custom_attributes(
    raw: Option<&str>,
    spec: &CustomBlockSpec,
    limits: ProtocolLimits,
) -> Result<BTreeMap<String, String>, CustomSyntaxError> {
    let mut metadata_bytes = spec
        .namespace()
        .len()
        .checked_add(spec.name().len())
        .ok_or(CustomSyntaxError::NumericOverflow("custom node metadata"))?;
    check_limit(
        "node.metadata",
        metadata_bytes,
        limits.max_node_metadata_bytes,
    )?;
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let bytes = raw.as_bytes();
    let mut cursor = 0;
    let mut parsed_attributes = Vec::new();

    while cursor < bytes.len() {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if cursor == bytes.len() {
            break;
        }
        let name_start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| is_attribute_name_byte(*byte))
        {
            cursor += 1;
        }
        if name_start == cursor {
            return Err(CustomSyntaxError::AttributeName);
        }
        let name = &raw[name_start..cursor];
        check_limit(
            "custom.attribute.key",
            name.len(),
            limits.max_metadata_value_bytes,
        )?;
        let attribute_count = parsed_attributes
            .len()
            .checked_add(1)
            .ok_or(CustomSyntaxError::NumericOverflow("custom attribute count"))?;
        check_limit(
            "custom.attributes",
            attribute_count,
            limits.max_attributes_per_node,
        )?;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }

        let value = if bytes.get(cursor) == Some(&b'=') {
            cursor += 1;
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            let Some(first) = bytes.get(cursor).copied() else {
                return Err(CustomSyntaxError::AttributeValue);
            };
            if matches!(first, b'\'' | b'"') {
                cursor += 1;
                let value_start = cursor;
                while bytes.get(cursor).is_some_and(|byte| *byte != first) {
                    cursor += 1;
                }
                let Some(_) = bytes.get(cursor) else {
                    return Err(CustomSyntaxError::AttributeValue);
                };
                let value = &raw[value_start..cursor];
                cursor += 1;
                value
            } else {
                let value_start = cursor;
                while bytes
                    .get(cursor)
                    .is_some_and(|byte| is_unquoted_attribute_value_byte(*byte))
                {
                    cursor += 1;
                }
                if value_start == cursor {
                    return Err(CustomSyntaxError::AttributeValue);
                }
                &raw[value_start..cursor]
            }
        } else {
            "true"
        };

        check_limit(
            "custom.attribute.value",
            value.len(),
            limits.max_metadata_value_bytes,
        )?;
        metadata_bytes = metadata_bytes
            .checked_add(name.len())
            .and_then(|bytes| bytes.checked_add(value.len()))
            .ok_or(CustomSyntaxError::NumericOverflow("custom node metadata"))?;
        check_limit(
            "node.metadata",
            metadata_bytes,
            limits.max_node_metadata_bytes,
        )?;

        parsed_attributes.push((name, value));
    }

    parsed_attributes.sort_unstable_by_key(|(name, _)| *name);
    if parsed_attributes
        .windows(2)
        .any(|attributes| attributes[0].0 == attributes[1].0)
    {
        return Err(CustomSyntaxError::DuplicateAttribute);
    }

    Ok(parsed_attributes
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect())
}

fn check_limit(field: &'static str, actual: usize, limit: usize) -> Result<(), CustomSyntaxError> {
    if actual > limit {
        Err(CustomSyntaxError::LimitExceeded {
            field,
            limit,
            actual,
        })
    } else {
        Ok(())
    }
}

fn is_attribute_name_byte(byte: u8) -> bool {
    !byte.is_ascii_whitespace() && !matches!(byte, 0 | b'"' | b'\'' | b'>' | b'/' | b'=' | b'<')
}

fn is_unquoted_attribute_value_byte(byte: u8) -> bool {
    !byte.is_ascii_whitespace() && !matches!(byte, b'"' | b'\'' | b'=' | b'<' | b'>' | b'`')
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn spec() -> CustomBlockSpec {
        CustomBlockSpec::try_new("app.custom/1", "thinking").unwrap()
    }

    fn spec_with_opaque(opaque: bool) -> CustomBlockSpec {
        spec().opaque(opaque)
    }

    fn parse_test_attributes(
        source: &str,
        limits: ProtocolLimits,
    ) -> Result<BTreeMap<String, String>, CustomSyntaxError> {
        let opening = parse_opening_tag(source).expect("opening tag must be recognized");
        parse_custom_attributes(opening.attributes, &spec(), limits)
    }

    #[test]
    fn quoted_delimiters_empty_values_and_nested_tags_are_lossless() {
        let source = concat!(
            "<thinking title=\"a > b\" empty=\"\">\n",
            "outer\n\n",
            "<thinking>\nx\n</thinking>\n",
            "</thinking>",
        );
        let scan = find_custom_blocks(source, &[spec()], ProtocolLimits::default()).unwrap();

        assert_eq!(scan.blocks.len(), 1);
        assert_eq!(scan.blocks[0].source, 0..source.len());
        let attributes = parse_custom_attributes(
            scan.blocks[0].attributes,
            &spec(),
            ProtocolLimits::default(),
        )
        .unwrap();
        assert_eq!(attributes["title"], "a > b");
        assert_eq!(attributes["empty"], "");
        assert_eq!(scan.pending_start, None);
    }

    #[test]
    fn self_closing_slash_must_be_a_lexically_independent_marker() {
        for source in [
            "<thinking path=/foo/>\nbody\n</thinking>",
            "<thinking path=/foo/ >\nbody\n</thinking>",
        ] {
            let scan = find_custom_blocks(source, &[spec()], ProtocolLimits::default()).unwrap();

            assert_eq!(scan.blocks.len(), 1, "source={source:?}");
            let attributes = parse_custom_attributes(
                scan.blocks[0].attributes,
                &spec(),
                ProtocolLimits::default(),
            )
            .unwrap();
            assert_eq!(attributes["path"], "/foo/");
        }

        for (source, expected) in [
            ("<thinking path=/foo/ />", Some(("path", "/foo/"))),
            ("<thinking path=\"/foo/\"/>", Some(("path", "/foo/"))),
            ("<thinking empty=\"\" />", Some(("empty", ""))),
            ("<thinking/>", None),
        ] {
            let opening = parse_opening_tag(source).expect("opening tag must be recognized");
            assert!(opening.self_closing, "source={source:?}");
            let attributes =
                parse_custom_attributes(opening.attributes, &spec(), ProtocolLimits::default())
                    .unwrap();
            match expected {
                Some((name, value)) => assert_eq!(attributes[name], value),
                None => assert!(attributes.is_empty()),
            }
        }
    }

    #[test]
    fn attribute_count_limit_reports_the_first_excess_attribute() {
        let limits = ProtocolLimits {
            max_attributes_per_node: 2,
            ..ProtocolLimits::default()
        };

        let exact = parse_test_attributes("<thinking a=1 b=2>", limits).unwrap();
        assert_eq!(exact.len(), 2);
        assert_eq!(
            parse_test_attributes("<thinking a=1 b=2 c=3 d=4>", limits),
            Err(CustomSyntaxError::LimitExceeded {
                field: "custom.attributes",
                limit: 2,
                actual: 3,
            })
        );
    }

    #[test]
    fn attribute_key_limit_is_checked_at_the_byte_boundary() {
        let limits = ProtocolLimits {
            max_metadata_value_bytes: 3,
            ..ProtocolLimits::default()
        };

        let exact = parse_test_attributes("<thinking abc=1>", limits).unwrap();
        assert_eq!(exact.get("abc").map(String::as_str), Some("1"));
        assert_eq!(
            parse_test_attributes("<thinking abcd=1>", limits),
            Err(CustomSyntaxError::LimitExceeded {
                field: "custom.attribute.key",
                limit: 3,
                actual: 4,
            })
        );
    }

    #[test]
    fn quoted_and_unquoted_attribute_values_share_the_same_byte_limit() {
        let limits = ProtocolLimits {
            max_metadata_value_bytes: 3,
            ..ProtocolLimits::default()
        };

        for source in ["<thinking x=abc>", "<thinking x=\"abc\">"] {
            let exact = parse_test_attributes(source, limits).unwrap();
            assert_eq!(exact.get("x").map(String::as_str), Some("abc"));
        }
        for source in ["<thinking x=abcd>", "<thinking x=\"abcd\">"] {
            assert_eq!(
                parse_test_attributes(source, limits),
                Err(CustomSyntaxError::LimitExceeded {
                    field: "custom.attribute.value",
                    limit: 3,
                    actual: 4,
                }),
                "source={source:?}",
            );
        }
    }

    #[test]
    fn node_metadata_limit_includes_static_and_boolean_attribute_bytes() {
        let spec = spec();
        let exact_bytes = spec.namespace().len() + spec.name().len() + "x".len() + "true".len();
        let exact_limits = ProtocolLimits {
            max_node_metadata_bytes: exact_bytes,
            ..ProtocolLimits::default()
        };

        let exact = parse_test_attributes("<thinking x>", exact_limits).unwrap();
        assert_eq!(exact.get("x").map(String::as_str), Some("true"));

        let exceeded_limits = ProtocolLimits {
            max_node_metadata_bytes: exact_bytes - 1,
            ..ProtocolLimits::default()
        };
        assert_eq!(
            parse_test_attributes("<thinking x>", exceeded_limits),
            Err(CustomSyntaxError::LimitExceeded {
                field: "node.metadata",
                limit: exact_bytes - 1,
                actual: exact_bytes,
            })
        );
    }

    #[test]
    fn fenced_tags_are_not_custom_blocks() {
        let source = "```html\n<thinking>\nfirst\n\nsecond\n</thinking>\n```";
        assert!(
            find_custom_blocks(source, &[spec()], ProtocolLimits::default())
                .unwrap()
                .blocks
                .is_empty()
        );
    }

    #[test]
    fn backticks_in_fence_info_do_not_open_a_commonmark_fence() {
        let source = "<thinking>\n``` info`\n</thinking>";
        let closer_start = source.rfind("</thinking>").unwrap();

        for opaque in [true, false] {
            let scan = find_custom_blocks(
                source,
                &[spec_with_opaque(opaque)],
                ProtocolLimits::default(),
            )
            .unwrap();
            assert_eq!(scan.blocks.len(), 1, "opaque={opaque}");
            assert_eq!(scan.blocks[0].body.end, closer_start, "opaque={opaque}");
            assert_eq!(scan.pending_start, None, "opaque={opaque}");
        }
    }

    #[test]
    fn markdown_code_literals_hide_custom_delimiters_for_all_body_modes() {
        let source = concat!(
            "<thinking>\n",
            "`</thinking>`\n\n",
            "    </thinking>\n\n",
            "```html\n",
            "<thinking>\n",
            "</thinking>\n",
            "```\n",
            "</thinking>",
        );

        for opaque in [true, false] {
            let scan = find_custom_blocks(
                source,
                &[spec_with_opaque(opaque)],
                ProtocolLimits::default(),
            )
            .unwrap();

            assert_eq!(scan.blocks.len(), 1, "opaque={opaque}");
            assert_eq!(scan.blocks[0].source, 0..source.len(), "opaque={opaque}");
        }
    }

    #[test]
    fn raw_html_literals_hide_inner_delimiters_but_not_the_outer_closer() {
        let source = concat!(
            "<thinking>\n",
            "<script>x </thinking></script>\n",
            "<style>x </thinking></style>\n",
            "<pre>x </thinking></pre>\n",
            "<textarea>x </thinking></textarea>\n",
            "<!-- </thinking> -->\n",
            "<?instruction </thinking> ?>\n",
            "<![CDATA[ </thinking> ]]>\n",
            "<!DOCTYPE </thinking>>\n",
            "</thinking>",
        );

        for opaque in [true, false] {
            let scan = find_custom_blocks(
                source,
                &[spec_with_opaque(opaque)],
                ProtocolLimits::default(),
            )
            .unwrap();

            assert_eq!(scan.blocks.len(), 1, "opaque={opaque}");
            assert_eq!(scan.blocks[0].source, 0..source.len(), "opaque={opaque}");
            assert_eq!(scan.pending_start, None, "opaque={opaque}");
        }
    }

    #[test]
    fn registered_raw_text_names_still_shield_an_opaque_parent() {
        let source = concat!(
            "<thinking>\n\n",
            "<script>\n",
            "</thinking>\n",
            "</script>\n",
            "</thinking>",
        );
        let specs = [
            spec(),
            CustomBlockSpec::try_new("app.script/1", "script").unwrap(),
        ];
        let scan = find_custom_blocks(source, &specs, ProtocolLimits::default()).unwrap();

        assert_eq!(scan.blocks.len(), 1);
        assert_eq!(scan.blocks[0].spec_index, 0);
        assert_eq!(scan.blocks[0].source, 0..source.len());
        assert_eq!(
            scan.blocks[0].body.end,
            source.rfind("</thinking>").unwrap()
        );
        assert_eq!(scan.pending_start, None);
    }

    #[test]
    fn markdown_escaped_and_code_span_html_do_not_hide_the_outer_custom_closer() {
        for protected_markdown in ["`<script>`", r"\<script>"] {
            let source = format!("<thinking>\n{protected_markdown}\n</thinking>");
            let closer_start = source.rfind("</thinking>").unwrap();

            for opaque in [true, false] {
                let scan = find_custom_blocks(
                    &source,
                    &[spec_with_opaque(opaque)],
                    ProtocolLimits::default(),
                )
                .unwrap();

                assert_eq!(scan.blocks.len(), 1, "opaque={opaque}, source={source:?}");
                assert_eq!(
                    scan.blocks[0].body.end, closer_start,
                    "opaque={opaque}, source={source:?}",
                );
                assert_eq!(
                    scan.pending_start, None,
                    "opaque={opaque}, source={source:?}",
                );
            }
        }
    }

    #[test]
    fn dense_unmatched_backtick_runs_use_one_precomputed_index() {
        let mut line = String::new();
        for length in 1..=768 {
            line.extend(std::iter::repeat_n('`', length));
            line.push('x');
        }
        let html_start = line.len();
        line.push_str("<script>");

        let protected = code_span_ranges(&line);
        assert!(protected.is_empty());
        let mut protected_index = 0;
        assert_eq!(
            next_unprotected_html_start(&line, 0, &protected, &mut protected_index),
            Some(html_start),
        );
    }

    #[test]
    fn raw_text_html_requires_an_unescaped_commonmark_block_start() {
        for protected in [
            "`<script>`",
            "``<script>``",
            r"\<script>",
            "\\``<!-- -->`",
            "`<!-- -->\\`",
        ] {
            assert_eq!(literal_after_line(protected), None, "source={protected:?}");
        }
        for block_start in ["<script>", "   <script>"] {
            assert_eq!(
                literal_after_line(block_start),
                Some(PendingCustomLiteral::RawText(RawTextElement::Script)),
                "source={block_start:?}",
            );
        }
        assert_eq!(
            literal_after_line("    <script>"),
            None,
            "four spaces form indented code rather than a raw HTML block",
        );
    }

    #[test]
    fn raw_text_html_uses_the_pinned_type_one_start_and_end_tokens() {
        for opening in ["<script", "<ScRiPt attr", "   <script"] {
            let source = format!(
                "<thinking>\n{opening}\n</script >\n</thinking>\n</SCRIPT>\n</thinking>\n</script>\n</thinking>"
            );
            let closer_start = source.rfind("</thinking>").unwrap();

            for opaque in [true, false] {
                let scan = find_custom_blocks(
                    &source,
                    &[spec_with_opaque(opaque)],
                    ProtocolLimits::default(),
                )
                .unwrap();
                assert_eq!(scan.blocks.len(), 1, "opaque={opaque}, source={source:?}");
                assert_eq!(
                    scan.blocks[0].body.end, closer_start,
                    "opaque={opaque}, source={source:?}",
                );
                assert_eq!(
                    scan.pending_start, None,
                    "opaque={opaque}, source={source:?}"
                );
            }
        }
    }

    #[test]
    fn a_closed_html_block_does_not_reopen_from_the_same_physical_line() {
        for line in [
            "<!-- --> <!--",
            "<![CDATA[ ]]> <![CDATA[",
            "<?one ?> <?two",
            "<!DOCTYPE> <!OPEN",
        ] {
            let source = format!("<thinking>\n{line}\n</thinking>");
            let closer_start = source.rfind("</thinking>").unwrap();

            for opaque in [true, false] {
                let scan = find_custom_blocks(
                    &source,
                    &[spec_with_opaque(opaque)],
                    ProtocolLimits::default(),
                )
                .unwrap();
                assert_eq!(scan.blocks.len(), 1, "opaque={opaque}, source={source:?}");
                assert_eq!(
                    scan.blocks[0].body.end, closer_start,
                    "opaque={opaque}, source={source:?}",
                );
            }
        }
    }

    #[test]
    fn indented_code_html_literals_do_not_hide_the_outer_custom_closer() {
        for protected_markdown in [
            "    <!--",
            "    <![CDATA[",
            "    <?instruction",
            "    <!DOCTYPE",
        ] {
            let source = format!("<thinking>\n{protected_markdown}\n    code\n</thinking>");
            let closer_start = source.rfind("</thinking>").unwrap();

            for opaque in [true, false] {
                let scan = find_custom_blocks(
                    &source,
                    &[spec_with_opaque(opaque)],
                    ProtocolLimits::default(),
                )
                .unwrap();

                assert_eq!(scan.blocks.len(), 1, "opaque={opaque}, source={source:?}");
                assert_eq!(
                    scan.blocks[0].body.end, closer_start,
                    "opaque={opaque}, source={source:?}",
                );
                assert_eq!(
                    scan.pending_start, None,
                    "opaque={opaque}, source={source:?}"
                );
            }
        }
    }

    #[test]
    fn top_level_literal_state_persists_until_its_physical_line_closes() {
        let specs = [spec()];
        let initial = find_custom_blocks("<script>\n", &specs, ProtocolLimits::default())
            .unwrap()
            .pending;
        assert_eq!(
            initial,
            Some(PendingCustomState::TopLevel {
                literal: Some(PendingCustomLiteral::RawText(RawTextElement::Script)),
                tentative_line: false,
            }),
        );

        let continued =
            append_reaches_custom_boundary("<script>\n", "<thinking>\n\n", &specs, initial);
        assert!(!continued.reaches_boundary);
        assert_eq!(continued.pending, initial);

        let closed = append_reaches_custom_boundary(
            "<script>\n<thinking>\n\n",
            "</script>\n",
            &specs,
            continued.pending,
        );
        assert!(!closed.reaches_boundary);
        assert_eq!(closed.pending, None);
    }

    #[test]
    fn consecutive_custom_blocks_use_one_linear_topology_scan() {
        let mut source = String::new();
        for index in 0..64 {
            write!(source, "<thinking>\n`</thinking>` {index}\n</thinking>\n\n").unwrap();
        }
        for opaque in [true, false] {
            let scan = find_custom_blocks(
                &source,
                &[spec_with_opaque(opaque)],
                ProtocolLimits::default(),
            )
            .unwrap();

            assert_eq!(scan.blocks.len(), 64, "opaque={opaque}");
            assert_eq!(scan.scan_source_bytes, source.len(), "opaque={opaque}");
            assert_eq!(scan.pending_start, None, "opaque={opaque}");
        }
    }

    #[test]
    fn duplicate_attributes_are_rejected() {
        let source = "<thinking x=1 x=2>\nbody\n</thinking>";
        let scan = find_custom_blocks(source, &[spec()], ProtocolLimits::default()).unwrap();
        assert_eq!(
            parse_custom_attributes(
                scan.blocks[0].attributes,
                &spec(),
                ProtocolLimits::default(),
            ),
            Err(CustomSyntaxError::DuplicateAttribute)
        );
    }

    #[test]
    fn incomplete_custom_block_reports_its_frontier_start() {
        let source = "prefix\n\n<thinking>\nfirst\n\nsecond";
        let scan = find_custom_blocks(source, &[spec()], ProtocolLimits::default()).unwrap();

        assert_eq!(scan.blocks.len(), 1);
        assert_eq!(scan.blocks[0].source.end, source.len());
        assert_eq!(scan.pending_start, source.find("<thinking>"));
    }

    #[test]
    fn topology_classifies_root_scope_without_leaking_markdown_containers() {
        let nested = concat!(
            "<thinking>\n",
            "\n",
            "<thinking>\ninner\n</thinking>\n",
            "</thinking>",
        );
        let scan = find_custom_blocks(
            nested,
            &[spec_with_opaque(false)],
            ProtocolLimits::default(),
        )
        .unwrap();
        assert_eq!(scan.roots, vec![0]);
        assert_eq!(scan.blocks.len(), 2);
        assert_eq!(scan.blocks[0].children, vec![1]);

        for rejected in [
            "paragraph\n<thinking>\nbody\n</thinking>",
            "- item\n\n  <thinking>\n  body\n  </thinking>",
            "    <thinking>\n    body\n    </thinking>",
        ] {
            assert!(
                find_custom_blocks(rejected, &[spec()], ProtocolLimits::default())
                    .unwrap()
                    .blocks
                    .is_empty(),
                "source={rejected:?}",
            );
        }

        let indented = "   <thinking>\nbody\n</thinking>";
        assert!(
            find_custom_blocks(indented, &[spec()], ProtocolLimits::default())
                .unwrap()
                .roots
                .is_empty()
        );
    }

    #[test]
    fn standalone_custom_openings_require_a_blank_boundary_even_for_html_block_names() {
        let div = CustomBlockSpec::try_new("app.custom/1", "div").unwrap();
        let attached = "paragraph\n<div>\nbody\n</div>";
        assert!(
            find_custom_blocks(
                attached,
                std::slice::from_ref(&div),
                ProtocolLimits::default()
            )
            .unwrap()
            .roots
            .is_empty()
        );

        let source = "paragraph\n\n<div>\nbody\n</div>";
        let scan = find_custom_blocks(source, &[div], ProtocolLimits::default()).unwrap();
        assert_eq!(scan.roots, vec![0]);
        assert_eq!(scan.blocks[0].source, "paragraph\n\n".len()..source.len());
    }

    #[test]
    fn rejected_opening_does_not_promote_a_following_custom_tag() {
        let thinking = CustomBlockSpec::try_new("app.custom/1", "thinking")
            .unwrap()
            .opaque(false);
        let tool = CustomBlockSpec::try_new("app.tool/1", "tool").unwrap();
        let source = concat!(
            "paragraph\n",
            "<thinking>\n",
            "<tool>\n",
            "body\n",
            "</tool>\n",
            "</thinking>",
        );

        let scan =
            find_custom_blocks(source, &[thinking, tool], ProtocolLimits::default()).unwrap();

        assert!(scan.blocks.is_empty());
    }

    #[test]
    fn different_custom_names_pair_strictly_with_the_stack_top() {
        let outer = CustomBlockSpec::try_new("app.outer/1", "outer")
            .unwrap()
            .opaque(false);
        let inner = CustomBlockSpec::try_new("app.inner/1", "inner")
            .unwrap()
            .opaque(false);
        let source = concat!(
            "<outer>\n",
            "\n",
            "<inner>\n",
            "</outer>\n",
            "</inner>\n",
            "</outer>",
        );

        let scan = find_custom_blocks(source, &[outer, inner], ProtocolLimits::default()).unwrap();

        assert_eq!(scan.roots, vec![0]);
        assert_eq!(scan.blocks[0].children, vec![1]);
        assert_eq!(scan.blocks[0].source, 0..source.len());
        assert_eq!(scan.blocks[1].body.end, source.find("</inner>").unwrap());
        assert!(scan.pending.is_none());
    }

    #[test]
    fn trailing_text_never_confirms_a_custom_closing_line() {
        let source = concat!(
            "<thinking>\n",
            "body\n",
            "</thinking> trailing\n",
            "</thinking>",
        );
        let scan = find_custom_blocks(source, &[spec()], ProtocolLimits::default()).unwrap();

        assert_eq!(scan.blocks[0].source, 0..source.len());
        assert_eq!(
            scan.blocks[0].body.end,
            source.rfind("</thinking>").unwrap()
        );
    }

    #[test]
    fn custom_topology_stops_before_allocating_the_first_unadmitted_node() {
        let source = concat!(
            "<thinking>\none\n</thinking>\n\n",
            "<thinking>\ntwo\n</thinking>\n\n",
            "<thinking>\npathological tail that must not need a third node",
        );

        assert_eq!(
            find_custom_blocks_with_node_limit(source, &[spec()], ProtocolLimits::default(), 1,),
            Err(CustomSyntaxError::LimitExceeded {
                field: "nodes",
                limit: 1,
                actual: 2,
            })
        );
    }

    #[test]
    fn custom_boundary_requires_a_complete_physical_line() {
        let pending = PendingCustomState::Custom {
            spec_index: 0,
            opaque_balance: 0,
            literal: None,
            tentative_line: false,
        };
        let specs = [spec()];

        assert!(
            !append_reaches_custom_boundary(
                "<thinking>\nbody\n",
                "</thinking>",
                &specs,
                Some(pending),
            )
            .reaches_boundary
        );
        assert!(
            append_reaches_custom_boundary(
                "<thinking>\nbody\n",
                "</thinking>\n",
                &specs,
                Some(pending),
            )
            .reaches_boundary
        );
    }
}
