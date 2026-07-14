use std::fmt;

use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, ChildList, ContentNode, Coordinate, DocumentLifecycle,
    Epoch, NodeId, ProjectionOp, ProtocolError, ProtocolMaturity, Reducer, ResourceId,
    SchemaVersion, SemanticResource, Sequence, Snapshot, SourceCursor, SourceDelta,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolTrace {
    pub id: String,
    pub schedule: String,
    /// Leading setup changes before the fixture source starts, for example an
    /// earlier epoch used by a reset trace.
    #[serde(default)]
    pub setup_changes: usize,
    /// Exact append/finalize events and the cumulative change boundary after
    /// each event. This represents coalescing, fanout, and empty no-op appends.
    pub input_events: Vec<TraceInputEvent>,
    pub changes: Vec<ChangeSet>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum TraceInputEvent {
    Append { chunk: String, change_end: usize },
    Reset { change_end: usize },
    Finish { change_end: usize },
}

impl TraceInputEvent {
    pub const fn change_end(&self) -> usize {
        match self {
            Self::Append { change_end, .. }
            | Self::Reset { change_end }
            | Self::Finish { change_end } => *change_end,
        }
    }
}

/// Builds the temporary source-only protocol bridge used before the 0.4 engine
/// begins emitting projection operations. Every chunk becomes one continuous
/// source append and the trace ends with an explicit finalization change.
pub fn source_only_trace<I, S>(
    id: impl Into<String>,
    schedule: impl Into<String>,
    epoch: Epoch,
    chunks: I,
) -> Result<ProtocolTrace, ProtocolError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let input_chunks = chunks
        .into_iter()
        .map(|chunk| chunk.as_ref().to_string())
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut input_events = Vec::with_capacity(input_chunks.len() + 1);
    let mut cursor = 0usize;

    for chunk in &input_chunks {
        if !chunk.is_empty() {
            if changes.is_empty() {
                changes.push(ChangeSet::start_epoch(
                    epoch,
                    ChangeId::new("conformance:start").expect("static change ID is valid"),
                    None,
                    SourceDelta::append(SourceCursor::new(0), chunk),
                    vec![],
                )?);
            } else {
                let sequence =
                    u64::try_from(changes.len()).map_err(|_| ProtocolError::CursorOverflow)?;
                changes.push(ChangeSet::new(
                    epoch,
                    Sequence::new(sequence),
                    ChangeId::new(format!("conformance:{sequence}"))
                        .expect("bounded decimal sequence forms a valid change ID"),
                    SourceDelta::append(
                        SourceCursor::new(
                            u64::try_from(cursor).map_err(|_| ProtocolError::CursorOverflow)?,
                        ),
                        chunk,
                    ),
                    vec![],
                )?);
            }
            cursor = cursor
                .checked_add(chunk.len())
                .ok_or(ProtocolError::CursorOverflow)?;
        }
        input_events.push(TraceInputEvent::Append {
            chunk: chunk.clone(),
            change_end: changes.len(),
        });
    }

    if changes.is_empty() {
        changes.push(ChangeSet::start_epoch(
            epoch,
            ChangeId::new("conformance:finish-empty").expect("static change ID is valid"),
            None,
            SourceDelta::unchanged(SourceCursor::new(0)),
            vec![ProjectionOp::FinishDocument],
        )?);
    } else {
        let finish_sequence =
            u64::try_from(changes.len()).map_err(|_| ProtocolError::CursorOverflow)?;
        changes.push(ChangeSet::new(
            epoch,
            Sequence::new(finish_sequence),
            ChangeId::new("conformance:finish").expect("static change ID is valid"),
            SourceDelta::unchanged(SourceCursor::new(
                u64::try_from(cursor).map_err(|_| ProtocolError::CursorOverflow)?,
            )),
            vec![ProjectionOp::FinishDocument],
        )?);
    }
    input_events.push(TraceInputEvent::Finish {
        change_end: changes.len(),
    });

    Ok(ProtocolTrace {
        id: id.into(),
        schedule: schedule.into(),
        setup_changes: 0,
        input_events,
        changes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredCheckpoint {
    pub id: String,
    pub trace: String,
    /// Zero-based index of the change after which this checkpoint is observed.
    pub after_change: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate: Option<Coordinate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<DocumentLifecycle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_snapshot: Option<NormalizedSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedSnapshot {
    pub schema: SchemaVersion,
    pub maturity: ProtocolMaturity,
    pub epoch: Epoch,
    pub lifecycle: DocumentLifecycle,
    pub source: String,
    pub roots: ChildList,
    pub nodes: Vec<ContentNode>,
    pub resources: Vec<SemanticResource>,
    pub next_node_id: NodeId,
    pub next_resource_id: ResourceId,
}

impl From<&Snapshot> for NormalizedSnapshot {
    fn from(snapshot: &Snapshot) -> Self {
        Self {
            schema: snapshot.schema().clone(),
            maturity: snapshot.maturity(),
            epoch: snapshot.coordinate().epoch,
            lifecycle: snapshot.lifecycle(),
            source: snapshot.source().to_string(),
            roots: snapshot.roots().clone(),
            nodes: snapshot.nodes().to_vec(),
            resources: snapshot.resources().to_vec(),
            next_node_id: snapshot.next_node_id(),
            next_resource_id: snapshot.next_resource_id(),
        }
    }
}

impl From<Snapshot> for NormalizedSnapshot {
    fn from(snapshot: Snapshot) -> Self {
        Self::from(&snapshot)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayStep {
    pub outcome: ApplyOutcome,
    pub snapshot: Snapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    pub steps: Vec<ReplayStep>,
    pub final_snapshot: Snapshot,
}

impl ReplayReport {
    pub fn normalized_final_snapshot(&self) -> NormalizedSnapshot {
        NormalizedSnapshot::from(&self.final_snapshot)
    }

    pub fn snapshot_after(&self, change_index: usize) -> Option<&Snapshot> {
        self.steps.get(change_index).map(|step| &step.snapshot)
    }
}

/// Replays a producer trace and rejects retries, stale changes, and fault-driven
/// recovery transitions. A predecessor-linked epoch start legitimately reports
/// `Recovered` because it atomically replaces the previous epoch.
pub fn replay_protocol_trace(trace: &ProtocolTrace) -> Result<ReplayReport, TraceError> {
    if trace.changes.is_empty() {
        return Err(TraceError::EmptyTrace);
    }

    let mut reducer = Reducer::new();
    let mut steps = Vec::with_capacity(trace.changes.len());
    for (index, change) in trace.changes.iter().cloned().enumerate() {
        let outcome = reducer
            .apply(change)
            .map_err(|error| TraceError::Protocol {
                change_index: index,
                message: error.to_string(),
            })?;
        let canonical = matches!(&outcome, ApplyOutcome::Applied { .. })
            || change_is_epoch_replacement(&trace.changes[index], &outcome);
        if !canonical {
            return Err(TraceError::NonCanonicalOutcome {
                change_index: index,
                outcome: format!("{outcome:?}"),
            });
        }
        let snapshot = reducer
            .document()
            .expect("an applied canonical change installs a document")
            .snapshot();
        steps.push(ReplayStep { outcome, snapshot });
    }

    let final_snapshot = steps
        .last()
        .expect("non-empty traces produce a replay step")
        .snapshot
        .clone();
    Ok(ReplayReport {
        steps,
        final_snapshot,
    })
}

fn change_is_epoch_replacement(change: &ChangeSet, outcome: &ApplyOutcome) -> bool {
    change.epoch_start().is_some() && matches!(outcome, ApplyOutcome::Recovered { .. })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceError {
    EmptyTrace,
    Protocol {
        change_index: usize,
        message: String,
    },
    NonCanonicalOutcome {
        change_index: usize,
        outcome: String,
    },
}

impl fmt::Display for TraceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTrace => formatter.write_str("protocol trace must not be empty"),
            Self::Protocol {
                change_index,
                message,
            } => write!(
                formatter,
                "change {change_index} violates the protocol: {message}"
            ),
            Self::NonCanonicalOutcome {
                change_index,
                outcome,
            } => write!(
                formatter,
                "change {change_index} produced non-canonical outcome {outcome}"
            ),
        }
    }
}

impl std::error::Error for TraceError {}
