use mdstream_protocol::{CodeBlockSyntax, CodeFenceMarker, ContentKind, Document, SourceCursor};

use super::{
    CustomBlockSpec,
    custom::{PendingCustomState, append_reaches_custom_boundary},
    draft::{DraftContentKind, DraftForest, DraftNode},
    types::CompilerError,
};
use crate::syntax::facts::fence_end;

pub(super) fn stable_root_prefix(
    draft: &DraftForest,
    source: &str,
    absolute_base: SourceCursor,
    finishing: bool,
) -> Result<usize, CompilerError> {
    if finishing || draft.roots.is_empty() {
        return Ok(draft.roots.len());
    }
    let custom_stability_limit = draft
        .pending_custom_start
        .map_or(draft.roots.len(), |start| {
            draft
                .roots
                .iter()
                .take_while(|root| root.source.end.get() <= start.get())
                .count()
        });
    let stable_before_last = draft.roots.len() - 1;
    if root_is_closed(
        draft
            .roots
            .last()
            .expect("non-empty roots have a last item"),
        source,
        absolute_base,
    )? {
        Ok(draft.roots.len().min(custom_stability_limit))
    } else {
        Ok(stable_before_last.min(custom_stability_limit))
    }
}

fn root_is_closed(
    root: &DraftNode,
    source: &str,
    absolute_base: SourceCursor,
) -> Result<bool, CompilerError> {
    let start = relative_offset(root.source.start, absolute_base)?;
    let end = relative_offset(root.source.end, absolute_base)?;
    let raw = source
        .get(start..end)
        .ok_or(CompilerError::InvalidSourceBoundary(root.source.start))?;
    match &root.content {
        DraftContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced { marker, length },
            ..
        } => Ok(has_closing_fence(raw, *marker, *length)),
        DraftContentKind::Heading { .. } | DraftContentKind::ThematicBreak => {
            Ok(raw.ends_with('\n') || source.get(end..).is_some_and(|tail| tail.starts_with('\n')))
        }
        DraftContentKind::Paragraph | DraftContentKind::Table { .. } => {
            Ok(trailing_blank_line(source))
        }
        DraftContentKind::Custom { .. } => Ok(true),
        _ => Ok(false),
    }
}

fn relative_offset(
    cursor: SourceCursor,
    absolute_base: SourceCursor,
) -> Result<usize, CompilerError> {
    cursor
        .get()
        .checked_sub(absolute_base.get())
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or(CompilerError::InvalidSourceBoundary(cursor))
}

fn trailing_blank_line(source: &str) -> bool {
    if !source.ends_with('\n') {
        return false;
    }
    let mut lines = source.split('\n').rev();
    let _trailing_empty = lines.next();
    lines.next().is_some_and(|line| line.trim().is_empty())
}

fn has_closing_fence(raw: &str, marker: CodeFenceMarker, length: u32) -> bool {
    let marker = fence_marker_char(marker);
    let required = usize::try_from(length).unwrap_or(usize::MAX);
    raw.split_inclusive('\n').skip(1).any(|line| {
        let Some(line) = line.strip_suffix('\n') else {
            return false;
        };
        fence_end(line, marker, required)
    })
}

const fn fence_marker_char(marker: CodeFenceMarker) -> char {
    match marker {
        CodeFenceMarker::Backtick => '`',
        CodeFenceMarker::Tilde => '~',
    }
}

pub(super) fn append_closes_structure(
    document: Option<&Document>,
    stable_root_count: usize,
    suffix: &str,
    custom_blocks: &[CustomBlockSpec],
    pending_custom: Option<PendingCustomState>,
) -> (bool, usize, Option<PendingCustomState>) {
    if suffix.is_empty() {
        return (false, 0, pending_custom);
    }
    let prefix = document.map_or("", Document::source);
    let frontier_root = document
        .and_then(|document| document.roots().get(stable_root_count))
        .and_then(|root_id| document.and_then(|document| document.node(*root_id)));

    if let Some(root) = frontier_root {
        if let ContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced { marker, length },
            ..
        } = root.content
        {
            let (boundary, source_bytes) =
                append_completes_fence_line(prefix, suffix, fence_marker_char(marker), length);
            return (boundary, source_bytes, pending_custom);
        }
    }
    let custom_scan = append_reaches_custom_boundary(prefix, suffix, custom_blocks, pending_custom);
    if custom_scan.reaches_boundary {
        return (true, custom_scan.source_bytes, custom_scan.pending);
    }
    let (contains_blank_line, contains_newline, source_bytes) =
        append_contains_blank_line(prefix, suffix);
    let source_bytes = source_bytes.saturating_add(custom_scan.source_bytes);
    if contains_blank_line {
        let closes_frontier = frontier_root.is_none_or(|root| {
            matches!(
                root.content,
                ContentKind::Paragraph {} | ContentKind::Table { .. }
            )
        });
        return (closes_frontier, source_bytes, custom_scan.pending);
    }
    let closes = frontier_root.is_some_and(|root| {
        matches!(
            root.content,
            ContentKind::Heading { .. } | ContentKind::ThematicBreak {}
        ) && contains_newline
    });
    (closes, source_bytes, custom_scan.pending)
}

fn append_contains_blank_line(prefix: &str, suffix: &str) -> (bool, bool, usize) {
    let contains_newline = suffix.as_bytes().contains(&b'\n');
    if !contains_newline {
        return (false, false, suffix.len());
    }
    let tail = prefix.rsplit_once('\n').map_or(prefix, |(_, tail)| tail);
    let mut appended = String::with_capacity(tail.len() + suffix.len());
    appended.push_str(tail);
    appended.push_str(suffix);
    let contains_blank_line = appended
        .split_inclusive('\n')
        .any(|line| line.ends_with('\n') && line.trim().is_empty());
    let source_bytes = suffix
        .len()
        .saturating_add(tail.len())
        .saturating_add(appended.len());
    (contains_blank_line, true, source_bytes)
}

fn append_completes_fence_line(
    prefix: &str,
    suffix: &str,
    marker: char,
    length: u32,
) -> (bool, usize) {
    if !suffix.as_bytes().contains(&b'\n') {
        return (false, suffix.len());
    }
    let tail = prefix.rsplit_once('\n').map_or(prefix, |(_, tail)| tail);
    let mut appended = String::with_capacity(tail.len() + suffix.len());
    appended.push_str(tail);
    appended.push_str(suffix);
    let required = usize::try_from(length).unwrap_or(usize::MAX);
    let closes = appended.split_inclusive('\n').any(|line| {
        if !line.ends_with('\n') {
            return false;
        }
        let line = line.strip_suffix('\n').expect("checked line ending");
        fence_end(line, marker, required)
    });
    let source_bytes = suffix
        .len()
        .saturating_add(tail.len())
        .saturating_add(appended.len());
    (closes, source_bytes)
}
