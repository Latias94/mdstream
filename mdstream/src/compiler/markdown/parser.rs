use std::{collections::BTreeMap, ops::Range};

use mdstream_protocol::{ProtocolLimits, SourceCursor, SourceRange};
use pulldown_cmark::{CowStr, Event, Parser, RefDefs, Tag, TagEnd};

use crate::compiler::{
    CustomBlockSpec,
    custom::{
        CustomBlockMatch, CustomStartContext, PendingCustomState,
        find_custom_blocks_with_node_budget, parse_custom_attributes,
    },
    draft::{
        DraftContentKind, DraftForest, DraftNode, DraftOriginHint, DraftResource,
        DraftResourceIndex, DraftResourceKey, DraftResourceRole, SyntheticRole,
    },
    extensions::{canonical_options, preserve_broken_reference},
    ranges::{
        absolute_cursor, absolute_range, checked_slice, delimited_body, semantic_text,
        without_trailing_line_ending,
    },
};

use super::{
    MarkdownError,
    budget::{DraftBudget, DraftUsage},
    frame::{Frame, FrameEnd, FramePayload, collect_semantic_event, end_name},
    limits::{draft_node_metadata, draft_resource_metadata, validate_draft_limits},
    normalization::{
        block_quote_kind, child_hull, citation_key, code_block_header, empty_code_body,
        empty_image_body, extend_range, heading_level, link_contract, list_is_tight,
        markdown_custom_error, offset_range, ordered_list_start, repair_collapsed_range,
        source_contained_body, synthesize_table_body, synthesize_tight_paragraphs,
        synthetic_container, table_alignment, tight_paragraph_count,
    },
};

#[cfg(test)]
pub(super) fn compile_markdown(
    source: &str,
    absolute_base: SourceCursor,
) -> Result<DraftForest, MarkdownError> {
    compile_markdown_with_custom(
        source,
        absolute_base,
        &[],
        ProtocolLimits::default(),
        DraftUsage::default(),
        CustomStartContext::DocumentStart,
        true,
    )
    .map(|compilation| compilation.forest)
}

pub(crate) struct MarkdownCompilation {
    pub(crate) forest: DraftForest,
    pub(crate) parse_passes: u64,
    pub(crate) parsed_source_bytes: u64,
    pub(crate) custom_scan_source_bytes: u64,
    pub(crate) pending_custom: Option<PendingCustomState>,
}

pub(crate) fn compile_markdown_with_custom(
    source: &str,
    absolute_base: SourceCursor,
    custom_blocks: &[CustomBlockSpec],
    limits: ProtocolLimits,
    baseline: DraftUsage,
    custom_start_context: CustomStartContext,
    confirm_eof: bool,
) -> Result<MarkdownCompilation, MarkdownError> {
    absolute_cursor(source.len(), absolute_base)?;
    let mut compiler = MarkdownCompiler::new(
        source,
        absolute_base,
        custom_blocks,
        limits,
        baseline,
        custom_start_context,
        confirm_eof,
    );
    compiler.compile_document()?;
    compiler.finish()
}

enum CustomCompileTask {
    Region {
        range: Range<usize>,
        children: Vec<usize>,
    },
    Markdown(Range<usize>),
    Custom(usize),
    CloseCustom,
}

struct CustomBody {
    range: Range<usize>,
    children: Vec<usize>,
}

struct ReferenceDefinitions {
    labels_by_span: BTreeMap<(usize, usize), String>,
}

impl ReferenceDefinitions {
    fn new(definitions: &RefDefs<'_>) -> Self {
        let labels_by_span = definitions
            .iter()
            .map(|(label, definition)| {
                (
                    (definition.span.start, definition.span.end),
                    label.to_string(),
                )
            })
            .collect();
        Self { labels_by_span }
    }

    fn canonical_label<'definitions>(
        &'definitions self,
        definitions: &RefDefs<'_>,
        label: &str,
    ) -> Option<&'definitions str> {
        let definition = definitions.get(label)?;
        self.labels_by_span
            .get(&(definition.span.start, definition.span.end))
            .map(String::as_str)
    }
}

