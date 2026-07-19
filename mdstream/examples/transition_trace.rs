use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::{
    HostReconstructionTrace, NormalizedSnapshot, ProtocolTrace, TraceInputEvent,
    reconstruct_host_trace,
};
use serde_json::json;

const SOURCE: &str = "# Transition trace\n\nStreaming caf\u{e9} with [a late link][guide].\n\n```mermaid\ngraph TD\n  A --> B\n```\n\n[guide]: https://mdstream.dev \"Guide\"\n";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let whole = compile_schedule("whole", [SOURCE])?;
    let scalar_chunks = utf8_scalar_chunks(SOURCE);
    let scalar = compile_schedule("utf8-scalar", scalar_chunks.iter().copied())?;

    if whole.final_snapshot != scalar.final_snapshot {
        return Err("chunk schedules produced different normalized snapshots".into());
    }

    let output = json!({
        "schema": "mdstream.host-reconstruction/0",
        "source_bytes": SOURCE.len(),
        "final_snapshot": whole.final_snapshot,
        "schedules": [whole.host_trace, scalar.host_trace],
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

struct CompiledSchedule {
    final_snapshot: NormalizedSnapshot,
    host_trace: HostReconstructionTrace,
}

fn compile_schedule<'a>(
    schedule: &str,
    chunks: impl IntoIterator<Item = &'a str>,
) -> Result<CompiledSchedule, Box<dyn std::error::Error>> {
    let mut engine = StreamEngine::new();
    let mut changes = Vec::new();
    let mut input_events = Vec::new();

    for chunk in chunks {
        append_changes(&mut changes, engine.append(chunk)?);
        input_events.push(TraceInputEvent::Append {
            chunk: chunk.to_string(),
            change_end: changes.len(),
        });
    }
    append_changes(&mut changes, engine.finish()?);
    input_events.push(TraceInputEvent::Finish {
        change_end: changes.len(),
    });

    let final_snapshot = NormalizedSnapshot::from(
        engine
            .snapshot()
            .expect("finishing a stream installs a canonical document"),
    );
    let trace = ProtocolTrace {
        id: format!("transition-trace:{schedule}"),
        schedule: schedule.to_string(),
        setup_changes: 0,
        input_events,
        changes,
    };
    let host_trace = reconstruct_host_trace(&trace)?;
    Ok(CompiledSchedule {
        final_snapshot,
        host_trace,
    })
}

fn append_changes(changes: &mut Vec<mdstream_protocol::ChangeSet>, output: EngineOutput) {
    changes.extend(output.into_changes());
}

fn utf8_scalar_chunks(source: &str) -> Vec<&str> {
    let mut boundaries = source.char_indices().map(|(index, _)| index).skip(1);
    let mut start = 0usize;
    let mut chunks = Vec::new();
    for end in boundaries.by_ref() {
        chunks.push(&source[start..end]);
        start = end;
    }
    chunks.push(&source[start..]);
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdstream_conformance::TextReconstruction;

    #[test]
    fn whole_and_utf8_scalar_schedules_converge() {
        let whole = compile_schedule("whole", [SOURCE]).unwrap();
        let scalar_chunks = utf8_scalar_chunks(SOURCE);
        let scalar = compile_schedule("utf8-scalar", scalar_chunks.iter().copied()).unwrap();

        assert_eq!(whole.final_snapshot, scalar.final_snapshot);
        assert_ne!(
            whole.host_trace.total_work, scalar.host_trace.total_work,
            "the baseline should expose schedule-local reconstruction work"
        );
        assert!(scalar.host_trace.steps.iter().any(|step| {
            step.nodes
                .iter()
                .any(|node| node.before.is_none() && node.after.is_some())
        }));
        assert!(scalar.host_trace.steps.iter().any(|step| {
            step.nodes
                .iter()
                .any(|node| matches!(node.text, Some(TextReconstruction::Appended { .. })))
        }));
        assert!(scalar.host_trace.steps.iter().any(|step| {
            step.nodes.iter().any(|node| {
                node.before.as_ref().is_some_and(|before| {
                    before.stability == mdstream_protocol::NodeStability::Provisional
                        && node.after.as_ref().is_some_and(|after| {
                            after.stability == mdstream_protocol::NodeStability::Stable
                        })
                })
            })
        }));
        assert!(
            scalar
                .host_trace
                .steps
                .iter()
                .any(|step| !step.structures.is_empty())
        );
        assert!(scalar.host_trace.steps.iter().any(|step| {
            step.pending_source
                .as_ref()
                .is_some_and(|pending| !pending.text.is_empty())
        }));
        assert!(
            scalar
                .host_trace
                .steps
                .last()
                .unwrap()
                .impact
                .lifecycle_changed
        );
        assert_eq!(
            serde_json::to_string(&whole.host_trace).unwrap(),
            serde_json::to_string(&compile_schedule("whole", [SOURCE]).unwrap().host_trace)
                .unwrap()
        );
    }
}
