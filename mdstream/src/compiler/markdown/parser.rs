use std::{collections::BTreeMap, ops::Range};

use mdstream_protocol::{ProtocolLimits, SourceCursor, SourceRange};
use pulldown_cmark::{CowStr, Event, Parser, RefDefs, Tag, TagEnd};
use unicase::UniCase;

use crate::compiler::{
    CompilerLimits, CustomBlockSpec,
    custom::{
        CustomBlockMatch, CustomStartContext, PendingCustomState,
        find_custom_blocks_with_node_budget, parse_custom_attributes,
    },
    definitions::DefinitionFact,
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
    definition_topology::merge_definition_nodes,
    frame::{Frame, FrameEnd, FramePayload, collect_semantic_event, end_name},
    limits::{draft_node_metadata, draft_resource_metadata_fields, validate_draft_limits},
    normalization::{
        block_quote_kind, child_hull, citation_key, code_block_header, empty_code_body,
        empty_image_body, extend_range, heading_level, link_contract, list_is_tight,
        markdown_custom_error, offset_range, ordered_list_start, repair_collapsed_range,
        source_contained_body, synthesize_table_body, synthesize_tight_paragraphs,
        synthetic_container, table_alignment, tight_paragraph_count,
        trim_redundant_trailing_blank_lines,
    },
    unresolved_footnotes::{
        classify_unresolved_footnotes, collect_canonical_events, overlay_unresolved_footnotes,
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
        MarkdownConfig::new(&[], ProtocolLimits::default(), CompilerLimits::default()),
        DraftUsage::default(),
        CustomStartContext::DocumentStart,
        true,
    )
    .map(|compilation| compilation.forest)
}

pub(crate) struct MarkdownCompilation {
    pub(crate) forest: DraftForest,
    pub(crate) definitions: Vec<DefinitionFact>,
    pub(crate) parse_passes: u64,
    pub(crate) parsed_source_bytes: u64,
    pub(crate) custom_scan_source_bytes: u64,
    pub(crate) pending_custom: Option<PendingCustomState>,
}

#[derive(Clone, Copy)]
pub(crate) struct MarkdownConfig<'config> {
    custom_blocks: &'config [CustomBlockSpec],
    protocol_limits: ProtocolLimits,
    compiler_limits: CompilerLimits,
}

impl<'config> MarkdownConfig<'config> {
    pub(crate) const fn new(
        custom_blocks: &'config [CustomBlockSpec],
        protocol_limits: ProtocolLimits,
        compiler_limits: CompilerLimits,
    ) -> Self {
        Self {
            custom_blocks,
            protocol_limits,
            compiler_limits,
        }
    }
}

