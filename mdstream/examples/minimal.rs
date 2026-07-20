use std::{collections::BTreeMap, env, io};

use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{
    ApplyOutcome, ContentKind, DocumentLifecycle, NodeId, NodeStability, NodeVersion, Reducer,
};
use serde_json::Value;

const SCENARIO: &str = include_str!("fixtures/golden-ai-stream.json");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let assert_mode = parse_args()?;
    let value: Value = serde_json::from_str(SCENARIO)?;
    let actions = value["episodes"]["mainline"]["actions"]
        .as_array()
        .ok_or_else(|| invalid_data("Golden scenario has no mainline actions"))?;
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut unresolved_citations = BTreeMap::<String, (NodeId, NodeVersion)>::new();

    println!("owner=mdstream responsibility=canonical-state,identity,lifecycle");
    println!("owner=host responsibility=presentation,timing,layout,accessibility");

    for action in actions {
        match required_str(action, "kind")? {
            "append" => apply(&mut reducer, engine.append(required_str(action, "chunk")?)?)?,
            "checkpoint" => report_checkpoint(
                action,
                reducer
                    .document()
                    .ok_or_else(|| invalid_data("checkpoint observed before the stream started"))?,
                assert_mode,
                &mut unresolved_citations,
            )?,
            "finish" => {
                apply(&mut reducer, engine.finish()?)?;
                report_checkpoint(
                    action,
                    reducer.document().expect("finish installs a document"),
                    assert_mode,
                    &mut unresolved_citations,
                )?;
            }
            kind => {
                return Err(invalid_data(format!("unsupported scenario action `{kind}`")).into());
            }
        }
    }

    let document = reducer
        .document()
        .ok_or_else(|| invalid_data("scenario produced no canonical document"))?;
    if assert_mode {
        assert_eq!(
            document.source(),
            required_str(&value["expected"], "final_source")?
        );
        assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized);
        assert!(
            document
                .nodes()
                .all(|node| node.stability == NodeStability::Stable)
        );
        println!("ASSERTIONS_OK scenario=golden-ai-stream");
    } else {
        println!("COMPLETE scenario=golden-ai-stream");
    }
    Ok(())
}

fn parse_args() -> Result<bool, io::Error> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(false),
        [flag] if flag == "--assert" => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run -p mdstream --example minimal -- [--assert]",
        )),
    }
}

fn apply(reducer: &mut Reducer, output: EngineOutput) -> Result<(), Box<dyn std::error::Error>> {
    for change in output.into_changes() {
        match reducer.apply(change)? {
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. } => {}
            other => return Err(format!("engine output was not continuous: {other:?}").into()),
        }
    }
    Ok(())
}

fn report_checkpoint(
    action: &Value,
    document: &mdstream_protocol::Document,
    assert_mode: bool,
    unresolved_citations: &mut BTreeMap<String, (NodeId, NodeVersion)>,
) -> Result<(), io::Error> {
    let id = required_str(action, "id")?;
    let observations = action["observations"]
        .as_array()
        .ok_or_else(|| invalid_data("checkpoint has no observations"))?;
    let observation_names = observations
        .iter()
        .map(|observation| {
            observation
                .as_str()
                .ok_or_else(|| invalid_data("observation must be a string"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if assert_mode {
        if let Some(expected_cursor) = action["source_cursor"].as_u64() {
            assert_eq!(
                document.source().len(),
                expected_cursor as usize,
                "checkpoint `{id}`"
            );
        }
        for observation in &observation_names {
            assert_observation(document, observation, id, unresolved_citations);
        }
    }

    println!(
        "checkpoint={id} canonical_bytes={} projected_bytes={} pending_bytes={} nodes={} lifecycle={:?} observations={}",
        document.source().len(),
        document.projection_cursor().get(),
        document.pending_source().len(),
        document.nodes().len(),
        document.lifecycle(),
        observation_names.join(","),
    );
    Ok(())
}

fn assert_observation(
    document: &mdstream_protocol::Document,
    observation: &str,
    checkpoint: &str,
    unresolved_citations: &mut BTreeMap<String, (NodeId, NodeVersion)>,
) {
    match observation {
        "pending_source" => assert!(
            !document.pending_source().is_empty(),
            "checkpoint `{checkpoint}` promised pending source"
        ),
        observation if observation.starts_with("provisional_") => assert!(
            document
                .nodes()
                .any(|node| node.stability == NodeStability::Provisional),
            "checkpoint `{checkpoint}` promised `{observation}`"
        ),
        "stable_code_block" => assert!(has_stable_code_block(document, "rust")),
        "stable_mermaid_block" => assert!(has_stable_code_block(document, "mermaid")),
        "unresolved_citation" => {
            let node = document
                .nodes()
                .find(|node| {
                    matches!(
                        &node.content,
                        ContentKind::CitationReference { key, target: None } if key == "engine"
                    )
                })
                .unwrap_or_else(|| {
                    panic!("checkpoint `{checkpoint}` promised an unresolved citation")
                });
            unresolved_citations
                .entry("engine".to_string())
                .or_insert((node.id, node.version.clone()));
        }
        "resolved_citation" | "semantic_correction" => {
            let node = document
                .nodes()
                .find(|node| {
                    matches!(
                        &node.content,
                        ContentKind::CitationReference { key, target: Some(_) } if key == "engine"
                    )
                })
                .unwrap_or_else(|| {
                    panic!("checkpoint `{checkpoint}` promised a resolved citation")
                });
            if observation == "semantic_correction" {
                let (old_id, old_version) = &unresolved_citations["engine"];
                assert_eq!(&node.id, old_id);
                assert_ne!(&node.version, old_version);
            }
        }
        "finalized" => assert_eq!(document.lifecycle(), DocumentLifecycle::Finalized),
        observation => panic!("unsupported scenario observation `{observation}`"),
    }
}

fn has_stable_code_block(document: &mdstream_protocol::Document, language: &str) -> bool {
    document.nodes().any(|node| {
        node.stability == NodeStability::Stable
            && node
                .content
                .code_language()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(language))
    })
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, io::Error> {
    value[key]
        .as_str()
        .ok_or_else(|| invalid_data(format!("missing string field `{key}`")))
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
