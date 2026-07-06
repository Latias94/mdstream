mod footnotes;
mod references;

use crate::options::ReferenceDefinitionsMode;
use crate::types::{Block, BlockId};

use self::footnotes::FootnoteSemantics;
use self::references::ReferenceSemantics;

#[derive(Debug, Default)]
pub(crate) struct DocumentSemantics {
    footnotes: FootnoteSemantics,
    references: ReferenceSemantics,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CommitEffects {
    pub(crate) invalidated: Vec<BlockId>,
}

impl DocumentSemantics {
    pub(crate) fn footnotes_detected(&self) -> bool {
        self.footnotes.detected()
    }

    pub(crate) fn observe_chunk_for_footnotes(&mut self, chunk: &str) {
        self.footnotes.observe_chunk(chunk);
    }

    pub(crate) fn observe_committed_block(
        &mut self,
        block: &Block,
        reference_mode: ReferenceDefinitionsMode,
    ) -> CommitEffects {
        CommitEffects {
            invalidated: self
                .references
                .observe_committed_block(block, reference_mode),
        }
    }

    pub(crate) fn clear_references(&mut self) {
        self.references.clear();
    }

    pub(crate) fn reset(&mut self) {
        self.footnotes.reset();
        self.references.clear();
    }
}