pub(crate) fn compile_markdown_with_custom(
    source: &str,
    absolute_base: SourceCursor,
    config: MarkdownConfig<'_>,
    baseline: DraftUsage,
    custom_start_context: CustomStartContext,
    confirm_eof: bool,
) -> Result<MarkdownCompilation, MarkdownError> {
    absolute_cursor(source.len(), absolute_base)?;
    let mut compiler = MarkdownCompiler::new(
        source,
        absolute_base,
        config,
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

struct MarkdownCompiler<'source, 'config> {
    source: &'source str,
    absolute_base: SourceCursor,
    config: MarkdownConfig<'config>,
    budget: DraftBudget,
    roots: Vec<DraftNode>,
    resources: Vec<DraftResource>,
    definitions: Vec<DefinitionFact>,
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
        config: MarkdownConfig<'config>,
        baseline: DraftUsage,
        custom_start_context: CustomStartContext,
        confirm_eof: bool,
    ) -> Self {
        Self {
            source,
            absolute_base,
            config,
            budget: DraftBudget::new(config.protocol_limits, baseline),
            roots: Vec::new(),
            resources: Vec::new(),
            definitions: Vec::new(),
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
        if self.config.custom_blocks.is_empty() {
            return self.compile_markdown_fragment(0..self.source.len());
        }

        let scan = find_custom_blocks_with_node_budget(
            self.source,
            self.config.custom_blocks,
            self.config.protocol_limits,
            self.budget.usage().nodes,
            self.config.protocol_limits.max_nodes,
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
        self.record_reference_definitions(parser.reference_definitions(), range.start)?;
        if fragment.contains("[^") {
            let events = collect_canonical_events(
                fragment,
                parser.into_offset_iter(),
                self.stack.len(),
                self.config.compiler_limits.max_markdown_events,
                self.config.protocol_limits.max_tree_depth,
            )?;
            self.record_parse(fragment.len())?;
            let unresolved = classify_unresolved_footnotes(
                fragment,
                &events,
                self.config.compiler_limits.max_markdown_events,
            )?;
            let events = overlay_unresolved_footnotes(
                fragment,
                events,
                unresolved,
                self.config.compiler_limits.max_markdown_overlap_work,
            )?;
            self.consume_fragment_events(events, range.start)?;
        } else {
            self.consume_fragment_events(parser.into_offset_iter().map(Ok), range.start)?;
        }
        Ok(())
    }

    fn consume_fragment_events<I>(
        &mut self,
        mut events: I,
        fragment_start: usize,
    ) -> Result<(), MarkdownError>
    where
        I: Iterator<Item = Result<(Event<'source>, Range<usize>), MarkdownError>>,
    {
        let mut pending_event = None;
        let initial_depth = self.stack.len();
        while let Some((event, event_range)) = next_merged_event(&mut events, &mut pending_event)? {
            self.consume(event, offset_range(event_range, fragment_start)?)?;
        }
        if self.stack.len() != initial_depth {
            return Err(MarkdownError::UnclosedContainer("fragment"));
        }
        Ok(())
    }

    fn record_reference_definitions(
        &mut self,
        definitions: &RefDefs<'_>,
        fragment_start: usize,
    ) -> Result<(), MarkdownError> {
        for (label, definition) in definitions.iter() {
            let relative = offset_range(definition.span.clone(), fragment_start)?;
            let source = absolute_range(relative, self.absolute_base)?;
            let label = label.to_string();
            let destination = definition.dest.to_string();
            let title = definition.title.as_ref().map(|title| title.to_string());
            self.definitions.push(DefinitionFact::reference(
                label.clone(),
                source,
                destination.clone(),
                title.clone(),
            ));
            if let Some(citation) = DefinitionFact::citation(label, source, destination, title) {
                self.definitions.push(citation);
            }
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
            .config
            .custom_blocks
            .get(block.spec_index)
            .ok_or(MarkdownError::Unsupported("custom-block-spec"))?;
        let attributes =
            parse_custom_attributes(block.attributes, spec, self.config.protocol_limits)
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

    fn consume(&mut self, event: Event<'source>, range: Range<usize>) -> Result<(), MarkdownError> {
        checked_slice(self.source, range.clone())?;

        if self
            .stack
            .last()
            .is_some_and(|frame| frame.payload.is_collector())
        {
            return self.consume_collected(event, range);
        }

        match event {
            Event::Start(tag) => self.open(tag, range),
            Event::End(end) => self.close(end),
            Event::Text(value) => self.push_text(range, value.as_ref()),
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
                        label: UniCase::new(label.as_ref()).to_folded_case(),
                        target: None,
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

    fn push_text(&mut self, range: Range<usize>, value: &str) -> Result<(), MarkdownError> {
        self.budget.reserve_node(0)?;
        let raw = checked_slice(self.source, range.clone())?;
        self.push_leaf_reserved(
            range.clone(),
            range,
            DraftContentKind::Text {
                text: semantic_text(raw, value),
            },
        )
    }

    fn open(&mut self, tag: Tag<'source>, mut range: Range<usize>) -> Result<(), MarkdownError> {
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
                label: UniCase::new(label.as_ref()).to_folded_case(),
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
                if let Some(key) = citation_key(link_type, reference_label.as_deref()) {
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
                            reference_label.as_deref(),
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
                let target = if resolved {
                    Some(self.push_resource(
                        DraftResourceRole::Image,
                        range.clone(),
                        reference_label.as_deref(),
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
        if matches!(&frame.payload, FramePayload::FootnoteDefinition { .. }) {
            trim_redundant_trailing_blank_lines(self.source, &mut frame.source)?;
        }
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
                        DraftContentKind::FootnoteDefinition {
                            label,
                            target: None,
                        }
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
        if !root && child_count > self.config.protocol_limits.max_children_per_list {
            return Err(MarkdownError::LimitExceeded {
                field: "children",
                limit: self.config.protocol_limits.max_children_per_list,
                actual: child_count,
            });
        }
        let metadata_bytes = draft_node_metadata(&node.content, self.config.protocol_limits)?;
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
        let metadata_bytes = draft_resource_metadata_fields(
            role,
            reference_label,
            destination,
            title,
            self.config.protocol_limits,
        )?;
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

    fn finish(mut self) -> Result<MarkdownCompilation, MarkdownError> {
        if let Some(frame) = self.stack.last() {
            return Err(MarkdownError::UnclosedContainer(frame.payload.name()));
        }
        let mut forest = DraftForest {
            roots: self.roots,
            resources: self.resources,
            pending_custom_start: self
                .pending_custom_start
                .map(|start| absolute_cursor(start, self.absolute_base))
                .transpose()?,
        };
        let mut definition_nodes = Vec::new();
        let mut definition_metadata = Vec::new();
        for definition in &self.definitions {
            let Some(key) = definition.citation_key() else {
                continue;
            };
            self.budget.reserve_node(0)?;
            let node = DraftNode::leaf(
                definition.source,
                definition.source,
                DraftContentKind::CitationDefinition {
                    key: key.to_string(),
                    target: None,
                },
            );
            let metadata_bytes = draft_node_metadata(&node.content, self.config.protocol_limits)?;
            definition_nodes.push(node);
            definition_metadata.push(metadata_bytes);
        }
        definition_nodes.sort_by_key(|node| (node.source.start, node.source.end));
        let root_definitions = merge_definition_nodes(&mut forest.roots, definition_nodes, true)?;
        for (index, metadata_bytes) in definition_metadata.into_iter().enumerate() {
            self.budget
                .reserve_node_payload(index < root_definitions, metadata_bytes)?;
        }
        let usage = validate_draft_limits(&forest, self.config.protocol_limits)?;
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
            definitions: self.definitions,
            parse_passes: self.parse_passes,
            parsed_source_bytes: self.parsed_source_bytes,
            custom_scan_source_bytes: self.custom_scan_source_bytes,
            pending_custom: self.pending_custom,
        })
    }

    fn ensure_tree_depth(&self, actual: usize) -> Result<(), MarkdownError> {
        if actual > self.config.protocol_limits.max_tree_depth {
            Err(MarkdownError::LimitExceeded {
                field: "tree.depth",
                limit: self.config.protocol_limits.max_tree_depth,
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
) -> Result<Option<(Event<'source>, Range<usize>)>, MarkdownError>
where
    I: Iterator<Item = Result<(Event<'source>, Range<usize>), MarkdownError>>,
{
    match (pending.take(), events.next().transpose()?) {
        (
            Some((Event::Text(last_text), last_range)),
            Some((Event::Text(next_text), next_range)),
        ) => {
            let mut text = last_text.into_string();
            text.push_str(&next_text);
            let mut range = last_range;
            range.end = next_range.end;
            loop {
                match events.next().transpose()? {
                    Some((Event::Text(next_text), next_range)) => {
                        text.push_str(&next_text);
                        range.end = next_range.end;
                    }
                    next_event => {
                        *pending = next_event;
                        if text.is_empty() {
                            return next_merged_event(events, pending);
                        }
                        return Ok(Some((
                            Event::Text(CowStr::Boxed(text.into_boxed_str())),
                            range,
                        )));
                    }
                }
            }
        }
        (None, Some(next_event)) => {
            *pending = Some(next_event);
            next_merged_event(events, pending)
        }
        (None, None) => Ok(None),
        (last_event, next_event) => {
            *pending = next_event;
            Ok(last_event)
        }
    }
}
