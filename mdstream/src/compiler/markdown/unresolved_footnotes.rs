use std::{collections::BTreeSet, ops::Range};

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};
use unicase::UniCase;

use crate::compiler::extensions::{canonical_options, preserve_broken_reference};

use super::MarkdownError;

type MarkdownEvent<'source> = (Event<'source>, Range<usize>);

#[derive(Debug)]
pub(super) struct UnresolvedFootnote {
    range: Range<usize>,
    label: String,
}

pub(super) fn collect_canonical_events<'source, I>(
    source: &str,
    events: I,
    initial_depth: usize,
    event_limit: usize,
    tree_depth_limit: usize,
) -> Result<Vec<MarkdownEvent<'source>>, MarkdownError>
where
    I: Iterator<Item = MarkdownEvent<'source>>,
{
    let mut collected = Vec::new();
    let mut event_count = 0usize;
    let mut parser_depth = 0usize;
    for (event, range) in events {
        reserve_markdown_event(&mut event_count, event_limit)?;
        source
            .get(range.clone())
            .ok_or(MarkdownError::InvalidUtf8Boundary {
                start: range.start,
                end: range.end,
            })?;
        match &event {
            Event::Start(_) => {
                parser_depth = parser_depth
                    .checked_add(1)
                    .ok_or(MarkdownError::NumericOverflow("parser depth"))?;
                let actual = initial_depth
                    .checked_add(parser_depth)
                    .ok_or(MarkdownError::NumericOverflow("tree depth"))?;
                if actual > tree_depth_limit {
                    return Err(MarkdownError::LimitExceeded {
                        field: "tree.depth",
                        limit: tree_depth_limit,
                        actual,
                    });
                }
            }
            Event::End(_) => {
                parser_depth =
                    parser_depth
                        .checked_sub(1)
                        .ok_or(MarkdownError::UnexpectedEvent {
                            event: "end",
                            context: "parser depth preflight",
                        })?;
            }
            _ => {}
        }
        collected.push((event, range));
    }
    if parser_depth != 0 {
        return Err(MarkdownError::UnclosedContainer("parser preflight"));
    }
    Ok(collected)
}

pub(super) fn classify_unresolved_footnotes<'source>(
    fragment: &'source str,
    canonical_events: &[MarkdownEvent<'source>],
    event_limit: usize,
) -> Result<Vec<UnresolvedFootnote>, MarkdownError> {
    let canonical = canonical_events
        .iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::FootnoteReference(_)).then_some((range.start, range.end))
        })
        .collect::<BTreeSet<_>>();
    let options = canonical_options() | Options::ENABLE_OLD_FOOTNOTES;
    let parser =
        Parser::new_with_broken_link_callback(fragment, options, Some(preserve_broken_reference));
    let mut event_count = 0usize;
    let mut unresolved = Vec::new();
    for (event, range) in parser.into_offset_iter() {
        reserve_markdown_event(&mut event_count, event_limit)?;
        if let Event::FootnoteReference(label) = event {
            if !canonical.contains(&(range.start, range.end)) {
                unresolved.push(UnresolvedFootnote {
                    range,
                    label: UniCase::new(label.as_ref()).to_folded_case(),
                });
            }
        }
    }
    unresolved.sort_by_key(|footnote| (footnote.range.start, footnote.range.end));
    unresolved.dedup_by(|right, left| right.range == left.range);
    Ok(unresolved)
}

fn reserve_markdown_event(count: &mut usize, limit: usize) -> Result<(), MarkdownError> {
    *count = count
        .checked_add(1)
        .ok_or(MarkdownError::NumericOverflow("parser event count"))?;
    if *count > limit {
        Err(MarkdownError::LimitExceeded {
            field: "markdown.events",
            limit,
            actual: *count,
        })
    } else {
        Ok(())
    }
}

