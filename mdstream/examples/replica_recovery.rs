use std::{collections::BTreeMap, io};

use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{
    ApplyOutcome, ChangeSet, DocumentLifecycle, Snapshot, TransitionFacts, TransitionNodeKey,
    TransitionReducer,
};
use serde_json::Value;

const SCENARIO: &str = include_str!("fixtures/golden-ai-stream.json");

struct GoldenTrace {
    value: Value,
    changes: Vec<ChangeSet>,
    snapshots: BTreeMap<String, Snapshot>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let trace = capture_golden()?;
    let recovery = trace.value["episodes"]["recovery"]["actions"]
        .as_array()
        .ok_or_else(|| invalid_data("Golden scenario has no recovery actions"))?;

    let first = ordinal(&recovery[0])?;
    let skipped = ordinal(&recovery[1])?;
    assert_action(
        &recovery[2],
        "recover_snapshot",
        "same-floor-replica",
        "retained_same_floor",
    )?;
    let same_floor_snapshot = named_snapshot(&trace, &recovery[2])?;
    let mut same_floor = TransitionReducer::new();
    assert!(matches!(
        same_floor.apply(trace.changes[first].clone())?.outcome,
        ApplyOutcome::Applied { .. }
    ));
    let retained_key = first_root_key(&same_floor);
    assert!(matches!(
        same_floor.apply(trace.changes[skipped].clone())?.outcome,
        ApplyOutcome::RecoveryRequired { .. }
    ));
    let recovered = same_floor.recover_snapshot(same_floor_snapshot)?;
    match recovered.outcome {
        ApplyOutcome::Recovered { impact, .. } => assert!(impact.is_empty()),
        other => return Err(format!("same-floor recovery returned {other:?}").into()),
    }
    assert!(recovered.facts.is_none());
    assert_eq!(first_root_key(&same_floor), retained_key);
    finish_replica(&mut same_floor, &trace.changes, first + 1)?;
    println!(
        "recovery=retained_same_floor continuity_generation={} host_action=retain-qualified-keys",
        same_floor.continuity_generation()
    );

    let first = ordinal(&recovery[3])?;
    let skipped = ordinal(&recovery[4])?;
    assert_action(
        &recovery[5],
        "recover_snapshot",
        "advanced-replica",
        "replaced_advanced",
    )?;
    let advanced_snapshot = named_snapshot(&trace, &recovery[5])?;
    let replacement_sequence = advanced_snapshot.coordinate().sequence;
    let mut advanced = TransitionReducer::new();
    advanced.apply(trace.changes[first].clone())?;
    let discarded_key = first_root_key(&advanced);
    assert!(matches!(
        advanced.apply(trace.changes[skipped].clone())?.outcome,
        ApplyOutcome::RecoveryRequired { .. }
    ));
    let recovered = advanced.recover_snapshot(advanced_snapshot)?;
    match recovered.outcome {
        ApplyOutcome::Recovered { impact, .. } => assert!(impact.full_replace),
        other => return Err(format!("advanced recovery returned {other:?}").into()),
    }
    assert!(matches!(
        recovered.facts,
        Some(TransitionFacts::FullReplace { .. })
    ));
    let replacement_key = first_root_key(&advanced);
    assert_eq!(discarded_key.node_id, replacement_key.node_id);
    assert_ne!(
        discarded_key.continuity_generation,
        replacement_key.continuity_generation
    );
    finish_replica_after(&mut advanced, &trace.changes, replacement_sequence)?;
    println!(
        "recovery=replaced_advanced continuity_generation={} host_action=clear-prior-continuity-state",
        advanced.continuity_generation()
    );

    assert_action(&recovery[6], "reset", "producer", "new_epoch")?;
    let expected_epoch = recovery[6]["expect_epoch"]
        .as_u64()
        .ok_or_else(|| invalid_data("reset action has no expected epoch"))?;
    let mut engine = finished_golden_engine(&trace.value)?;
    let reset = engine.reset()?;
    let mut reset_replica = TransitionReducer::new();
    reset_replica.recover_snapshot(
        trace
            .snapshots
            .get("finalized")
            .ok_or_else(|| invalid_data("missing finalized snapshot"))?
            .clone(),
    )?;
    for change in reset.into_changes() {
        let outcome = reset_replica.apply(change)?;
        match outcome.outcome {
            ApplyOutcome::Recovered { coordinate, impact } => {
                assert_eq!(coordinate.epoch.get(), expected_epoch);
                assert!(impact.full_replace);
            }
            other => return Err(format!("reset returned {other:?}").into()),
        }
    }
    println!("recovery=new_epoch epoch={expected_epoch} host_action=clear-prior-epoch-state");
    Ok(())
}

