use crate::options::ReferenceDefinitionsMode;
use crate::reference::ReferenceIndex;
use crate::types::{Block, BlockId};

#[derive(Debug, Default)]
pub(crate) struct ReferenceSemantics {
    index: ReferenceIndex,
}

impl ReferenceSemantics {
    pub(crate) fn observe_committed_block(
        &mut self,
        block: &Block,
        reference_mode: ReferenceDefinitionsMode,
    ) -> Vec<BlockId> {
        self.index.observe_committed_block(block, reference_mode)
    }

    pub(crate) fn clear(&mut self) {
        self.index.clear();
    }
}
