use std::ops::Range;

use mdstream_protocol::{BlockQuoteKind, CodeBlockSyntax, LinkStyle, TableAlignment};
use pulldown_cmark::{Event, TagEnd};

use crate::compiler::draft::{DraftContentKind, DraftNode, DraftResourceIndex};

use super::MarkdownError;

pub(super) struct Frame {
    pub(super) expected: FrameEnd,
    pub(super) source: Range<usize>,
    pub(super) payload: FramePayload,
    pub(super) children: Vec<DraftNode>,
    pub(super) collector_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FrameEnd {
    Parser(TagEnd),
    Custom,
}

impl FrameEnd {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Parser(end) => end_name(end),
            Self::Custom => "custom-block",
        }
    }
}

pub(super) enum FramePayload {
    Paragraph,
    Heading {
        level: u8,
    },
    BlockQuote {
        style: BlockQuoteKind,
    },
    CodeBlock {
        syntax: CodeBlockSyntax,
        info: Option<String>,
        text: String,
        body: Option<Range<usize>>,
    },
    HtmlBlock {
        text: String,
        body: Option<Range<usize>>,
    },
    Custom {
        content: DraftContentKind,
        body: Range<usize>,
    },
    List {
        ordered: bool,
        start: Option<u32>,
    },
    Item {
        checked: Option<bool>,
    },
    FootnoteDefinition {
        label: String,
    },
    Table {
        alignments: Vec<TableAlignment>,
    },
    TableHead,
    TableRow,
    TableCell {
        column: u32,
    },
    Emphasis,
    Strong,
    Strikethrough,
    Link {
        target: Option<DraftResourceIndex>,
        reference_label: Option<String>,
        style: LinkStyle,
    },
    LiteralLink {
        body: Option<Range<usize>>,
    },
    CitationReference {
        key: String,
        target: Option<DraftResourceIndex>,
    },
    Image {
        target: Option<DraftResourceIndex>,
        reference_label: Option<String>,
        style: LinkStyle,
        alt: String,
        body: Option<Range<usize>>,
    },
}

impl FramePayload {
    pub(super) const fn is_collector(&self) -> bool {
        matches!(
            self,
            Self::CodeBlock { .. }
                | Self::HtmlBlock { .. }
                | Self::LiteralLink { .. }
                | Self::Image { .. }
        )
    }

    pub(super) const fn prohibits_nested_link(&self) -> bool {
        matches!(self, Self::Link { .. } | Self::CitationReference { .. })
    }

    pub(super) fn collector_body_mut(
        &mut self,
    ) -> Result<&mut Option<Range<usize>>, MarkdownError> {
        match self {
            Self::CodeBlock { body, .. }
            | Self::HtmlBlock { body, .. }
            | Self::LiteralLink { body }
            | Self::Image { body, .. } => Ok(body),
            _ => Err(MarkdownError::UnexpectedEvent {
                event: "collector-body",
                context: self.name(),
            }),
        }
    }

    pub(super) const fn name(&self) -> &'static str {
        match self {
            Self::Paragraph => "paragraph",
            Self::Heading { .. } => "heading",
            Self::BlockQuote { .. } => "block-quote",
            Self::CodeBlock { .. } => "code-block",
            Self::HtmlBlock { .. } => "html-block",
            Self::Custom { .. } => "custom-block",
            Self::List { .. } => "list",
            Self::Item { .. } => "list-item",
            Self::FootnoteDefinition { .. } => "footnote-definition",
            Self::Table { .. } => "table",
            Self::TableHead => "table-head",
            Self::TableRow => "table-row",
            Self::TableCell { .. } => "table-cell",
            Self::Emphasis => "emphasis",
            Self::Strong => "strong",
            Self::Strikethrough => "strikethrough",
            Self::Link { .. } => "link",
            Self::LiteralLink { .. } => "literal-link",
            Self::CitationReference { .. } => "citation-reference",
            Self::Image { .. } => "image",
        }
    }
}

