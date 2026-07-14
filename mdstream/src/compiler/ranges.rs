use std::ops::Range;

use mdstream_protocol::{SemanticText, SourceCursor, SourceRange};

use super::MarkdownError;

pub(super) fn absolute_range(
    relative: Range<usize>,
    absolute_base: SourceCursor,
) -> Result<SourceRange, MarkdownError> {
    if relative.start > relative.end {
        return Err(MarkdownError::InvalidRange {
            start: relative.start,
            end: relative.end,
        });
    }
    Ok(SourceRange::new(
        absolute_cursor(relative.start, absolute_base)?,
        absolute_cursor(relative.end, absolute_base)?,
    ))
}

pub(super) fn absolute_cursor(
    relative: usize,
    absolute_base: SourceCursor,
) -> Result<SourceCursor, MarkdownError> {
    let relative = u64::try_from(relative).map_err(|_| MarkdownError::CursorOverflow)?;
    absolute_base
        .checked_add(relative)
        .ok_or(MarkdownError::CursorOverflow)
}

pub(super) fn semantic_text(raw: &str, semantic: &str) -> SemanticText {
    if raw == semantic {
        SemanticText::Source {}
    } else {
        SemanticText::Normalized {
            value: semantic.to_string(),
        }
    }
}

pub(super) fn checked_slice(source: &str, range: Range<usize>) -> Result<&str, MarkdownError> {
    if range.start > range.end || range.end > source.len() {
        return Err(MarkdownError::InvalidRange {
            start: range.start,
            end: range.end,
        });
    }
    source
        .get(range.clone())
        .ok_or(MarkdownError::InvalidUtf8Boundary {
            start: range.start,
            end: range.end,
        })
}

pub(super) fn without_trailing_line_ending(
    source: &str,
    mut range: Range<usize>,
) -> Result<Range<usize>, MarkdownError> {
    let raw = checked_slice(source, range.clone())?;
    let line_ending_bytes = if raw.ends_with("\r\n") {
        2
    } else if raw.ends_with(['\n', '\r']) {
        1
    } else {
        0
    };
    range.end = range
        .end
        .checked_sub(line_ending_bytes)
        .ok_or(MarkdownError::InvalidRange {
            start: range.start,
            end: range.end,
        })?;
    Ok(range)
}

pub(super) fn delimited_body(
    source: &str,
    range: Range<usize>,
    marker: u8,
    expected_length: Option<usize>,
) -> Result<Range<usize>, MarkdownError> {
    let raw = checked_slice(source, range.clone())?;
    let opening = raw.bytes().take_while(|byte| *byte == marker).count();
    let closing = raw.bytes().rev().take_while(|byte| *byte == marker).count();
    let valid = opening > 0
        && opening == closing
        && expected_length.is_none_or(|expected| opening == expected)
        && opening
            .checked_add(closing)
            .is_some_and(|delimiters| delimiters <= raw.len());
    if !valid {
        return Err(MarkdownError::InvalidDelimiterRange {
            marker: char::from(marker),
            start: range.start,
            end: range.end,
        });
    }
    let start = range
        .start
        .checked_add(opening)
        .ok_or(MarkdownError::CursorOverflow)?;
    let end = range
        .end
        .checked_sub(closing)
        .ok_or(MarkdownError::InvalidDelimiterRange {
            marker: char::from(marker),
            start: range.start,
            end: range.end,
        })?;
    Ok(start..end)
}