pub(super) fn overlay_unresolved_footnotes<'source>(
    source: &'source str,
    events: Vec<MarkdownEvent<'source>>,
    candidates: Vec<UnresolvedFootnote>,
    overlap_limit: usize,
) -> Result<UnresolvedFootnoteOverlay<'source>, MarkdownError> {
    let candidates = safe_unresolved_footnotes(source, &events, candidates, overlap_limit)?;
    let emitted = vec![false; candidates.len()];
    Ok(UnresolvedFootnoteOverlay {
        source,
        events: events.into_iter(),
        candidates,
        emitted,
        text: None,
        exhausted: false,
    })
}

fn safe_unresolved_footnotes(
    source: &str,
    events: &[MarkdownEvent<'_>],
    candidates: Vec<UnresolvedFootnote>,
    overlap_limit: usize,
) -> Result<Vec<UnresolvedFootnote>, MarkdownError> {
    let mut filtered = Vec::with_capacity(candidates.len());
    let mut previous_end = 0usize;
    for candidate in candidates {
        let raw =
            source
                .get(candidate.range.clone())
                .ok_or(MarkdownError::InvalidUtf8Boundary {
                    start: candidate.range.start,
                    end: candidate.range.end,
                })?;
        if candidate.range.start < previous_end
            || !raw.starts_with("[^")
            || !raw.ends_with(']')
            || raw.contains(['\n', '\r'])
        {
            continue;
        }
        previous_end = candidate.range.end;
        filtered.push(candidate);
    }
    if filtered.is_empty() {
        return Ok(filtered);
    }

    let mut safe = vec![true; filtered.len()];
    let mut has_owner = vec![false; filtered.len()];
    let mut coverable = vec![false; filtered.len()];
    let mut overlap_work = 0usize;
    for (event, range) in events {
        if range.is_empty() {
            continue;
        }
        let role = overlay_event_role(event);
        let start = filtered.partition_point(|candidate| candidate.range.end <= range.start);
        for index in start..filtered.len() {
            let candidate = &filtered[index];
            if candidate.range.start >= range.end {
                break;
            }
            overlap_work = overlap_work
                .checked_add(1)
                .ok_or(MarkdownError::NumericOverflow("footnote overlap work"))?;
            if overlap_work > overlap_limit {
                return Err(MarkdownError::LimitExceeded {
                    field: "markdown.footnote_overlap_work",
                    limit: overlap_limit,
                    actual: overlap_work,
                });
            }
            if !ranges_overlap(&candidate.range, range) {
                continue;
            }
            let event_contains =
                range.start <= candidate.range.start && candidate.range.end <= range.end;
            let candidate_contains =
                candidate.range.start <= range.start && range.end <= candidate.range.end;
            match role {
                OverlayEventRole::Text => coverable[index] = true,
                OverlayEventRole::Owner => {
                    if event_contains {
                        has_owner[index] = true;
                    } else {
                        safe[index] = false;
                    }
                }
                OverlayEventRole::Formatting => {
                    if candidate_contains {
                        coverable[index] = true;
                    } else if !event_contains {
                        safe[index] = false;
                    }
                }
                OverlayEventRole::ReplaceableLeaf => {
                    if candidate_contains {
                        coverable[index] = true;
                    } else {
                        safe[index] = false;
                    }
                }
                OverlayEventRole::Protected => safe[index] = false,
                OverlayEventRole::Ancestor => {
                    if !event_contains {
                        safe[index] = false;
                    }
                }
            }
        }
    }

    Ok(filtered
        .into_iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            (safe[index] && has_owner[index] && coverable[index]).then_some(candidate)
        })
        .collect())
}

#[derive(Clone, Copy)]
enum OverlayEventRole {
    Text,
    Owner,
    Formatting,
    ReplaceableLeaf,
    Protected,
    Ancestor,
}

