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

impl Block {
    pub fn display_or_raw(&self) -> &str {
        self.display.as_deref().unwrap_or(&self.raw)
    }

    pub fn code_fence_header(&self) -> Option<crate::syntax::CodeFenceHeader<'_>> {
        if self.kind != BlockKind::CodeFence {
            return None;
        }
        crate::syntax::parse_code_fence_header_from_block(&self.raw)
    }

    pub fn code_fence_language(&self) -> Option<&str> {
        self.code_fence_header().and_then(|h| h.language)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub committed: Vec<Block>,
    pub pending: Option<Block>,
    /// If true, consumers must drop all previously rendered state and rebuild from this update.
    ///
    /// This is used for scope-driven transitions that inherently require a full re-parse (e.g.
    /// when switching into single-block footnote mode after detecting `[^id]` / `[^id]:`).
    pub reset: bool,
    /// Optional list of committed block IDs that adapters may want to re-parse.
    ///
    /// Note: populated in post-MVP invalidation mode.
    pub invalidated: Vec<BlockId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedUpdate {
    pub reset: bool,
    pub invalidated: Vec<BlockId>,
}

impl Update {
    pub fn empty() -> Self {
        Self {
            committed: Vec::new(),
            pending: None,
            reset: false,
            invalidated: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.committed.is_empty()
            && self.pending.is_none()
            && !self.reset
            && self.invalidated.is_empty()
    }

    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.committed.iter().chain(self.pending.iter())
    }

    pub fn apply_to(
        self,
        committed: &mut Vec<Block>,
        pending: &mut Option<Block>,
    ) -> AppliedUpdate {
        if self.reset {
            committed.clear();
            *pending = None;
        }
        committed.extend(self.committed);
        *pending = self.pending;
        AppliedUpdate {
            reset: self.reset,
            invalidated: self.invalidated,
        }
    }
}
