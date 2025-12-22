use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u64);

impl fmt::Debug for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlockId({})", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockStatus {
    Committed,
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading,
    ThematicBreak,
    CodeFence,
    List,
    BlockQuote,
    Table,
    HtmlBlock,
    MathBlock,
    FootnoteDefinition,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
    pub status: BlockStatus,
    pub kind: BlockKind,
    pub raw: String,
    /// Optional display string for pending blocks (remend-like termination, JSON repair, etc.).
    pub display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub committed: Vec<Block>,
    pub pending: Option<Block>,
    /// Optional list of committed block IDs that adapters may want to re-parse.
    ///
    /// Note: populated in post-MVP invalidation mode.
    pub invalidated: Vec<BlockId>,
}

impl Update {
    pub fn empty() -> Self {
        Self {
            committed: Vec::new(),
            pending: None,
            invalidated: Vec::new(),
        }
    }
}

