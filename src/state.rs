use crate::types::{AppliedUpdate, Block, BlockId, Update};

/// A UI-friendly document state container for streaming Markdown.
///
/// This keeps only the stable, renderable state:
/// - committed blocks (append-only)
/// - an optional pending block (can change every tick)
///
/// It intentionally does not own the parser (`MdStream`) to stay render- and pipeline-agnostic.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DocumentState {
    committed: Vec<Block>,
    pending: Option<Block>,
}

impl DocumentState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn committed(&self) -> &[Block] {
        &self.committed
    }

    pub fn pending(&self) -> Option<&Block> {
        self.pending.as_ref()
    }

    pub fn pending_mut(&mut self) -> Option<&mut Block> {
        self.pending.as_mut()
    }

    pub fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.committed.iter().chain(self.pending.iter())
    }

    pub fn clear(&mut self) {
        self.committed.clear();
        self.pending = None;
    }

    pub fn apply(&mut self, update: Update) -> AppliedUpdate {
        update.apply_to(&mut self.committed, &mut self.pending)
    }

    pub fn find_committed(&self, id: BlockId) -> Option<&Block> {
        self.committed.iter().find(|b| b.id == id)
    }

    pub fn find_committed_mut(&mut self, id: BlockId) -> Option<&mut Block> {
        self.committed.iter_mut().find(|b| b.id == id)
    }
}