struct MarkdownCompiler<'source, 'config> {
    source: &'source str,
    absolute_base: SourceCursor,
    custom_blocks: &'config [CustomBlockSpec],
    limits: ProtocolLimits,
    budget: DraftBudget,
    roots: Vec<DraftNode>,
    resources: Vec<DraftResource>,
    reference_resources: BTreeMap<DraftResourceRole, BTreeMap<String, DraftResourceIndex>>,
    stack: Vec<Frame>,
    pending_custom_start: Option<usize>,
    parse_passes: u64,
    parsed_source_bytes: u64,
    custom_scan_source_bytes: u64,
    pending_custom: Option<PendingCustomState>,
    custom_start_context: CustomStartContext,
    confirm_eof: bool,
}

impl<'source, 'config> MarkdownCompiler<'source, 'config> {
    const fn new(
        source: &'source str,
        absolute_base: SourceCursor,
        custom_blocks: &'config [CustomBlockSpec],
        limits: ProtocolLimits,
        baseline: DraftUsage,
        custom_start_context: CustomStartContext,
        confirm_eof: bool,
    ) -> Self {
        Self {
            source,
            absolute_base,
            custom_blocks,
            limits,
            budget: DraftBudget::new(limits, baseline),
            roots: Vec::new(),
            resources: Vec::new(),
            reference_resources: BTreeMap::new(),
            stack: Vec::new(),
            pending_custom_start: None,
            parse_passes: 0,
            parsed_source_bytes: 0,
            custom_scan_source_bytes: 0,
            pending_custom: None,
            custom_start_context,
            confirm_eof,
        }
    }

    fn compile_document(&mut self) -> Result<(), MarkdownError> {
        if self.source.is_empty() {
            return Ok(());
        }
        if self.custom_blocks.is_empty() {
            return self.compile_markdown_fragment(0..self.source.len());
        }

        let scan = find_custom_blocks_with_node_budget(
            self.source,
            self.custom_blocks,
            self.limits,
            self.budget.usage().nodes,
            self.limits.max_nodes,
            self.custom_start_context,
            self.confirm_eof,
        )
        .map_err(markdown_custom_error)?;
        self.custom_scan_source_bytes = self
            .custom_scan_source_bytes
            .checked_add(
                u64::try_from(scan.scan_source_bytes)
                    .map_err(|_| MarkdownError::NumericOverflow("custom scan source bytes"))?,
            )
            .ok_or(MarkdownError::NumericOverflow("custom scan source bytes"))?;
        self.pending_custom_start = scan.pending_start;
        self.pending_custom = scan.pending;
        let mut blocks = scan.blocks.into_iter().map(Some).collect::<Vec<_>>();
        let mut tasks = vec![CustomCompileTask::Region {
            range: 0..self.source.len(),
            children: scan.roots,
        }];

        while let Some(task) = tasks.pop() {
            match task {
                CustomCompileTask::Region { range, children } => {
                    self.schedule_custom_region(range, &children, &blocks, &mut tasks)?;
                }
                CustomCompileTask::Markdown(range) => {
                    self.compile_markdown_fragment(range)?;
                }
                CustomCompileTask::Custom(index) => {
                    let block = blocks
                        .get_mut(index)
                        .and_then(Option::take)
                        .ok_or(MarkdownError::Unsupported("custom-topology"))?;
                    if let Some(body) = self.begin_custom_block(block)? {
                        tasks.push(CustomCompileTask::CloseCustom);
                        tasks.push(CustomCompileTask::Region {
                            range: body.range,
                            children: body.children,
                        });
                    }
                }
                CustomCompileTask::CloseCustom => self.close_custom_block()?,
            }
        }
        Ok(())
    }

