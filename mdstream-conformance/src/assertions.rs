use std::{collections::BTreeMap, fmt};

use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, ProjectionOp, ProtocolError, RecoveryReason, Reducer,
    ReducerStatus, SourceDelta,
};

use crate::{
    Fixture, FixtureError, NormalizedSnapshot, ProtocolTrace, ReplayReport, RequiredCheckpoint,
    TraceError, replay_protocol_trace,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureReport {
    pub traces: Vec<(String, ReplayReport)>,
}

/// Validates all checked-in protocol traces, their checkpoints, and their
/// schedule-independent normalized result.
pub fn assert_fixture_protocol(fixture: &Fixture) -> Result<FixtureReport, ConformanceError> {
    fixture
        .validate()
        .map_err(ConformanceError::InvalidFixture)?;

    let mut reports = Vec::with_capacity(fixture.traces.len());
    let mut normalized_baseline: Option<NormalizedSnapshot> = None;
    for trace in &fixture.traces {
        let report = replay_protocol_trace(trace).map_err(|source| ConformanceError::Trace {
            trace: trace.id.clone(),
            source,
        })?;
        let normalized = report.normalized_final_snapshot();
        if normalized.source != fixture.source {
            return Err(ConformanceError::SourceMismatch {
                trace: trace.id.clone(),
                expected: fixture.source.clone(),
                actual: normalized.source,
            });
        }
        if let Some(expected) = &fixture.expected.normalized_snapshot {
            if &normalized != expected {
                return Err(ConformanceError::SnapshotMismatch {
                    trace: trace.id.clone(),
                });
            }
        }
        if let Some(baseline) = &normalized_baseline {
            if &normalized != baseline {
                return Err(ConformanceError::ScheduleDivergence {
                    trace: trace.id.clone(),
                });
            }
        } else {
            normalized_baseline = Some(normalized);
        }
        assert_trace_laws(trace)?;
        reports.push((trace.id.clone(), report));
    }

    let by_id = reports
        .iter()
        .map(|(id, report)| (id.as_str(), report))
        .collect::<BTreeMap<_, _>>();
    for checkpoint in &fixture.required_checkpoints {
        let report = by_id
            .get(checkpoint.trace.as_str())
            .expect("Fixture::validate checks checkpoint trace references");
        assert_checkpoint(checkpoint, report)?;
    }

    Ok(FixtureReport { traces: reports })
}

/// Runs every applicable ordered-delivery fault law against one canonical
/// trace. This is intentionally separate from cross-schedule normalization:
/// recovery within a trace must reproduce its complete wire snapshot.
pub fn assert_trace_laws(trace: &ProtocolTrace) -> Result<(), ConformanceError> {
    assert_last_retry_idempotent(trace)?;
    if trace.changes.len() >= 2 {
        assert_older_change_stale(trace)?;
    }
    for accepted_index in 0..trace.changes.len() {
        assert_fork_snapshot_recovery(trace, accepted_index)?;
    }
    for missing_index in 1..trace.changes.len().saturating_sub(1) {
        match assert_gap_snapshot_recovery(trace, missing_index) {
            Ok(()) | Err(ConformanceError::LawNotApplicable(_)) => {}
            Err(error) => return Err(error),
        }
    }
    if trace
        .changes
        .iter()
        .skip(1)
        .any(|change| change.epoch_start().is_some())
    {
        assert_epoch_reset_isolation(trace)?;
    }
    Ok(())
}

pub fn assert_checkpoint(
    checkpoint: &RequiredCheckpoint,
    report: &ReplayReport,
) -> Result<(), ConformanceError> {
    let snapshot = report
        .snapshot_after(checkpoint.after_change)
        .ok_or_else(|| ConformanceError::CheckpointMismatch {
            checkpoint: checkpoint.id.clone(),
            field: "after_change",
        })?;

    if checkpoint
        .coordinate
        .as_ref()
        .is_some_and(|expected| snapshot.coordinate() != expected)
    {
        return Err(ConformanceError::CheckpointMismatch {
            checkpoint: checkpoint.id.clone(),
            field: "coordinate",
        });
    }
    if checkpoint
        .lifecycle
        .is_some_and(|expected| snapshot.lifecycle() != expected)
    {
        return Err(ConformanceError::CheckpointMismatch {
            checkpoint: checkpoint.id.clone(),
            field: "lifecycle",
        });
    }
    if checkpoint
        .source
        .as_ref()
        .is_some_and(|expected| snapshot.source() != expected)
    {
        return Err(ConformanceError::CheckpointMismatch {
            checkpoint: checkpoint.id.clone(),
            field: "source",
        });
    }
    if checkpoint
        .normalized_snapshot
        .as_ref()
        .is_some_and(|expected| &NormalizedSnapshot::from(snapshot) != expected)
    {
        return Err(ConformanceError::CheckpointMismatch {
            checkpoint: checkpoint.id.clone(),
            field: "normalized_snapshot",
        });
    }
    Ok(())
}

/// The last accepted change must be an idempotent retry and leave state intact.
pub fn assert_last_retry_idempotent(trace: &ProtocolTrace) -> Result<(), ConformanceError> {
    let last = trace
        .changes
        .last()
        .ok_or(ConformanceError::LawNotApplicable("empty trace"))?
        .clone();
    let mut reducer = replay_into_reducer(trace)?;
    let before = reducer
        .document()
        .expect("a replayed non-empty trace has a document")
        .snapshot();
    let outcome = reducer
        .apply(last)
        .map_err(|error| law_protocol("retry", error))?;
    if outcome != ApplyOutcome::Idempotent {
        return Err(ConformanceError::LawViolation {
            law: "retry",
            message: format!("expected Idempotent, received {outcome:?}"),
        });
    }
    let after = reducer
        .document()
        .expect("retry retains the document")
        .snapshot();
    if after != before {
        return Err(ConformanceError::LawViolation {
            law: "retry",
            message: "idempotent retry mutated the snapshot".to_string(),
        });
    }
    Ok(())
}

/// Replaying any accepted change older than the current coordinate is stale
/// and must not mutate the reducer.
pub fn assert_older_change_stale(trace: &ProtocolTrace) -> Result<(), ConformanceError> {
    if trace.changes.len() < 2 {
        return Err(ConformanceError::LawNotApplicable(
            "stale replay requires at least two changes",
        ));
    }
    let stale = trace.changes[0].clone();
    let mut reducer = replay_into_reducer(trace)?;
    let before = reducer
        .document()
        .expect("a replayed trace has a document")
        .snapshot();
    let outcome = reducer
        .apply(stale)
        .map_err(|error| law_protocol("stale", error))?;
    if !matches!(outcome, ApplyOutcome::Stale { .. }) {
        return Err(ConformanceError::LawViolation {
            law: "stale",
            message: format!("expected Stale, received {outcome:?}"),
        });
    }
    if reducer
        .document()
        .expect("stale replay retains the document")
        .snapshot()
        != before
    {
        return Err(ConformanceError::LawViolation {
            law: "stale",
            message: "stale replay mutated the snapshot".to_string(),
        });
    }
    Ok(())
}

/// Deletes one ordinary change, proves gap recovery, installs the producer's
/// checkpoint, and resumes with the next continuous change.
pub fn assert_gap_snapshot_recovery(
    trace: &ProtocolTrace,
    missing_index: usize,
) -> Result<(), ConformanceError> {
    if missing_index == 0 || missing_index + 1 >= trace.changes.len() {
        return Err(ConformanceError::LawNotApplicable(
            "gap recovery needs a middle ordinary change",
        ));
    }
    let previous = &trace.changes[missing_index - 1];
    let missing = &trace.changes[missing_index];
    let following = &trace.changes[missing_index + 1];
    if missing.epoch_start().is_some()
        || following.epoch_start().is_some()
        || previous.epoch() != missing.epoch()
        || missing.epoch() != following.epoch()
    {
        return Err(ConformanceError::LawNotApplicable(
            "gap recovery requires three changes in one epoch",
        ));
    }

    let canonical = replay_protocol_trace(trace).map_err(|source| ConformanceError::Trace {
        trace: trace.id.clone(),
        source,
    })?;
    let mut consumer = Reducer::new();
    apply_prefix(&mut consumer, &trace.changes[..missing_index], "gap")?;
    let before = consumer
        .document()
        .expect("prefix installs a document")
        .snapshot();
    let outcome = consumer
        .apply(following.clone())
        .map_err(|error| law_protocol("gap", error))?;
    if !matches!(
        outcome,
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceGap { .. },
            ..
        }
    ) {
        return Err(ConformanceError::LawViolation {
            law: "gap",
            message: format!("expected SequenceGap recovery, received {outcome:?}"),
        });
    }
    if consumer
        .document()
        .expect("gap retains the last-good document")
        .snapshot()
        != before
    {
        return Err(ConformanceError::LawViolation {
            law: "gap",
            message: "gap mutated the retained document".to_string(),
        });
    }
    if consumer.apply(following.clone()) != Err(ProtocolError::NeedsSnapshot) {
        return Err(ConformanceError::LawViolation {
            law: "gap",
            message: "NeedsSnapshot accepted an ordinary delta".to_string(),
        });
    }

    let recovery = canonical
        .snapshot_after(missing_index)
        .expect("canonical report contains missing checkpoint")
        .clone();
    let recovered = consumer
        .recover_snapshot(recovery)
        .map_err(|error| law_protocol("gap recovery", error))?;
    if !matches!(recovered, ApplyOutcome::Recovered { .. }) {
        return Err(ConformanceError::LawViolation {
            law: "gap recovery",
            message: format!("expected Recovered, received {recovered:?}"),
        });
    }
    apply_prefix(
        &mut consumer,
        &trace.changes[missing_index + 1..],
        "gap recovery continuation",
    )?;
    assert_final_matches("gap recovery", &consumer, &canonical)
}