fn capture_golden() -> Result<GoldenTrace, Box<dyn std::error::Error>> {
    let value: Value = serde_json::from_str(SCENARIO)?;
    let actions = value["episodes"]["mainline"]["actions"]
        .as_array()
        .ok_or_else(|| invalid_data("Golden scenario has no mainline actions"))?;
    let mut engine = StreamEngine::new();
    let mut producer = TransitionReducer::new();
    let mut changes = Vec::new();
    let mut snapshots = BTreeMap::new();

    for action in actions {
        match required_str(action, "kind")? {
            "append" => collect(
                &mut producer,
                &mut changes,
                engine.append(required_str(action, "chunk")?)?,
            )?,
            "checkpoint" => {
                let id = required_str(action, "id")?;
                let expected_cursor = action["source_cursor"]
                    .as_u64()
                    .ok_or_else(|| invalid_data("checkpoint has no source cursor"))?
                    as usize;
                let document = producer.document().unwrap();
                assert_eq!(
                    document.source().len(),
                    expected_cursor,
                    "checkpoint `{id}`"
                );
                snapshots.insert(id.to_string(), document.snapshot());
            }
            "finish" => {
                collect(&mut producer, &mut changes, engine.finish()?)?;
                snapshots.insert(
                    required_str(action, "id")?.to_string(),
                    producer.document().unwrap().snapshot(),
                );
            }
            kind => {
                return Err(invalid_data(format!("unsupported scenario action `{kind}`")).into());
            }
        }
    }
    Ok(GoldenTrace {
        value,
        changes,
        snapshots,
    })
}

fn collect(
    producer: &mut TransitionReducer,
    changes: &mut Vec<ChangeSet>,
    output: EngineOutput,
) -> Result<(), Box<dyn std::error::Error>> {
    for change in output.into_changes() {
        match producer.apply(change.clone())?.outcome {
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. } => changes.push(change),
            other => return Err(format!("producer output was not continuous: {other:?}").into()),
        }
    }
    Ok(())
}

fn first_root_key(reducer: &TransitionReducer) -> TransitionNodeKey {
    let document = reducer.document().unwrap();
    TransitionNodeKey {
        continuity_generation: reducer.continuity_generation(),
        epoch: document.coordinate().epoch,
        node_id: document.roots().as_slice()[0],
    }
}

fn finish_replica(
    replica: &mut TransitionReducer,
    changes: &[ChangeSet],
    start: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    for change in &changes[start..] {
        assert!(matches!(
            replica.apply(change.clone())?.outcome,
            ApplyOutcome::Applied { .. }
        ));
    }
    assert_eq!(
        replica.document().unwrap().lifecycle(),
        DocumentLifecycle::Finalized
    );
    Ok(())
}

fn finish_replica_after(
    replica: &mut TransitionReducer,
    changes: &[ChangeSet],
    sequence: mdstream_protocol::Sequence,
) -> Result<(), Box<dyn std::error::Error>> {
    for change in changes.iter().filter(|change| change.sequence() > sequence) {
        assert!(matches!(
            replica.apply(change.clone())?.outcome,
            ApplyOutcome::Applied { .. }
        ));
    }
    assert_eq!(
        replica.document().unwrap().lifecycle(),
        DocumentLifecycle::Finalized
    );
    Ok(())
}

fn named_snapshot(
    trace: &GoldenTrace,
    action: &Value,
) -> Result<Snapshot, Box<dyn std::error::Error>> {
    let name = required_str(action, "snapshot")?;
    trace
        .snapshots
        .get(name)
        .cloned()
        .ok_or_else(|| invalid_data(format!("unknown snapshot `{name}`")).into())
}

fn ordinal(action: &Value) -> Result<usize, io::Error> {
    action["change_ordinal"]
        .as_u64()
        .map(|ordinal| ordinal as usize)
        .ok_or_else(|| invalid_data("recovery action has no change ordinal"))
}

fn assert_action(
    action: &Value,
    kind: &str,
    target: &str,
    continuity: &str,
) -> Result<(), io::Error> {
    if required_str(action, "kind")? != kind
        || required_str(action, "target")? != target
        || required_str(action, "continuity")? != continuity
    {
        return Err(invalid_data(format!(
            "expected {kind}/{target}/{continuity} recovery action"
        )));
    }
    Ok(())
}

fn finished_golden_engine(value: &Value) -> Result<StreamEngine, Box<dyn std::error::Error>> {
    let mut engine = StreamEngine::new();
    for action in value["episodes"]["mainline"]["actions"]
        .as_array()
        .ok_or_else(|| invalid_data("Golden scenario has no mainline actions"))?
    {
        match required_str(action, "kind")? {
            "append" => {
                engine.append(required_str(action, "chunk")?)?;
            }
            "checkpoint" => {}
            "finish" => {
                engine.finish()?;
            }
            kind => {
                return Err(invalid_data(format!("unsupported scenario action `{kind}`")).into());
            }
        }
    }
    Ok(engine)
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, io::Error> {
    value[key]
        .as_str()
        .ok_or_else(|| invalid_data(format!("missing string field `{key}`")))
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