    fn schedule_custom_region(
        &self,
        range: Range<usize>,
        children: &[usize],
        blocks: &[Option<CustomBlockMatch<'_>>],
        tasks: &mut Vec<CustomCompileTask>,
    ) -> Result<(), MarkdownError> {
        let trailing_start = children.last().map_or(range.start, |index| {
            blocks[*index]
                .as_ref()
                .expect("scheduled custom blocks are available")
                .source
                .end
        });
        tasks.push(CustomCompileTask::Markdown(trailing_start..range.end));

        for (position, index) in children.iter().copied().enumerate().rev() {
            let block = blocks
                .get(index)
                .and_then(Option::as_ref)
                .ok_or(MarkdownError::Unsupported("custom-topology"))?;
            let gap_start = if position == 0 {
                range.start
            } else {
                blocks[children[position - 1]]
                    .as_ref()
                    .expect("scheduled custom blocks are available")
                    .source
                    .end
            };
            tasks.push(CustomCompileTask::Custom(index));
            tasks.push(CustomCompileTask::Markdown(gap_start..block.source.start));
        }
        Ok(())
    }

    fn compile_markdown_fragment(&mut self, range: Range<usize>) -> Result<(), MarkdownError> {
        if range.is_empty() {
            return Ok(());
        }
        let fragment_len = checked_slice(self.source, range.clone())?.len();
        self.record_parse(fragment_len)?;
        let fragment = checked_slice(self.source, range.clone())?;
        let parser = Parser::new_with_broken_link_callback(
            fragment,
            canonical_options(),
            Some(preserve_broken_reference),
        );
        let reference_labels = ReferenceDefinitions::new(parser.reference_definitions());
        let mut parser = parser.into_offset_iter();
        let mut pending_event = None;
        let initial_depth = self.stack.len();
        while let Some((event, event_range)) = next_merged_event(&mut parser, &mut pending_event) {
            self.consume(
                event,
                offset_range(event_range, range.start)?,
                parser.reference_definitions(),
                &reference_labels,
            )?;
        }
        if self.stack.len() != initial_depth {
            return Err(MarkdownError::UnclosedContainer("fragment"));
        }
        Ok(())
    }

    fn record_parse(&mut self, source_bytes: usize) -> Result<(), MarkdownError> {
        let source_bytes = u64::try_from(source_bytes)
            .map_err(|_| MarkdownError::NumericOverflow("parsed source bytes"))?;
        self.parse_passes = self
            .parse_passes
            .checked_add(1)
            .ok_or(MarkdownError::NumericOverflow("parse passes"))?;
        self.parsed_source_bytes = self
            .parsed_source_bytes
            .checked_add(source_bytes)
            .ok_or(MarkdownError::NumericOverflow("parsed source bytes"))?;
        Ok(())
    }

    fn begin_custom_block(
        &mut self,
        block: CustomBlockMatch<'_>,
    ) -> Result<Option<CustomBody>, MarkdownError> {
        self.ensure_tree_depth(self.stack.len().saturating_add(1))?;
        self.budget.reserve_node(0)?;
        let spec = self
            .custom_blocks
            .get(block.spec_index)
            .ok_or(MarkdownError::Unsupported("custom-block-spec"))?;
        let attributes = parse_custom_attributes(block.attributes, spec, self.limits)
            .map_err(markdown_custom_error)?;
        let content = DraftContentKind::Custom {
            namespace: spec.namespace().to_string(),
            name: spec.name().to_string(),
            opaque: spec.is_opaque(),
            attributes,
        };
        if spec.is_opaque() {
            self.push_leaf_reserved(block.source, block.body, content)?;
            return Ok(None);
        }

        self.stack.push(Frame {
            expected: FrameEnd::Custom,
            source: block.source,
            payload: FramePayload::Custom {
                content,
                body: block.body.clone(),
            },
            children: Vec::new(),
            collector_depth: 0,
        });
        Ok(Some(CustomBody {
            range: block.body,
            children: block.children,
        }))
    }

    fn close_custom_block(&mut self) -> Result<(), MarkdownError> {
        let frame = self.stack.pop().ok_or(MarkdownError::StackMismatch {
            expected: "custom-block",
            actual: "document",
        })?;
        let node = self.finish_frame(frame)?;
        self.push_node(node)
    }

    fn consume(
        &mut self,
        event: Event<'source>,
        range: Range<usize>,
        reference_definitions: &RefDefs<'_>,
        reference_labels: &ReferenceDefinitions,
    ) -> Result<(), MarkdownError> {
        checked_slice(self.source, range.clone())?;

        if self
            .stack
            .last()
            .is_some_and(|frame| frame.payload.is_collector())
        {
            return self.consume_collected(event, range);
        }

        match event {
            Event::Start(tag) => self.open(tag, range, reference_definitions, reference_labels),
            Event::End(end) => self.close(end),
            Event::Text(value) => {
                self.budget.reserve_node(0)?;
                let raw = checked_slice(self.source, range.clone())?;
                self.push_leaf_reserved(
                    range.clone(),
                    range,
                    DraftContentKind::Text {
                        text: semantic_text(raw, &value),
                    },
                )
            }
            Event::Code(value) => {
                self.budget.reserve_node(0)?;
                let body = delimited_body(self.source, range.clone(), b'`', None)?;
                let raw = checked_slice(self.source, body.clone())?;
                self.push_leaf_reserved(
                    range,
                    body,
                    DraftContentKind::InlineCode {
                        text: semantic_text(raw, &value),
                    },
                )
            }
            Event::InlineMath(value) => self.push_math(range, value.as_ref(), false),
            Event::DisplayMath(value) => self.push_math(range, value.as_ref(), true),
            Event::InlineHtml(value) => {
                self.budget.reserve_node(0)?;
                let raw = checked_slice(self.source, range.clone())?;
                self.push_leaf_reserved(
                    range.clone(),
                    range,
                    DraftContentKind::Html {
                        block: false,
                        text: semantic_text(raw, &value),
                    },
                )
            }
            Event::FootnoteReference(label) => {
                self.budget.reserve_node(0)?;
                self.push_leaf_reserved(
                    range.clone(),
                    range,
                    DraftContentKind::FootnoteReference {
                        label: label.into_string(),
                    },
                )
            }
            Event::SoftBreak => {
                self.budget.reserve_node(0)?;
                self.push_leaf_reserved(range.clone(), range, DraftContentKind::SoftBreak)
            }
            Event::HardBreak => {
                self.budget.reserve_node(0)?;
                self.push_leaf_reserved(range.clone(), range, DraftContentKind::HardBreak)
            }
            Event::Rule => {
                self.budget.reserve_node(0)?;
                self.push_leaf_reserved(range.clone(), range, DraftContentKind::ThematicBreak)
            }
            Event::TaskListMarker(checked) => self.apply_task_marker(checked),
            Event::Html(_) => Err(MarkdownError::UnexpectedEvent {
                event: "html",
                context: "document",
            }),
        }
    }

    fn open(
        &mut self,
        tag: Tag<'source>,
        mut range: Range<usize>,
        reference_definitions: &RefDefs<'_>,
        reference_labels: &ReferenceDefinitions,
    ) -> Result<(), MarkdownError> {
        self.ensure_tree_depth(self.stack.len().saturating_add(1))?;
        let content_structural_items = match &tag {
            Tag::Table(alignments) => alignments.len(),
            _ => 0,
        };
        self.budget.reserve_node(content_structural_items)?;
        let expected = FrameEnd::Parser(tag.to_end());
        let payload = match tag {
            Tag::Paragraph => FramePayload::Paragraph,
            Tag::Heading {
                level,
                id,
                classes,
                attrs,
            } => {
                if id.is_some() || !classes.is_empty() || !attrs.is_empty() {
                    return Err(MarkdownError::Unsupported("heading-attributes"));
                }
                range = without_trailing_line_ending(self.source, range)?;
                FramePayload::Heading {
                    level: heading_level(level),
                }
            }
            Tag::BlockQuote(style) => FramePayload::BlockQuote {
                style: block_quote_kind(style),
            },
            Tag::CodeBlock(kind) => {
                let (syntax, info) = code_block_header(self.source, range.clone(), kind)?;
                FramePayload::CodeBlock {
                    syntax,
                    info,
                    text: String::new(),
                    body: None,
                }
            }
            Tag::HtmlBlock => FramePayload::HtmlBlock {
                text: String::new(),
                body: None,
            },
            Tag::List(start) => {
                let start = start.map(ordered_list_start).transpose()?;
                FramePayload::List {
                    ordered: start.is_some(),
                    start,
                }
            }
            Tag::Item => FramePayload::Item { checked: None },
            Tag::FootnoteDefinition(label) => FramePayload::FootnoteDefinition {
                label: label.into_string(),
            },
            Tag::Table(alignments) => FramePayload::Table {
                alignments: alignments.into_iter().map(table_alignment).collect(),
            },
            Tag::TableHead => FramePayload::TableHead,
            Tag::TableRow => FramePayload::TableRow,
            Tag::TableCell => FramePayload::TableCell {
                column: self.next_table_column()?,
            },
            Tag::Emphasis => FramePayload::Emphasis,
            Tag::Strong => FramePayload::Strong,
            Tag::Strikethrough => FramePayload::Strikethrough,
            Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            } => {
                repair_collapsed_range(self.source, &mut range, link_type)?;
                let (style, reference_label, resolved) = link_contract(link_type, id.as_ref())?;
                let canonical_reference_label = reference_label.as_deref().map(|label| {
                    reference_labels
                        .canonical_label(reference_definitions, label)
                        .unwrap_or(label)
                });
                if let Some(key) = citation_key(link_type, canonical_reference_label) {
                    let target = if resolved {
                        Some(self.push_resource(
                            DraftResourceRole::Citation,
                            range.clone(),
                            Some(key.as_str()),
                            dest_url.as_ref(),
                            (!title.is_empty()).then_some(title.as_ref()),
                        )?)
                    } else {
                        None
                    };
                    FramePayload::CitationReference { key, target }
                } else {
                    let target = if resolved {
                        Some(self.push_resource(
                            DraftResourceRole::Link,
                            range.clone(),
                            canonical_reference_label,
                            dest_url.as_ref(),
                            (!title.is_empty()).then_some(title.as_ref()),
                        )?)
                    } else {
                        None
                    };
                    FramePayload::Link {
                        target,
                        reference_label,
                        style,
                    }
                }
            }
            Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            } => {
                repair_collapsed_range(self.source, &mut range, link_type)?;
                let (style, reference_label, resolved) = link_contract(link_type, id.as_ref())?;
                let canonical_reference_label = reference_label.as_deref().map(|label| {
                    reference_labels
                        .canonical_label(reference_definitions, label)
                        .unwrap_or(label)
                });
                let target = if resolved {
                    Some(self.push_resource(
                        DraftResourceRole::Image,
                        range.clone(),
                        canonical_reference_label,
                        dest_url.as_ref(),
                        (!title.is_empty()).then_some(title.as_ref()),
                    )?)
                } else {
                    None
                };
                FramePayload::Image {
                    target,
                    reference_label,
                    style,
                    alt: String::new(),
                    body: None,
                }
            }
            Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
                return Err(MarkdownError::Unsupported("definition-list"));
            }
            Tag::Superscript => return Err(MarkdownError::Unsupported("superscript")),
            Tag::Subscript => return Err(MarkdownError::Unsupported("subscript")),
            Tag::MetadataBlock(_) => return Err(MarkdownError::Unsupported("metadata-block")),
        };

        self.stack.push(Frame {
            expected,
            source: range,
            payload,
            children: Vec::new(),
            collector_depth: 0,
        });
        Ok(())
    }

    fn close(&mut self, end: TagEnd) -> Result<(), MarkdownError> {
        let frame = self.stack.pop().ok_or(MarkdownError::StackMismatch {
            expected: "container-start",
            actual: end_name(end),
        })?;
        if frame.expected != FrameEnd::Parser(end) {
            return Err(MarkdownError::StackMismatch {
                expected: frame.expected.name(),
                actual: end_name(end),
            });
        }
        let node = self.finish_frame(frame)?;
        self.push_node(node)
    }

    fn consume_collected(
        &mut self,
        event: Event<'source>,
        range: Range<usize>,
    ) -> Result<(), MarkdownError> {
        let closes_collector = matches!(event, Event::End(end) if self.stack.last().is_some_and(|frame| frame.collector_depth == 0 && frame.expected == FrameEnd::Parser(end)));
        if closes_collector {
            let Event::End(end) = event else {
                return Err(MarkdownError::UnexpectedEvent {
                    event: "non-end",
                    context: "collector-close",
                });
            };
            return self.close(end);
        }

        let frame = self.stack.last_mut().ok_or(MarkdownError::StackMismatch {
            expected: "collector",
            actual: "document",
        })?;
        extend_range(frame.payload.collector_body_mut()?, range.clone());

        match event {
            Event::Start(_) => {
                frame.collector_depth = frame
                    .collector_depth
                    .checked_add(1)
                    .ok_or(MarkdownError::NumericOverflow("collector nesting"))?;
            }
            Event::End(end) => {
                if frame.collector_depth == 0 {
                    return Err(MarkdownError::StackMismatch {
                        expected: frame.expected.name(),
                        actual: end_name(end),
                    });
                }
                frame.collector_depth = frame
                    .collector_depth
                    .checked_sub(1)
                    .ok_or(MarkdownError::NumericOverflow("collector nesting"))?;
            }
            event => collect_semantic_event(&mut frame.payload, event)?,
        }
        Ok(())
    }

    fn finish_frame(&mut self, mut frame: Frame) -> Result<DraftNode, MarkdownError> {
        let source = absolute_range(frame.source.clone(), self.absolute_base)?;

        let (content, children, body, origin) = match frame.payload {
            FramePayload::CodeBlock {
                syntax,
                info,
                text,
                body,
            } => {
                let body = match body {
                    Some(body) => body,
                    None => empty_code_body(self.source, frame.source.clone())?,
                };
                let raw = checked_slice(self.source, body.clone())?;
                (
                    DraftContentKind::CodeBlock {
                        syntax,
                        info,
                        text: semantic_text(raw, &text),
                    },
                    Vec::new(),
                    absolute_range(body, self.absolute_base)?,
                    DraftOriginHint::Parsed,
                )
            }
            FramePayload::HtmlBlock { text, body } => {
                let body = body.unwrap_or_else(|| frame.source.clone());
                let raw = checked_slice(self.source, body.clone())?;
                (
                    DraftContentKind::Html {
                        block: true,
                        text: semantic_text(raw, &text),
                    },
                    Vec::new(),
                    absolute_range(body, self.absolute_base)?,
                    DraftOriginHint::Parsed,
                )
            }
            FramePayload::Custom { content, body } => (
                content,
                frame.children,
                absolute_range(body, self.absolute_base)?,
                DraftOriginHint::Parsed,
            ),
            FramePayload::Image {
                target,
                reference_label,
                style,
                alt,
                body,
            } => {
                let body = match body {
                    Some(body) => body,
                    None => empty_image_body(self.source, frame.source.clone())?,
                };
                let raw = checked_slice(self.source, body.clone())?;
                (
                    DraftContentKind::Image {
                        target,
                        reference_label,
                        style,
                        alt: semantic_text(raw, &alt),
                    },
                    Vec::new(),
                    absolute_range(body, self.absolute_base)?,
                    DraftOriginHint::Parsed,
                )
            }
            payload => {
                let content = match payload {
                    FramePayload::Paragraph => DraftContentKind::Paragraph,
                    FramePayload::Heading { level } => DraftContentKind::Heading { level },
                    FramePayload::BlockQuote { style } => DraftContentKind::BlockQuote { style },
                    FramePayload::List { ordered, start } => {
                        let tight = list_is_tight(&frame.children);
                        DraftContentKind::List {
                            ordered,
                            start,
                            tight,
                        }
                    }
                    FramePayload::Item { checked } => {
                        self.budget
                            .reserve_synthetic_nodes(tight_paragraph_count(&frame.children))?;
                        frame.children = synthesize_tight_paragraphs(frame.children);
                        DraftContentKind::ListItem { checked }
                    }
                    FramePayload::FootnoteDefinition { label } => {
                        DraftContentKind::FootnoteDefinition { label }
                    }
                    FramePayload::Table { alignments } => {
                        self.budget.reserve_synthetic_nodes(1)?;
                        frame.children = synthesize_table_body(frame.children, source)?;
                        DraftContentKind::Table { alignments }
                    }
                    FramePayload::TableHead => {
                        self.budget.reserve_synthetic_nodes(1)?;
                        frame.children = vec![synthetic_container(
                            DraftContentKind::TableRow,
                            SyntheticRole::TableHeaderRow,
                            frame.children,
                            source,
                        )];
                        DraftContentKind::TableHead
                    }
                    FramePayload::TableRow => DraftContentKind::TableRow,
                    FramePayload::TableCell { column } => DraftContentKind::TableCell { column },
                    FramePayload::Emphasis => DraftContentKind::Emphasis,
                    FramePayload::Strong => DraftContentKind::Strong,
                    FramePayload::Strikethrough => DraftContentKind::Strikethrough,
                    FramePayload::Link {
                        target,
                        reference_label,
                        style,
                    } => DraftContentKind::Link {
                        target,
                        reference_label,
                        style,
                    },
                    FramePayload::CitationReference { key, target } => {
                        DraftContentKind::CitationReference { key, target }
                    }
                    FramePayload::CodeBlock { .. }
                    | FramePayload::HtmlBlock { .. }
                    | FramePayload::Custom { .. }
                    | FramePayload::Image { .. } => {
                        return Err(MarkdownError::UnexpectedEvent {
                            event: "collector",
                            context: "generic-container-close",
                        });
                    }
                };
                let body = child_hull(&frame.children)
                    .unwrap_or(SourceRange::new(source.start, source.start));
                (content, frame.children, body, DraftOriginHint::Parsed)
            }
        };

        Ok(DraftNode::container(
            source,
            source_contained_body(source, body)?,
            origin,
            content,
            children,
        ))
    }

    fn push_math(
        &mut self,
        range: Range<usize>,
        value: &str,
        display: bool,
    ) -> Result<(), MarkdownError> {
        self.budget.reserve_node(0)?;
        let body = delimited_body(
            self.source,
            range.clone(),
            b'$',
            Some(if display { 2 } else { 1 }),
        )?;
        let raw = checked_slice(self.source, body.clone())?;
        self.push_leaf_reserved(
            range,
            body,
            DraftContentKind::Math {
                display,
                text: semantic_text(raw, value),
            },
        )
    }

    fn push_leaf_reserved(
        &mut self,
        source: Range<usize>,
        body: Range<usize>,
        content: DraftContentKind,
    ) -> Result<(), MarkdownError> {
        let source = absolute_range(source, self.absolute_base)?;
        let body = absolute_range(body, self.absolute_base)?;
        self.push_node(DraftNode::leaf(source, body, content))
    }

    fn push_node(&mut self, node: DraftNode) -> Result<(), MarkdownError> {
        self.ensure_tree_depth(self.stack.len().saturating_add(1))?;
        let root = self.stack.is_empty();
        let target = self
            .stack
            .last()
            .map_or(&self.roots, |frame| &frame.children);
        let child_count = target
            .len()
            .checked_add(1)
            .ok_or(MarkdownError::NumericOverflow("child-list length"))?;
        if !root && child_count > self.limits.max_children_per_list {
            return Err(MarkdownError::LimitExceeded {
                field: "children",
                limit: self.limits.max_children_per_list,
                actual: child_count,
            });
        }
        let metadata_bytes = draft_node_metadata(&node.content, self.limits)?;
        self.budget.reserve_node_payload(root, metadata_bytes)?;
        let target = self
            .stack
            .last_mut()
            .map_or(&mut self.roots, |frame| &mut frame.children);
        target.push(node);
        Ok(())
    }

    fn push_resource(
        &mut self,
        role: DraftResourceRole,
        source: Range<usize>,
        reference_label: Option<&str>,
        destination: &str,
        title: Option<&str>,
    ) -> Result<DraftResourceIndex, MarkdownError> {
        if let Some(reference_label) = reference_label {
            let existing = self
                .reference_resources
                .get(&role)
                .and_then(|resources| resources.get(reference_label))
                .copied();
            if let Some(index) = existing {
                return Ok(index);
            }
        }

        let source = absolute_range(source, self.absolute_base)?;
        let metadata_bytes =
            resource_metadata_bytes(role, reference_label, destination, title, self.limits)?;
        self.budget.reserve_resource(metadata_bytes)?;

        let index = DraftResourceIndex::new(self.resources.len());
        let resource = DraftResource {
            key: DraftResourceKey {
                role,
                source,
                reference_label: reference_label.map(str::to_owned),
            },
            destination: destination.to_owned(),
            title: title.map(str::to_owned),
        };
        debug_assert_eq!(
            draft_resource_metadata(&resource, self.limits),
            Ok(metadata_bytes)
        );
        self.resources.push(resource);
        if let Some(reference_label) = reference_label {
            self.reference_resources
                .entry(role)
                .or_default()
                .insert(reference_label.to_owned(), index);
        }
        Ok(index)
    }

    fn next_table_column(&self) -> Result<u32, MarkdownError> {
        let owner = self.stack.last().ok_or(MarkdownError::UnexpectedEvent {
            event: "table-cell",
            context: "document",
        })?;
        if !matches!(
            owner.payload,
            FramePayload::TableHead | FramePayload::TableRow
        ) {
            return Err(MarkdownError::UnexpectedEvent {
                event: "table-cell",
                context: owner.payload.name(),
            });
        }
        u32::try_from(owner.children.len())
            .map_err(|_| MarkdownError::NumericOverflow("table column"))
    }

    fn apply_task_marker(&mut self, checked: bool) -> Result<(), MarkdownError> {
        for frame in self.stack.iter_mut().rev() {
            if let FramePayload::Item { checked: current } = &mut frame.payload {
                if current.is_some() {
                    return Err(MarkdownError::UnexpectedEvent {
                        event: "task-list-marker",
                        context: "already-marked-list-item",
                    });
                }
                *current = Some(checked);
                return Ok(());
            }
        }
        Err(MarkdownError::UnexpectedEvent {
            event: "task-list-marker",
            context: "outside-list-item",
        })
    }

    fn finish(self) -> Result<MarkdownCompilation, MarkdownError> {
        if let Some(frame) = self.stack.last() {
            return Err(MarkdownError::UnclosedContainer(frame.payload.name()));
        }
        let forest = DraftForest {
            roots: self.roots,
            resources: self.resources,
            pending_custom_start: self
                .pending_custom_start
                .map(|start| absolute_cursor(start, self.absolute_base))
                .transpose()?,
        };
        let usage = validate_draft_limits(&forest, self.limits)?;
        let baseline = self.budget.baseline();
        debug_assert_eq!(
            DraftUsage {
                roots: baseline.roots.saturating_add(usage.roots),
                nodes: baseline.nodes.saturating_add(usage.nodes),
                resources: baseline.resources.saturating_add(usage.resources),
                structural_items: baseline
                    .structural_items
                    .saturating_add(usage.structural_items),
                metadata_bytes: baseline.metadata_bytes.saturating_add(usage.metadata_bytes),
            },
            self.budget.usage()
        );
        Ok(MarkdownCompilation {
            forest,
            parse_passes: self.parse_passes,
            parsed_source_bytes: self.parsed_source_bytes,
            custom_scan_source_bytes: self.custom_scan_source_bytes,
            pending_custom: self.pending_custom,
        })
    }

    fn ensure_tree_depth(&self, actual: usize) -> Result<(), MarkdownError> {
        if actual > self.limits.max_tree_depth {
            Err(MarkdownError::LimitExceeded {
                field: "tree.depth",
                limit: self.limits.max_tree_depth,
                actual,
            })
        } else {
            Ok(())
        }
    }
}