/// Changes the ID at an already accepted sequence, proves fork recovery, and
/// resumes from the same-floor canonical snapshot.
pub fn assert_fork_snapshot_recovery(
    trace: &ProtocolTrace,
    accepted_index: usize,
) -> Result<(), ConformanceError> {
    if accepted_index >= trace.changes.len() {
        return Err(ConformanceError::LawNotApplicable(
            "fork index is outside the trace",
        ));
    }
    let canonical = replay_protocol_trace(trace).map_err(|source| ConformanceError::Trace {
        trace: trace.id.clone(),
        source,
    })?;
    let mut consumer = Reducer::new();
    apply_prefix(
        &mut consumer,
        &trace.changes[..=accepted_index],
        "fork prefix",
    )?;
    let before = consumer
        .document()
        .expect("fork prefix installs a document")
        .snapshot();
    let conflicting = conflicting_change_id(&trace.changes[accepted_index], accepted_index)?;
    let outcome = consumer
        .apply(conflicting)
        .map_err(|error| law_protocol("fork", error))?;
    if !matches!(
        outcome,
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::SequenceFork { .. },
            ..
        }
    ) {
        return Err(ConformanceError::LawViolation {
            law: "fork",
            message: format!("expected SequenceFork recovery, received {outcome:?}"),
        });
    }
    if consumer
        .document()
        .expect("fork retains the last-good document")
        .snapshot()
        != before
    {
        return Err(ConformanceError::LawViolation {
            law: "fork",
            message: "fork mutated the retained document".to_string(),
        });
    }
    let coordinate = consumer
        .document()
        .expect("fork retains the last-good document")
        .coordinate()
        .clone();
    let probe_sequence =
        coordinate
            .sequence
            .checked_add(1)
            .ok_or(ConformanceError::LawNotApplicable(
                "fork recovery probe sequence overflowed",
            ))?;
    let ordinary_probe = ChangeSet::new(
        coordinate.epoch,
        probe_sequence,
        ChangeId::new("conformance:fork:sticky").expect("static conformance change ID is valid"),
        SourceDelta::unchanged(coordinate.source_cursor),
        vec![ProjectionOp::FinishDocument],
    )
    .map_err(|error| law_protocol("fork", error))?;
    if consumer.apply(ordinary_probe) != Err(ProtocolError::NeedsSnapshot) {
        return Err(ConformanceError::LawViolation {
            law: "fork",
            message: "NeedsSnapshot accepted an ordinary delta after a fork".to_string(),
        });
    }

    let recovery = canonical
        .snapshot_after(accepted_index)
        .expect("canonical report contains fork checkpoint")
        .clone();
    consumer
        .recover_snapshot(recovery)
        .map_err(|error| law_protocol("fork recovery", error))?;
    if consumer.status() != ReducerStatus::Ready {
        return Err(ConformanceError::LawViolation {
            law: "fork recovery",
            message: "same-floor snapshot did not restore Ready".to_string(),
        });
    }
    apply_prefix(
        &mut consumer,
        &trace.changes[accepted_index + 1..],
        "fork recovery continuation",
    )?;
    assert_final_matches("fork recovery", &consumer, &canonical)
}

