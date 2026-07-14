use std::ops::Range;

use mdstream_protocol::{
    BlockQuoteKind, CodeBlockSyntax, CodeFenceMarker, LinkStyle, SourceRange, TableAlignment,
};
use pulldown_cmark::{
    Alignment, BlockQuoteKind as PulldownBlockQuoteKind, CodeBlockKind, HeadingLevel, LinkType,
};

use crate::compiler::{
    custom::CustomSyntaxError,
    draft::{DraftContentKind, DraftNode, DraftOriginHint, SyntheticRole},
    ranges::checked_slice,
};

use super::MarkdownError;

pub(super) const fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

pub(super) const fn block_quote_kind(style: Option<PulldownBlockQuoteKind>) -> BlockQuoteKind {
    match style {
        None => BlockQuoteKind::Plain,
        Some(PulldownBlockQuoteKind::Note) => BlockQuoteKind::Note,
        Some(PulldownBlockQuoteKind::Tip) => BlockQuoteKind::Tip,
        Some(PulldownBlockQuoteKind::Important) => BlockQuoteKind::Important,
        Some(PulldownBlockQuoteKind::Warning) => BlockQuoteKind::Warning,
        Some(PulldownBlockQuoteKind::Caution) => BlockQuoteKind::Caution,
    }
}

pub(super) const fn table_alignment(alignment: Alignment) -> TableAlignment {
    match alignment {
        Alignment::None => TableAlignment::None,
        Alignment::Left => TableAlignment::Left,
        Alignment::Center => TableAlignment::Center,
        Alignment::Right => TableAlignment::Right,
    }
}

pub(super) fn ordered_list_start(start: u64) -> Result<u32, MarkdownError> {
    if start > 999_999_999 {
        return Err(MarkdownError::InvalidListStart(start));
    }
    u32::try_from(start).map_err(|_| MarkdownError::InvalidListStart(start))
}

pub(super) fn link_contract(
    link_type: LinkType,
    id: &str,
) -> Result<(LinkStyle, Option<String>, bool), MarkdownError> {
    let contract = match link_type {
        LinkType::Inline => (LinkStyle::Inline, None, true),
        LinkType::Reference => (LinkStyle::Reference, Some(id.to_string()), true),
        LinkType::ReferenceUnknown => (LinkStyle::ReferenceUnknown, Some(id.to_string()), false),
        LinkType::Collapsed => (LinkStyle::Collapsed, Some(id.to_string()), true),
        LinkType::CollapsedUnknown => (LinkStyle::CollapsedUnknown, Some(id.to_string()), false),
        LinkType::Shortcut => (LinkStyle::Shortcut, Some(id.to_string()), true),
        LinkType::ShortcutUnknown => (LinkStyle::ShortcutUnknown, Some(id.to_string()), false),
        LinkType::Autolink => (LinkStyle::Autolink, None, true),
        LinkType::Email => (LinkStyle::Email, None, true),
        LinkType::WikiLink { .. } => return Err(MarkdownError::Unsupported("wikilink")),
    };
    Ok(contract)
}

pub(super) fn citation_key(link_type: LinkType, label: Option<&str>) -> Option<String> {
    if !matches!(
        link_type,
        LinkType::Reference
            | LinkType::ReferenceUnknown
            | LinkType::Collapsed
            | LinkType::CollapsedUnknown
            | LinkType::Shortcut
            | LinkType::ShortcutUnknown
    ) {
        return None;
    }
    let label = label?.trim();
    let key = label.strip_prefix('@')?.trim();
    (!key.is_empty()).then(|| key.to_lowercase())
}

pub(super) fn repair_collapsed_range(
    source: &str,
    range: &mut Range<usize>,
    link_type: LinkType,
) -> Result<(), MarkdownError> {
    if !matches!(link_type, LinkType::Collapsed | LinkType::CollapsedUnknown) {
        return Ok(());
    }
    let repaired_end = range
        .end
        .checked_add(2)
        .ok_or(MarkdownError::CursorOverflow)?;
    if source.get(range.end..repaired_end) == Some("[]") {
        range.end = repaired_end;
    }
    checked_slice(source, range.clone())?;
    Ok(())
}