fn next_merged_event<'source, I>(
    events: &mut I,
    pending: &mut Option<(Event<'source>, Range<usize>)>,
) -> Option<(Event<'source>, Range<usize>)>
where
    I: Iterator<Item = (Event<'source>, Range<usize>)>,
{
    match (pending.take(), events.next()) {
        (
            Some((Event::Text(last_text), last_range)),
            Some((Event::Text(next_text), next_range)),
        ) => {
            let mut text = last_text.into_string();
            text.push_str(&next_text);
            let mut range = last_range;
            range.end = next_range.end;
            loop {
                match events.next() {
                    Some((Event::Text(next_text), next_range)) => {
                        text.push_str(&next_text);
                        range.end = next_range.end;
                    }
                    next_event => {
                        *pending = next_event;
                        if text.is_empty() {
                            return next_merged_event(events, pending);
                        }
                        return Some((Event::Text(CowStr::Boxed(text.into_boxed_str())), range));
                    }
                }
            }
        }
        (None, Some(next_event)) => {
            *pending = Some(next_event);
            next_merged_event(events, pending)
        }
        (None, None) => None,
        (last_event, next_event) => {
            *pending = next_event;
            last_event
        }
    }
}

fn resource_metadata_bytes(
    role: DraftResourceRole,
    reference_label: Option<&str>,
    destination: &str,
    title: Option<&str>,
    limits: ProtocolLimits,
) -> Result<usize, MarkdownError> {
    let mut bytes = 0usize;
    if role == DraftResourceRole::Citation {
        if let Some(label) = reference_label {
            add_resource_metadata(
                &mut bytes,
                "resource.citation.key",
                label.trim_start_matches('@'),
                limits,
            )?;
        }
    }
    add_resource_metadata(&mut bytes, "resource.destination", destination, limits)?;
    if let Some(title) = title {
        add_resource_metadata(&mut bytes, "resource.title", title, limits)?;
    }
    if bytes > limits.max_node_metadata_bytes {
        return Err(MarkdownError::LimitExceeded {
            field: "resource.metadata",
            limit: limits.max_node_metadata_bytes,
            actual: bytes,
        });
    }
    Ok(bytes)
}

fn add_resource_metadata(
    bytes: &mut usize,
    field: &'static str,
    value: &str,
    limits: ProtocolLimits,
) -> Result<(), MarkdownError> {
    if value.len() > limits.max_metadata_value_bytes {
        return Err(MarkdownError::LimitExceeded {
            field,
            limit: limits.max_metadata_value_bytes,
            actual: value.len(),
        });
    }
    *bytes = bytes
        .checked_add(value.len())
        .ok_or(MarkdownError::NumericOverflow("metadata bytes"))?;
    Ok(())
}