/// After an epoch reset, a delayed change from the prior epoch is stale and
/// cannot mutate the replacement document.
pub fn assert_epoch_reset_isolation(trace: &ProtocolTrace) -> Result<(), ConformanceError> {
    let reset_index = trace
        .changes
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, change)| change.epoch_start().is_some().then_some(index))
        .ok_or(ConformanceError::LawNotApplicable(
            "trace does not contain an epoch reset",
        ))?;
    let delayed = trace.changes[reset_index - 1].clone();
    let mut reducer = replay_into_reducer(trace)?;
    let before = reducer
        .document()
        .expect("reset trace installs a document")
        .snapshot();
    let outcome = reducer
        .apply(delayed)
        .map_err(|error| law_protocol("epoch reset", error))?;
    if !matches!(outcome, ApplyOutcome::Stale { .. }) {
        return Err(ConformanceError::LawViolation {
            law: "epoch reset",
            message: format!("expected prior epoch to be Stale, received {outcome:?}"),
        });
    }
    if reducer
        .document()
        .expect("stale prior epoch retains replacement")
        .snapshot()
        != before
    {
        return Err(ConformanceError::LawViolation {
            law: "epoch reset",
            message: "delayed prior-epoch change mutated the replacement".to_string(),
        });
    }
    Ok(())
}