fn overlay_event_role(event: &Event<'_>) -> OverlayEventRole {
    match event {
        Event::Text(_) => OverlayEventRole::Text,
        Event::Start(
            Tag::Paragraph
            | Tag::Heading { .. }
            | Tag::Item
            | Tag::FootnoteDefinition(_)
            | Tag::TableCell,
        )
        | Event::End(
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::Item
            | TagEnd::FootnoteDefinition
            | TagEnd::TableCell,
        ) => OverlayEventRole::Owner,
        Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough)
        | Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough) => {
            OverlayEventRole::Formatting
        }
        Event::Code(_) | Event::InlineHtml(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {
            OverlayEventRole::ReplaceableLeaf
        }
        Event::Start(
            Tag::CodeBlock(_)
            | Tag::HtmlBlock
            | Tag::Link { .. }
            | Tag::Image { .. }
            | Tag::Superscript
            | Tag::Subscript
            | Tag::MetadataBlock(_),
        )
        | Event::End(
            TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::Link
            | TagEnd::Image
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::MetadataBlock(_),
        )
        | Event::Html(_)
        | Event::FootnoteReference(_)
        | Event::SoftBreak
        | Event::HardBreak
        | Event::Rule
        | Event::TaskListMarker(_) => OverlayEventRole::Protected,
        Event::Start(_) | Event::End(_) => OverlayEventRole::Ancestor,
    }
}

struct TextOverlay {
    range: Range<usize>,
    cursor: usize,
    candidate_index: usize,
}

pub(super) struct UnresolvedFootnoteOverlay<'source> {
    source: &'source str,
    events: std::vec::IntoIter<MarkdownEvent<'source>>,
    candidates: Vec<UnresolvedFootnote>,
    emitted: Vec<bool>,
    text: Option<TextOverlay>,
    exhausted: bool,
}

impl<'source> UnresolvedFootnoteOverlay<'source> {
    fn next_text_piece(&mut self) -> Option<Result<MarkdownEvent<'source>, MarkdownError>> {
        loop {
            let state = self.text.as_ref()?;
            let range = state.range.clone();
            let cursor = state.cursor;
            let index = state.candidate_index;

            let Some(candidate) = self.candidates.get(index) else {
                return self.finish_text_overlay(range, cursor);
            };
            if candidate.range.start >= range.end {
                return self.finish_text_overlay(range, cursor);
            }
            if !ranges_overlap(&candidate.range, &range) {
                let next_index = match index.checked_add(1) {
                    Some(next_index) => next_index,
                    None => {
                        return Some(Err(MarkdownError::NumericOverflow(
                            "footnote candidate index",
                        )));
                    }
                };
                let Some(state) = self.text.as_mut() else {
                    return Some(Err(text_overlay_state_error()));
                };
                state.candidate_index = next_index;
                continue;
            }

            let prefix_end = candidate.range.start.max(range.start);
            if cursor < prefix_end {
                let Some(state) = self.text.as_mut() else {
                    return Some(Err(text_overlay_state_error()));
                };
                state.cursor = prefix_end;
                return Some(overlay_text(self.source, cursor..prefix_end));
            }

            let candidate_end = candidate.range.end;
            let next_index = match index.checked_add(1) {
                Some(next_index) => next_index,
                None => {
                    return Some(Err(MarkdownError::NumericOverflow(
                        "footnote candidate index",
                    )));
                }
            };
            let Some(state) = self.text.as_mut() else {
                return Some(Err(text_overlay_state_error()));
            };
            state.cursor = cursor.max(candidate_end.min(range.end));
            state.candidate_index = next_index;
            if let Some(event) = self.emit_candidate(index) {
                return Some(Ok(event));
            }
        }
    }

    fn finish_text_overlay(
        &mut self,
        range: Range<usize>,
        cursor: usize,
    ) -> Option<Result<MarkdownEvent<'source>, MarkdownError>> {
        if cursor < range.end {
            let Some(state) = self.text.as_mut() else {
                return Some(Err(text_overlay_state_error()));
            };
            state.cursor = range.end;
            Some(overlay_text(self.source, cursor..range.end))
        } else {
            self.text = None;
            None
        }
    }

    fn emit_candidate(&mut self, index: usize) -> Option<MarkdownEvent<'source>> {
        if self.emitted[index] {
            return None;
        }
        self.emitted[index] = true;
        let candidate = &self.candidates[index];
        Some((
            Event::FootnoteReference(CowStr::from(candidate.label.clone())),
            candidate.range.clone(),
        ))
    }
}