pub(super) fn offset_range(
    range: Range<usize>,
    offset: usize,
) -> Result<Range<usize>, MarkdownError> {
    let start = range
        .start
        .checked_add(offset)
        .ok_or(MarkdownError::CursorOverflow)?;
    let end = range
        .end
        .checked_add(offset)
        .ok_or(MarkdownError::CursorOverflow)?;
    Ok(start..end)
}

pub(super) fn markdown_custom_error(error: CustomSyntaxError) -> MarkdownError {
    match error {
        CustomSyntaxError::AttributeName => MarkdownError::InvalidCustomAttributeName,
        CustomSyntaxError::AttributeValue => MarkdownError::InvalidCustomAttributeValue,
        CustomSyntaxError::DuplicateAttribute => MarkdownError::DuplicateCustomAttribute,
        CustomSyntaxError::LimitExceeded {
            field,
            limit,
            actual,
        } => MarkdownError::LimitExceeded {
            field,
            limit,
            actual,
        },
        CustomSyntaxError::NumericOverflow(field) => MarkdownError::NumericOverflow(field),
    }
}

pub(super) fn code_block_header(
    source: &str,
    range: Range<usize>,
    kind: CodeBlockKind<'_>,
) -> Result<(CodeBlockSyntax, Option<String>), MarkdownError> {
    match kind {
        CodeBlockKind::Indented => Ok((CodeBlockSyntax::Indented, None)),
        CodeBlockKind::Fenced(raw_info) => {
            let (marker, length) = parse_fence(source, range)?;
            let info = raw_info.trim();
            let info = if info.is_empty() {
                None
            } else {
                Some(info.to_string())
            };
            Ok((CodeBlockSyntax::Fenced { marker, length }, info))
        }
    }
}

fn parse_fence(source: &str, range: Range<usize>) -> Result<(CodeFenceMarker, u32), MarkdownError> {
    let raw = checked_slice(source, range)?;
    let first_line = raw
        .split('\n')
        .next()
        .ok_or(MarkdownError::InvalidCodeFence)?;
    let bytes = first_line.trim_end_matches('\r').as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'`' || byte == b'~' {
            let length = bytes[index..]
                .iter()
                .take_while(|candidate| **candidate == byte)
                .count();
            let prefix_is_container = bytes[..index]
                .iter()
                .all(|prefix| matches!(*prefix, b' ' | b'\t' | b'>'));
            if length >= 3 && prefix_is_container {
                let marker = if byte == b'`' {
                    CodeFenceMarker::Backtick
                } else {
                    CodeFenceMarker::Tilde
                };
                return Ok((
                    marker,
                    u32::try_from(length)
                        .map_err(|_| MarkdownError::NumericOverflow("code-fence length"))?,
                ));
            }
            index = index
                .checked_add(length.max(1))
                .ok_or(MarkdownError::CursorOverflow)?;
        } else {
            index = index.checked_add(1).ok_or(MarkdownError::CursorOverflow)?;
        }
    }
    Err(MarkdownError::InvalidCodeFence)
}

pub(super) fn empty_code_body(
    source: &str,
    range: Range<usize>,
) -> Result<Range<usize>, MarkdownError> {
    let header = checked_slice(source, range.clone())?;
    let relative = match header.find('\n') {
        Some(index) => index.checked_add(1).ok_or(MarkdownError::CursorOverflow)?,
        None => header.len(),
    };
    let start = range
        .start
        .checked_add(relative)
        .ok_or(MarkdownError::CursorOverflow)?;
    Ok(start..start)
}

pub(super) fn empty_image_body(
    source: &str,
    range: Range<usize>,
) -> Result<Range<usize>, MarkdownError> {
    let raw = checked_slice(source, range.clone())?;
    let relative = raw.find("![").ok_or(MarkdownError::InvalidDelimiterRange {
        marker: '[',
        start: range.start,
        end: range.end,
    })?;
    let start = range
        .start
        .checked_add(relative)
        .and_then(|start| start.checked_add(2))
        .ok_or(MarkdownError::CursorOverflow)?;
    Ok(start..start)
}

