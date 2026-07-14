use mdstream_protocol::SourceCursor;

pub(crate) const INITIAL_CHECKPOINT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompileReason {
    InitialProjection,
    StructuralBoundary,
    Checkpoint,
    Finish,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CheckpointGate {
    next_checkpoint: usize,
    last_compiled_revision: Option<SourceCursor>,
}

impl Default for CheckpointGate {
    fn default() -> Self {
        Self {
            next_checkpoint: INITIAL_CHECKPOINT,
            last_compiled_revision: None,
        }
    }
}

impl CheckpointGate {
    pub(crate) fn reason(
        self,
        revision: SourceCursor,
        frontier_bytes: usize,
        needs_initial_projection: bool,
        structural_boundary: bool,
        finishing: bool,
    ) -> Option<CompileReason> {
        if frontier_bytes == 0 || self.last_compiled_revision == Some(revision) {
            return None;
        }
        if finishing {
            return Some(CompileReason::Finish);
        }
        if needs_initial_projection {
            return Some(CompileReason::InitialProjection);
        }
        if structural_boundary {
            return Some(CompileReason::StructuralBoundary);
        }
        if frontier_bytes >= self.next_checkpoint {
            return Some(CompileReason::Checkpoint);
        }
        None
    }

    pub(crate) fn record_compile(
        &mut self,
        revision: SourceCursor,
        remaining_frontier_bytes: usize,
    ) {
        self.last_compiled_revision = Some(revision);
        self.next_checkpoint = checkpoint_after(remaining_frontier_bytes);
    }

    pub(crate) fn next_checkpoint(self) -> usize {
        self.next_checkpoint
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

fn checkpoint_after(frontier_bytes: usize) -> usize {
    frontier_bytes
        .checked_add(1)
        .and_then(usize::checked_next_power_of_two)
        .unwrap_or(usize::MAX)
        .max(INITIAL_CHECKPOINT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_consumes_all_crossed_checkpoints_in_one_compile() {
        let mut gate = CheckpointGate::default();
        assert_eq!(
            gate.reason(SourceCursor::new(65_536), 65_536, true, false, false),
            Some(CompileReason::InitialProjection)
        );

        gate.record_compile(SourceCursor::new(65_536), 65_536);

        assert_eq!(gate.next_checkpoint(), 131_072);
        assert_eq!(
            gate.reason(SourceCursor::new(65_536), 65_536, true, true, true),
            None
        );
    }

    #[test]
    fn checkpoint_is_relative_to_the_remaining_frontier() {
        let mut gate = CheckpointGate::default();
        gate.record_compile(SourceCursor::new(1_024), 300);
        assert_eq!(gate.next_checkpoint(), 512);
        assert_eq!(
            gate.reason(SourceCursor::new(1_235), 511, false, false, false),
            None
        );
        assert_eq!(
            gate.reason(SourceCursor::new(1_236), 512, false, false, false),
            Some(CompileReason::Checkpoint)
        );
    }

    #[test]
    fn finish_reuses_a_compile_of_the_same_revision() {
        let mut gate = CheckpointGate::default();
        gate.record_compile(SourceCursor::new(7), 7);
        assert_eq!(
            gate.reason(SourceCursor::new(7), 7, false, false, true),
            None
        );
        assert_eq!(
            gate.reason(SourceCursor::new(8), 8, false, false, true),
            Some(CompileReason::Finish)
        );
    }

    #[test]
    fn initial_projection_is_an_explicit_frontier_state() {
        let gate = CheckpointGate::default();

        assert_eq!(
            gate.reason(SourceCursor::new(1), 1, true, false, false),
            Some(CompileReason::InitialProjection)
        );
        assert_eq!(
            gate.reason(SourceCursor::new(2), 2, false, false, false),
            None
        );
    }

    #[test]
    fn next_checkpoint_is_strict_and_saturates_at_address_space_exhaustion() {
        assert_eq!(checkpoint_after(0), INITIAL_CHECKPOINT);
        assert_eq!(checkpoint_after(INITIAL_CHECKPOINT - 1), INITIAL_CHECKPOINT);
        assert_eq!(checkpoint_after(INITIAL_CHECKPOINT), 2 * INITIAL_CHECKPOINT);
        assert_eq!(checkpoint_after(usize::MAX), usize::MAX);
    }
}
