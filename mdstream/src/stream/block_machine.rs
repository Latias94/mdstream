use super::mode::BlockMode;
use crate::types::BlockId;

#[derive(Debug, Clone)]
pub(super) struct BlockMachine {
    pub(super) processed_line: usize,
    pub(super) current_block_start_line: usize,
    pub(super) current_block_id: BlockId,
    pub(super) next_block_id: u64,
    pub(super) current_mode: BlockMode,
}

impl BlockMachine {
    pub(super) fn new() -> Self {
        Self {
            processed_line: 0,
            current_block_start_line: 0,
            current_block_id: BlockId(1),
            next_block_id: 2,
            current_mode: BlockMode::Unknown,
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    pub(super) fn reset_for_single_block(&mut self, line_count: usize) {
        self.reset();
        self.processed_line = line_count;
    }

    pub(super) fn start_next_block_after(&mut self, end_line_inclusive: usize) {
        self.current_block_start_line = end_line_inclusive + 1;
        self.current_block_id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        self.current_mode = BlockMode::Unknown;
    }
}

impl Default for BlockMachine {
    fn default() -> Self {
        Self::new()
    }
}