pub(super) fn extend_range(target: &mut Option<Range<usize>>, range: Range<usize>) {
    if let Some(target) = target {
        target.start = target.start.min(range.start);
        target.end = target.end.max(range.end);
    } else {
        *target = Some(range);
    }
}

pub(super) fn child_hull(children: &[DraftNode]) -> Option<SourceRange> {
    let first = children.first()?;
    let mut start = first.source.start;
    let mut end = first.source.end;
    for child in &children[1..] {
        if child.source.start.get() < start.get() {
            start = child.source.start;
        }
        if child.source.end.get() > end.get() {
            end = child.source.end;
        }
    }
    Some(SourceRange::new(start, end))
}

pub(super) fn synthetic_container(
    content: DraftContentKind,
    role: SyntheticRole,
    children: Vec<DraftNode>,
    fallback: SourceRange,
) -> DraftNode {
    let range = child_hull(&children).unwrap_or(SourceRange::new(fallback.start, fallback.start));
    DraftNode::container(
        range,
        range,
        DraftOriginHint::Synthetic(role),
        content,
        children,
    )
}

pub(super) fn synthesize_tight_paragraphs(children: Vec<DraftNode>) -> Vec<DraftNode> {
    let mut output = Vec::with_capacity(children.len());
    let mut phrasing = Vec::new();

    for child in children {
        if child.content.is_phrasing() {
            phrasing.push(child);
        } else {
            flush_tight_paragraph(&mut output, &mut phrasing);
            output.push(child);
        }
    }
    flush_tight_paragraph(&mut output, &mut phrasing);
    output
}

pub(super) fn tight_paragraph_count(children: &[DraftNode]) -> usize {
    let mut count = 0usize;
    let mut in_phrasing_run = false;
    for child in children {
        if child.content.is_phrasing() {
            if !in_phrasing_run {
                count = count.saturating_add(1);
                in_phrasing_run = true;
            }
        } else {
            in_phrasing_run = false;
        }
    }
    count
}

fn flush_tight_paragraph(output: &mut Vec<DraftNode>, phrasing: &mut Vec<DraftNode>) {
    if phrasing.is_empty() {
        return;
    }
    let children = std::mem::take(phrasing);
    let Some(range) = child_hull(&children) else {
        return;
    };
    output.push(DraftNode::container(
        range,
        range,
        DraftOriginHint::Synthetic(SyntheticRole::TightParagraph),
        DraftContentKind::Paragraph,
        children,
    ));
}

pub(super) fn list_is_tight(items: &[DraftNode]) -> bool {
    !items.iter().any(|item| {
        item.children.iter().any(|child| {
            matches!(child.content, DraftContentKind::Paragraph)
                && child.origin == DraftOriginHint::Parsed
        })
    })
}

pub(super) fn synthesize_table_body(
    children: Vec<DraftNode>,
    table_source: SourceRange,
) -> Result<Vec<DraftNode>, MarkdownError> {
    let mut children = children.into_iter();
    let Some(head) = children.next() else {
        return Err(MarkdownError::UnexpectedEvent {
            event: "table-close",
            context: "table-without-head",
        });
    };
    if !matches!(head.content, DraftContentKind::TableHead) {
        return Err(MarkdownError::UnexpectedEvent {
            event: "table-child",
            context: "expected-table-head",
        });
    }
    let rows = children.collect::<Vec<_>>();
    if rows
        .iter()
        .any(|row| !matches!(row.content, DraftContentKind::TableRow))
    {
        return Err(MarkdownError::UnexpectedEvent {
            event: "table-child",
            context: "expected-table-row",
        });
    }
    let body = synthetic_container(
        DraftContentKind::TableBody,
        SyntheticRole::TableBody,
        rows,
        SourceRange::new(table_source.end, table_source.end),
    );
    Ok(vec![head, body])
}

pub(super) fn source_contained_body(
    source: SourceRange,
    body: SourceRange,
) -> Result<SourceRange, MarkdownError> {
    if source.contains(body) {
        Ok(body)
    } else {
        let start = usize::try_from(body.start.get()).map_err(|_| MarkdownError::CursorOverflow)?;
        let end = usize::try_from(body.end.get()).map_err(|_| MarkdownError::CursorOverflow)?;
        Err(MarkdownError::InvalidRange { start, end })
    }
}
