use crate::types::BlockKind;

#[derive(Debug, Clone)]
pub(super) enum BlockMode {
    Unknown,
    Paragraph,
    Heading,
    ThematicBreak,
    CodeFence {
        fence_char: char,
        fence_len: usize,
    },
    CustomBoundary {
        plugin_index: usize,
        started: bool,
    },
    List,
    BlockQuote,
    HtmlBlock {
        stack: Vec<String>,
        in_comment: bool,
    },
    Table,
    MathBlock {
        open_count: usize,
    },
    FootnoteDefinition,
}

impl BlockMode {
    pub(super) fn kind(&self) -> BlockKind {
        match self {
            BlockMode::Paragraph => BlockKind::Paragraph,
            BlockMode::Heading => BlockKind::Heading,
            BlockMode::ThematicBreak => BlockKind::ThematicBreak,
            BlockMode::CodeFence { .. } => BlockKind::CodeFence,
            BlockMode::CustomBoundary { .. } => BlockKind::Unknown,
            BlockMode::List => BlockKind::List,
            BlockMode::BlockQuote => BlockKind::BlockQuote,
            BlockMode::HtmlBlock { .. } => BlockKind::HtmlBlock,
            BlockMode::Table => BlockKind::Table,
            BlockMode::MathBlock { .. } => BlockKind::MathBlock,
            BlockMode::FootnoteDefinition => BlockKind::FootnoteDefinition,
            BlockMode::Unknown => BlockKind::Unknown,
        }
    }
}
