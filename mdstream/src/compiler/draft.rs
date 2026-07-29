use std::collections::BTreeMap;

use mdstream_protocol::{
    BlockQuoteKind, CodeBlockSyntax, LinkStyle, SemanticText, SourceRange, TableAlignment,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct DraftResourceIndex(usize);

impl DraftResourceIndex {
    pub(crate) const fn new(value: usize) -> Self {
        Self(value)
    }

    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum DraftResourceRole {
    Link,
    Image,
    Footnote,
    Citation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DraftResourceKey {
    pub(crate) role: DraftResourceRole,
    pub(crate) source: SourceRange,
    pub(crate) reference_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DraftResource {
    pub(crate) key: DraftResourceKey,
    pub(crate) destination: String,
    pub(crate) title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum SyntheticRole {
    TightParagraph,
    TableHeaderRow,
    TableBody,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum DraftOriginHint {
    #[default]
    Parsed,
    Synthetic(SyntheticRole),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DraftContentKind {
    Paragraph,
    Heading {
        level: u8,
    },
    Text {
        text: SemanticText,
    },
    Emphasis,
    Strong,
    Strikethrough,
    Link {
        target: Option<DraftResourceIndex>,
        reference_label: Option<String>,
        style: LinkStyle,
    },
    CitationReference {
        key: String,
        target: Option<DraftResourceIndex>,
    },
    Image {
        target: Option<DraftResourceIndex>,
        reference_label: Option<String>,
        style: LinkStyle,
        alt: SemanticText,
    },
    InlineCode {
        text: SemanticText,
    },
    CodeBlock {
        syntax: CodeBlockSyntax,
        info: Option<String>,
        text: SemanticText,
    },
    List {
        ordered: bool,
        start: Option<u32>,
        tight: bool,
    },
    ListItem {
        checked: Option<bool>,
    },
    BlockQuote {
        style: BlockQuoteKind,
    },
    ThematicBreak,
    Table {
        alignments: Vec<TableAlignment>,
    },
    TableHead,
    TableBody,
    TableRow,
    TableCell {
        column: u32,
    },
    Html {
        block: bool,
        text: SemanticText,
    },
    Custom {
        namespace: String,
        name: String,
        opaque: bool,
        attributes: BTreeMap<String, String>,
    },
    Math {
        display: bool,
        text: SemanticText,
    },
    FootnoteDefinition {
        label: String,
        target: Option<DraftResourceIndex>,
    },
    FootnoteReference {
        label: String,
        target: Option<DraftResourceIndex>,
    },
    CitationDefinition {
        key: String,
        target: Option<DraftResourceIndex>,
    },
    SoftBreak,
    HardBreak,
}

impl DraftContentKind {
    pub(crate) const fn is_phrasing(&self) -> bool {
        matches!(
            self,
            Self::Text { .. }
                | Self::Emphasis
                | Self::Strong
                | Self::Strikethrough
                | Self::Link { .. }
                | Self::Image { .. }
                | Self::CitationReference { .. }
                | Self::InlineCode { .. }
                | Self::Math { .. }
                | Self::FootnoteReference { .. }
                | Self::SoftBreak
                | Self::HardBreak
                | Self::Html { block: false, .. }
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct DraftNode {
    pub(crate) source: SourceRange,
    pub(crate) body: SourceRange,
    pub(crate) origin: DraftOriginHint,
    pub(crate) content: DraftContentKind,
    pub(crate) children: Vec<Self>,
}

impl DraftNode {
    pub(crate) fn leaf(source: SourceRange, body: SourceRange, content: DraftContentKind) -> Self {
        Self {
            source,
            body,
            origin: DraftOriginHint::Parsed,
            content,
            children: Vec::new(),
        }
    }

    pub(crate) fn container(
        source: SourceRange,
        body: SourceRange,
        origin: DraftOriginHint,
        content: DraftContentKind,
        children: Vec<Self>,
    ) -> Self {
        Self {
            source,
            body,
            origin,
            content,
            children,
        }
    }
}

impl Drop for DraftNode {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.children);
        while let Some(mut node) = pending.pop() {
            pending.append(&mut node.children);
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DraftForest {
    pub(crate) roots: Vec<DraftNode>,
    pub(crate) resources: Vec<DraftResource>,
    pub(crate) pending_custom_start: Option<mdstream_protocol::SourceCursor>,
}

#[cfg(test)]
mod tests {
    use mdstream_protocol::{SourceCursor, SourceRange};

    use super::*;

    #[test]
    fn deeply_nested_drafts_drop_iteratively_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
                let mut root = DraftNode::leaf(range, range, DraftContentKind::Paragraph);
                for _ in 0..50_000 {
                    root = DraftNode::container(
                        range,
                        range,
                        DraftOriginHint::Parsed,
                        DraftContentKind::Paragraph,
                        vec![root],
                    );
                }
                drop(root);
            })
            .expect("small-stack draft thread must start")
            .join()
            .expect("iterative draft drop must not overflow the stack");
    }
}
