use std::collections::BTreeSet;

use mdstream::{EngineOutput, StreamEngine};
use mdstream_processors::{
    ArtifactHost, CitationProcessor, CompletionOutcome, ConfigurationVersion, ContentProcessor,
    ProcessingPolicy, ProcessorLimits, run_catching,
};
use mdstream_protocol::{ApplyOutcome, ContentKind, NodeId, NodeVersion, Reducer};

#[derive(Debug)]
struct NodeUpdate {
    key: NodeId,
    version: Option<NodeVersion>,
}

struct HeadlessState {
    reducer: Reducer,
    artifacts: ArtifactHost,
    invalidated: BTreeSet<NodeId>,
}

impl HeadlessState {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            reducer: Reducer::new(),
            artifacts: ArtifactHost::new(ProcessorLimits::default())?,
            invalidated: BTreeSet::new(),
        })
    }

    fn apply(&mut self, output: EngineOutput) -> Result<(), Box<dyn std::error::Error>> {
        for change in output.into_changes() {
            let outcome = self.reducer.apply(change)?;
            let impact = match outcome {
                ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => {
                    impact
                }
                other => {
                    return Err(format!("producer change was not continuous: {other:?}").into());
                }
            };
            self.artifacts
                .reconcile(self.reducer.document().unwrap(), &impact)?;
            self.invalidated.extend(impact.changed_nodes);
        }
        Ok(())
    }

    fn take_node_updates(&mut self) -> Vec<NodeUpdate> {
        let document = self.reducer.document();
        std::mem::take(&mut self.invalidated)
            .into_iter()
            .map(|key| NodeUpdate {
                key,
                version: document
                    .and_then(|document| document.node(key))
                    .map(|node| node.version.clone()),
            })
            .collect()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = StreamEngine::new();
    let mut state = HeadlessState::new()?;
    for chunk in ["See [@Engine]\n\n", "[@engine]: https://mdstream.dev\n"] {
        state.apply(engine.append(chunk)?)?;
        for update in state.take_node_updates() {
            println!("node {} -> {:?}", update.key, update.version);
        }
    }
    state.apply(engine.finish()?)?;

    let citation_id = state
        .reducer
        .document()
        .unwrap()
        .nodes()
        .find(|node| matches!(node.content, ContentKind::CitationReference { .. }))
        .unwrap()
        .id;
    let processor = CitationProcessor::new();
    let request = state.artifacts.begin(
        state.reducer.document().unwrap(),
        processor.descriptor().clone(),
        citation_id,
        ConfigurationVersion::new("example.citation.v1")?,
        ProcessingPolicy::StableOnly,
    )?;
    assert_eq!(
        state.artifacts.complete(
            state.reducer.document().unwrap(),
            run_catching(&processor, &request),
        )?,
        CompletionOutcome::Applied
    );
    assert!(state.artifacts.artifact(request.key().slot()).is_some());
    Ok(())
}