pub(super) fn collect_semantic_event(
    payload: &mut FramePayload,
    event: Event<'_>,
) -> Result<(), MarkdownError> {
    match payload {
        FramePayload::CodeBlock { text, .. } => match event {
            Event::Text(value) => text.push_str(&value),
            _ => {
                return Err(MarkdownError::UnexpectedEvent {
                    event: event_name(&event),
                    context: "code-block",
                });
            }
        },
        FramePayload::HtmlBlock { text, .. } => match event {
            Event::Html(value) | Event::InlineHtml(value) | Event::Text(value) => {
                text.push_str(&value);
            }
            _ => {
                return Err(MarkdownError::UnexpectedEvent {
                    event: event_name(&event),
                    context: "html-block",
                });
            }
        },
        FramePayload::LiteralLink { .. } => {}
        FramePayload::Image { alt, .. } => match event {
            Event::Text(value) | Event::Code(value) | Event::InlineHtml(value) => {
                alt.push_str(&value);
            }
            Event::Html(_) => {}
            Event::InlineMath(value) => {
                alt.push('$');
                alt.push_str(&value);
                alt.push('$');
            }
            Event::DisplayMath(value) => {
                alt.push_str("$$");
                alt.push_str(&value);
                alt.push_str("$$");
            }
            Event::FootnoteReference(label) => {
                alt.push_str("[^");
                alt.push_str(&label);
                alt.push(']');
            }
            Event::SoftBreak | Event::HardBreak | Event::Rule => alt.push(' '),
            Event::TaskListMarker(true) => alt.push_str("[x]"),
            Event::TaskListMarker(false) => alt.push_str("[ ]"),
            Event::Start(_) | Event::End(_) => {
                return Err(MarkdownError::UnexpectedEvent {
                    event: event_name(&event),
                    context: "image-collector-value",
                });
            }
        },
        _ => {
            return Err(MarkdownError::UnexpectedEvent {
                event: event_name(&event),
                context: payload.name(),
            });
        }
    }
    Ok(())
}

fn event_name(event: &Event<'_>) -> &'static str {
    match event {
        Event::Start(_) => "start",
        Event::End(_) => "end",
        Event::Text(_) => "text",
        Event::Code(_) => "inline-code",
        Event::InlineMath(_) => "inline-math",
        Event::DisplayMath(_) => "display-math",
        Event::Html(_) => "html",
        Event::InlineHtml(_) => "inline-html",
        Event::FootnoteReference(_) => "footnote-reference",
        Event::SoftBreak => "soft-break",
        Event::HardBreak => "hard-break",
        Event::Rule => "rule",
        Event::TaskListMarker(_) => "task-list-marker",
    }
}

pub(super) const fn end_name(end: TagEnd) -> &'static str {
    match end {
        TagEnd::Paragraph => "paragraph",
        TagEnd::Heading(_) => "heading",
        TagEnd::BlockQuote(_) => "block-quote",
        TagEnd::CodeBlock => "code-block",
        TagEnd::HtmlBlock => "html-block",
        TagEnd::List(_) => "list",
        TagEnd::Item => "list-item",
        TagEnd::FootnoteDefinition => "footnote-definition",
        TagEnd::DefinitionList | TagEnd::DefinitionListTitle | TagEnd::DefinitionListDefinition => {
            "definition-list"
        }
        TagEnd::Table => "table",
        TagEnd::TableHead => "table-head",
        TagEnd::TableRow => "table-row",
        TagEnd::TableCell => "table-cell",
        TagEnd::Emphasis => "emphasis",
        TagEnd::Strong => "strong",
        TagEnd::Strikethrough => "strikethrough",
        TagEnd::Superscript => "superscript",
        TagEnd::Subscript => "subscript",
        TagEnd::Link => "link",
        TagEnd::Image => "image",
        TagEnd::MetadataBlock(_) => "metadata-block",
    }
}
