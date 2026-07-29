use std::{env, io};

use mdstream::{EngineOutput, StreamEngine};
use mdstream_merman::{
    DEFAULT_CONFIGURATION_VERSION, MERMAID_ARTIFACT_PROTOCOL, MERMAID_MEDIA_TYPE, MermaidProcessor,
};
use mdstream_processors::{
    ArtifactHost, CompletionOutcome, ConfigurationVersion, ContentProcessor, ProcessingPolicy,
    ProcessorLimits, run_catching,
};
use mdstream_protocol::{ApplyOutcome, DocumentLifecycle, NodeStability, TransitionReducer};
use serde_json::Value;

const GOLDEN_SCENARIO: &str = include_str!("fixtures/golden-ai-stream.json");
const HOST_TRUST_HANDOFF: &str = "sanitizeSvgArtifact";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assert_mode = parse_args()?;
    let scenario: Value = serde_json::from_str(GOLDEN_SCENARIO)?;
    let actions = scenario["episodes"]["mainline"]["actions"]
        .as_array()
        .ok_or_else(|| invalid_data("Golden scenario has no mainline actions"))?;

    let mut engine = StreamEngine::new();
    let mut reducer = TransitionReducer::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default())?;
    for action in actions {
        match required_str(action, "kind")? {
            "append" => apply(
                &mut reducer,
                &mut host,
                engine.append(required_str(action, "chunk")?)?,
            )?,
            "checkpoint" => {
                let expected = action["source_cursor"]
                    .as_u64()
                    .ok_or_else(|| invalid_data("checkpoint has no source cursor"))?;
                let actual = reducer
                    .document()
                    .ok_or_else(|| invalid_data("checkpoint precedes the first document"))?
                    .source()
                    .len() as u64;
                if actual != expected {
                    return Err(invalid_data(format!(
                        "checkpoint `{}` expected source cursor {expected}, got {actual}",
                        required_str(action, "id")?
                    ))
                    .into());
                }
            }
            "finish" => apply(&mut reducer, &mut host, engine.finish()?)?,
            kind => {
                return Err(
                    invalid_data(format!("unsupported Golden scenario action `{kind}`")).into(),
                );
            }
        }
    }

    let document = reducer
        .document()
        .ok_or_else(|| invalid_data("Golden scenario produced no document"))?;
    let mut mermaid_nodes = document.nodes().filter(|node| {
        node.stability == NodeStability::Stable && node.content.is_mermaid_code_block()
    });
    let mermaid_id = mermaid_nodes
        .next()
        .ok_or_else(|| invalid_data("Golden scenario produced no stable Mermaid code node"))?
        .id;
    if mermaid_nodes.next().is_some() {
        return Err(invalid_data("Golden scenario produced multiple stable Mermaid nodes").into());
    }

    let canonical_before = document.snapshot();
    let processor = MermaidProcessor::default();
    let request = host.begin(
        document,
        processor.descriptor().clone(),
        mermaid_id,
        ConfigurationVersion::new(DEFAULT_CONFIGURATION_VERSION)?,
        ProcessingPolicy::StableOnly,
    )?;
    let key = request.key().clone();
    let slot = key.slot().clone();
    if host.complete(document, run_catching(&processor, &request))? != CompletionOutcome::Applied {
        return Err(invalid_data("current Merman result was not applied").into());
    }
    let artifact = host
        .artifact(&slot)
        .ok_or_else(|| invalid_data("Merman produced no retained artifact"))?;
    if artifact.protocol() != MERMAID_ARTIFACT_PROTOCOL
        || artifact.media_type() != MERMAID_MEDIA_TYPE
        || artifact.as_text().is_none()
    {
        return Err(invalid_data("Merman returned an unexpected artifact contract").into());
    }

    println!(
        "artifact_key=epoch:{};node:{};processor:{};node_version:{};input_version:{};processor_version:{};configuration_version:{};generation:{} protocol={} media_type={}",
        key.slot().epoch(),
        key.slot().node_id(),
        key.slot().processor_id(),
        key.node_version(),
        key.input_version(),
        key.processor_version(),
        key.configuration_version(),
        key.generation(),
        artifact.protocol(),
        artifact.media_type()
    );
    println!(
        "host_handoff={HOST_TRUST_HANDOFF} status=required artifact_bytes={}",
        artifact.byte_len()
    );

    if assert_mode {
        let expected_source = scenario["expected"]["final_source"]
            .as_str()
            .ok_or_else(|| invalid_data("Golden scenario has no expected final source"))?;
        assert_eq!(document.source(), expected_source);
        assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
        assert!(
            document
                .nodes()
                .all(|node| node.stability == NodeStability::Stable)
        );
        assert_eq!(document.snapshot(), canonical_before);
        assert!(
            artifact
                .as_text()
                .is_some_and(|svg| svg.starts_with("<svg"))
        );
        println!("mdstream-merman golden stream: ok");
    }
    Ok(())
}

fn apply(
    reducer: &mut TransitionReducer,
    host: &mut ArtifactHost,
    output: EngineOutput,
) -> Result<(), Box<dyn std::error::Error>> {
    for change in output.into_changes() {
        let outcome = reducer.apply(change)?.outcome;
        let impact = match outcome {
            ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => impact,
            other => return Err(format!("engine output was not continuous: {other:?}").into()),
        };
        host.reconcile(reducer.document().unwrap(), &impact)?;
    }
    Ok(())
}

fn parse_args() -> Result<bool, io::Error> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(false),
        [flag] if flag == "--assert" => Ok(true),
        _ => Err(invalid_data("usage: render_golden [--assert]")),
    }
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, io::Error> {
    value[field]
        .as_str()
        .ok_or_else(|| invalid_data(format!("Golden scenario field `{field}` is missing")))
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