fn replay_into_reducer(trace: &ProtocolTrace) -> Result<Reducer, ConformanceError> {
    if trace.changes.is_empty() {
        return Err(ConformanceError::LawNotApplicable("empty trace"));
    }
    let mut reducer = Reducer::new();
    apply_prefix(&mut reducer, &trace.changes, "canonical replay")?;
    Ok(reducer)
}

fn apply_prefix(
    reducer: &mut Reducer,
    changes: &[ChangeSet],
    law: &'static str,
) -> Result<(), ConformanceError> {
    for change in changes {
        let outcome = reducer
            .apply(change.clone())
            .map_err(|error| law_protocol(law, error))?;
        let canonical = matches!(&outcome, ApplyOutcome::Applied { .. })
            || (change.epoch_start().is_some()
                && matches!(&outcome, ApplyOutcome::Recovered { .. }));
        if !canonical {
            return Err(ConformanceError::LawViolation {
                law,
                message: format!("canonical change produced {outcome:?}"),
            });
        }
    }
    Ok(())
}

fn conflicting_change_id(
    accepted: &ChangeSet,
    index: usize,
) -> Result<ChangeSet, ConformanceError> {
    let mut value =
        serde_json::to_value(accepted).map_err(|error| ConformanceError::LawViolation {
            law: "fork",
            message: format!("failed to encode accepted change: {error}"),
        })?;
    let mut conflicting_id = format!("conformance:fork:{index}");
    if accepted.change_id().as_str() == conflicting_id {
        conflicting_id.push_str(":alternate");
    }
    value["change_id"] = serde_json::Value::String(conflicting_id);
    serde_json::from_value(value).map_err(|error| ConformanceError::LawViolation {
        law: "fork",
        message: format!("failed to construct conflicting change: {error}"),
    })
}

fn assert_final_matches(
    law: &'static str,
    reducer: &Reducer,
    canonical: &ReplayReport,
) -> Result<(), ConformanceError> {
    let actual = reducer
        .document()
        .expect("completed law replay has a document")
        .snapshot();
    if actual != canonical.final_snapshot {
        return Err(ConformanceError::LawViolation {
            law,
            message: "recovered replay differs from the complete canonical snapshot".to_string(),
        });
    }
    Ok(())
}

fn law_protocol(law: &'static str, error: ProtocolError) -> ConformanceError {
    ConformanceError::LawViolation {
        law,
        message: error.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConformanceError {
    InvalidFixture(FixtureError),
    Trace {
        trace: String,
        source: TraceError,
    },
    SourceMismatch {
        trace: String,
        expected: String,
        actual: String,
    },
    SnapshotMismatch {
        trace: String,
    },
    ScheduleDivergence {
        trace: String,
    },
    CheckpointMismatch {
        checkpoint: String,
        field: &'static str,
    },
    LawNotApplicable(&'static str),
    LawViolation {
        law: &'static str,
        message: String,
    },
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFixture(error) => write!(formatter, "invalid fixture: {error}"),
            Self::Trace { trace, source } => write!(formatter, "trace `{trace}` failed: {source}"),
            Self::SourceMismatch {
                trace,
                expected,
                actual,
            } => write!(
                formatter,
                "trace `{trace}` source mismatch: expected {expected:?}, received {actual:?}"
            ),
            Self::SnapshotMismatch { trace } => {
                write!(
                    formatter,
                    "trace `{trace}` differs from the expected snapshot"
                )
            }
            Self::ScheduleDivergence { trace } => write!(
                formatter,
                "trace `{trace}` differs from the normalized schedule baseline"
            ),
            Self::CheckpointMismatch { checkpoint, field } => {
                write!(formatter, "checkpoint `{checkpoint}` mismatched `{field}`")
            }
            Self::LawNotApplicable(message) => {
                write!(formatter, "law is not applicable: {message}")
            }
            Self::LawViolation { law, message } => {
                write!(formatter, "{law} law failed: {message}")
            }
        }
    }
}

impl std::error::Error for ConformanceError {}