impl<'source> Iterator for UnresolvedFootnoteOverlay<'source> {
    type Item = Result<MarkdownEvent<'source>, MarkdownError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.text.is_some() {
                if let Some(event) = self.next_text_piece() {
                    return Some(event);
                }
            }

            let Some((event, range)) = self.events.next() else {
                if !self.exhausted {
                    debug_assert!(self.emitted.iter().all(|was_emitted| *was_emitted));
                    self.exhausted = true;
                }
                return None;
            };
            if let Event::Text(value) = event {
                let index = self
                    .candidates
                    .partition_point(|candidate| candidate.range.end <= range.start);
                if self
                    .candidates
                    .get(index)
                    .is_some_and(|candidate| ranges_overlap(&candidate.range, &range))
                {
                    self.text = Some(TextOverlay {
                        cursor: range.start,
                        range,
                        candidate_index: index,
                    });
                } else {
                    return Some(Ok((Event::Text(value), range)));
                }
                continue;
            }

            let replaceable = matches!(
                overlay_event_role(&event),
                OverlayEventRole::Formatting | OverlayEventRole::ReplaceableLeaf
            );
            if replaceable {
                if let Some(index) = candidate_containing_range(&self.candidates, &range) {
                    if let Some(event) = self.emit_candidate(index) {
                        return Some(Ok(event));
                    }
                    continue;
                }
            }
            return Some(Ok((event, range)));
        }
    }
}

fn text_overlay_state_error() -> MarkdownError {
    MarkdownError::UnexpectedEvent {
        event: "text-overlay-state",
        context: "unresolved-footnote overlay",
    }
}

fn overlay_text(source: &str, range: Range<usize>) -> Result<MarkdownEvent<'_>, MarkdownError> {
    let raw = source
        .get(range.clone())
        .ok_or(MarkdownError::InvalidUtf8Boundary {
            start: range.start,
            end: range.end,
        })?;
    Ok((
        Event::Text(CowStr::from(normalize_classified_text(raw))),
        range,
    ))
}

fn candidate_containing_range(
    candidates: &[UnresolvedFootnote],
    range: &Range<usize>,
) -> Option<usize> {
    let index = candidates.partition_point(|candidate| candidate.range.end <= range.start);
    candidates.get(index).and_then(|candidate| {
        (candidate.range.start <= range.start && range.end <= candidate.range.end).then_some(index)
    })
}

fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn normalize_classified_text(raw: &str) -> String {
    let mut output = String::new();
    for event in Parser::new_ext(raw, canonical_options()) {
        match event {
            Event::Text(value)
            | Event::Code(value)
            | Event::InlineHtml(value)
            | Event::Html(value) => output.push_str(&value),
            Event::InlineMath(value) => {
                output.push('$');
                output.push_str(&value);
                output.push('$');
            }
            Event::DisplayMath(value) => {
                output.push_str("$$");
                output.push_str(&value);
                output.push_str("$$");
            }
            Event::FootnoteReference(label) => {
                output.push_str("[^");
                output.push_str(&label);
                output.push(']');
            }
            Event::SoftBreak | Event::HardBreak => output.push('\n'),
            Event::Rule => output.push_str(raw),
            Event::TaskListMarker(true) => output.push_str("[x]"),
            Event::TaskListMarker(false) => output.push_str("[ ]"),
            Event::Start(_) | Event::End(_) => {}
        }
    }
    if output.is_empty() && !raw.is_empty() {
        raw.to_string()
    } else {
        output
    }
}
