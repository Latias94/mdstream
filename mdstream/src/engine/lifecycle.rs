use std::fmt;

use mdstream_protocol::{
    ChangeId, ChangeSet, Epoch, ProjectionOp, ProtocolError, Reducer, Sequence, SourceCursor,
    SourceDelta,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    Finished,
    EpochOverflow,
    SequenceOverflow,
    CursorOverflow,
    Protocol(ProtocolError),
    InternalInvariant(ProtocolError),
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finished => formatter.write_str("stream engine is finalized"),
            Self::EpochOverflow => formatter.write_str("stream engine epoch overflowed"),
            Self::SequenceOverflow => formatter.write_str("stream engine sequence overflowed"),
            Self::CursorOverflow => formatter.write_str("stream engine source cursor overflowed"),
            Self::Protocol(error) => write!(formatter, "stream input violates protocol: {error}"),
            Self::InternalInvariant(error) => {
                write!(
                    formatter,
                    "stream engine generated an invalid change: {error}"
                )
            }
        }
    }
}

impl std::error::Error for EngineError {}

pub(super) fn source_end(reducer: &Reducer, suffix: &str) -> Result<SourceCursor, EngineError> {
    let start = reducer.document().map_or(SourceCursor::new(0), |document| {
        document.coordinate().source_cursor
    });
    let suffix_len = u64::try_from(suffix.len()).map_err(|_| EngineError::CursorOverflow)?;
    start
        .checked_add(suffix_len)
        .ok_or(EngineError::CursorOverflow)
}

pub(super) fn append_change(
    reducer: &Reducer,
    initial_epoch: Epoch,
    suffix: String,
    operations: Vec<ProjectionOp>,
) -> Result<ChangeSet, EngineError> {
    if let Some(document) = reducer.document() {
        let coordinate = document.coordinate();
        let sequence = coordinate
            .sequence
            .checked_add(1)
            .ok_or(EngineError::SequenceOverflow)?;
        deterministic_change(
            coordinate.epoch,
            sequence,
            None,
            SourceDelta::append(coordinate.source_cursor, suffix),
            operations,
        )
    } else {
        deterministic_change(
            initial_epoch,
            Sequence::new(0),
            Some(None),
            SourceDelta::append(SourceCursor::new(0), suffix),
            operations,
        )
    }
}

pub(super) fn reset_change(
    reducer: &Reducer,
    initial_epoch: Epoch,
) -> Result<ChangeSet, EngineError> {
    let (epoch, predecessor) = if let Some(document) = reducer.document() {
        (
            document
                .coordinate()
                .epoch
                .checked_add(1)
                .ok_or(EngineError::EpochOverflow)?,
            Some(document.coordinate().clone()),
        )
    } else {
        (initial_epoch, None)
    };
    deterministic_change(
        epoch,
        Sequence::new(0),
        Some(predecessor),
        SourceDelta::unchanged(SourceCursor::new(0)),
        Vec::new(),
    )
}

fn deterministic_change(
    epoch: Epoch,
    sequence: Sequence,
    epoch_start: Option<Option<mdstream_protocol::Coordinate>>,
    source: SourceDelta,
    operations: Vec<ProjectionOp>,
) -> Result<ChangeSet, EngineError> {
    let placeholder = ChangeId::new("engine:pending")
        .expect("the static engine placeholder is a valid change ID");
    let probe = construct(
        epoch,
        sequence,
        placeholder,
        epoch_start.clone(),
        source.clone(),
        operations.clone(),
    )?;
    let change_id = ChangeId::digest(probe.payload_digest().as_str().as_bytes());
    construct(epoch, sequence, change_id, epoch_start, source, operations)
}

fn construct(
    epoch: Epoch,
    sequence: Sequence,
    change_id: ChangeId,
    epoch_start: Option<Option<mdstream_protocol::Coordinate>>,
    source: SourceDelta,
    operations: Vec<ProjectionOp>,
) -> Result<ChangeSet, EngineError> {
    match epoch_start {
        Some(predecessor) => {
            ChangeSet::start_epoch(epoch, change_id, predecessor, source, operations)
        }
        None => ChangeSet::new(epoch, sequence, change_id, source, operations),
    }
    .map_err(EngineError::InternalInvariant)
}
