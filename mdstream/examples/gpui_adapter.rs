use mdstream::{EngineOutput, StreamEngine};
use mdstream_processors::{
    ArtifactHost, CompletionOutcome, ConfigurationVersion, ContentProcessor, ProcessingPolicy,
    ProcessorArtifact, ProcessorCapabilities, ProcessorDescriptor, ProcessorFailure,
    ProcessorLimits, run_catching,
};
use mdstream_protocol::{ApplyOutcome, ContentKind, Reducer};

struct EchoProcessor {
    descriptor: ProcessorDescriptor,
}

impl EchoProcessor {
    fn new(id: &str) -> Self {
        Self {
            descriptor: ProcessorDescriptor::new(id, "v1", ProcessorCapabilities::stable_only())
                .unwrap(),
        }
    }
}

impl ContentProcessor for EchoProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        &self.descriptor
    }

    fn process(
        &self,
        request: &mdstream_processors::ProcessorRequest,
    ) -> Result<ProcessorArtifact, ProcessorFailure> {
        ProcessorArtifact::text("example.echo/1", "text/plain", request.input().body()).map_err(
            |error| {
                ProcessorFailure::new(
                    mdstream_processors::ProcessorFailureCode::Processor,
                    error.to_string(),
                )
            },
        )
    }
}

fn apply(
    reducer: &mut Reducer,
    host: &mut ArtifactHost,
    output: EngineOutput,
) -> Result<(), Box<dyn std::error::Error>> {
    for change in output.into_changes() {
        let outcome = reducer.apply(change)?;
        let impact = match outcome {
            ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => impact,
            other => return Err(format!("engine output was not continuous: {other:?}").into()),
        };
        host.reconcile(reducer.document().unwrap(), &impact)?;
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default())?;
    apply(
        &mut reducer,
        &mut host,
        engine.append("```text\nrender me\n```\n")?,
    )?;
    apply(&mut reducer, &mut host, engine.finish()?)?;

    let node_id = reducer
        .document()
        .unwrap()
        .nodes()
        .find(|node| matches!(node.content, ContentKind::CodeBlock { .. }))
        .unwrap()
        .id;
    let configuration = ConfigurationVersion::new("example.default")?;

    let ready_processor = EchoProcessor::new("example.ready");
    let ready = host.begin(
        reducer.document().unwrap(),
        ready_processor.descriptor().clone(),
        node_id,
        configuration.clone(),
        ProcessingPolicy::StableOnly,
    )?;
    let ready_slot = ready.key().slot().clone();
    assert_eq!(
        host.complete(
            reducer.document().unwrap(),
            run_catching(&ready_processor, &ready),
        )?,
        CompletionOutcome::Applied
    );
    assert!(host.artifact(&ready_slot).is_some());

    let pending_processor = EchoProcessor::new("example.pending");
    let pending = host.begin(
        reducer.document().unwrap(),
        pending_processor.descriptor().clone(),
        node_id,
        configuration,
        ProcessingPolicy::StableOnly,
    )?;
    let late_result = run_catching(&pending_processor, &pending);

    apply(&mut reducer, &mut host, engine.reset()?)?;
    assert!(host.artifact(&ready_slot).is_none());
    assert_eq!(
        host.complete(reducer.document().unwrap(), late_result)?,
        CompletionOutcome::Stale
    );
    Ok(())
}
