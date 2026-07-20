use mdstream::{CustomBlockSpec, EngineOutput, StreamEngine};
use mdstream_processors::{
    ArtifactHost, CompletionOutcome, ConfigurationVersion, ContentProcessor, ProcessingPolicy,
    ProcessorArtifact, ProcessorCapabilities, ProcessorDescriptor, ProcessorFailure,
    ProcessorFailureCode, ProcessorLimits, ProcessorRequest, run_catching,
};
use mdstream_protocol::{ApplyOutcome, ContentKind, NodeStability, Reducer};

const ARTIFACT_PROTOCOL: &str = "app.thinking.text/1";
const ARTIFACT_MEDIA_TYPE: &str = "text/plain";

struct ThinkingProcessor {
    descriptor: ProcessorDescriptor,
}

impl ThinkingProcessor {
    fn new() -> Self {
        Self {
            descriptor: ProcessorDescriptor::new(
                "app.thinking",
                "v1",
                ProcessorCapabilities::stable_only(),
            )
            .unwrap(),
        }
    }
}

impl ContentProcessor for ThinkingProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        &self.descriptor
    }

    fn process(&self, request: &ProcessorRequest) -> Result<ProcessorArtifact, ProcessorFailure> {
        match &request.input().node().content {
            ContentKind::Custom {
                namespace, name, ..
            } if namespace == "app.thinking/1" && name == "thinking" => ProcessorArtifact::text(
                ARTIFACT_PROTOCOL,
                ARTIFACT_MEDIA_TYPE,
                request.input().body().trim(),
            )
            .map_err(|error| {
                ProcessorFailure::new(ProcessorFailureCode::Processor, error.to_string())
            }),
            _ => Err(ProcessorFailure::new(
                ProcessorFailureCode::UnsupportedContent,
                "thinking processor requires typed app.thinking/1 content",
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum HostDisplay<'a> {
    PlainText(&'a str),
}

fn dispatch_artifact(artifact: &ProcessorArtifact) -> Option<HostDisplay<'_>> {
    (artifact.protocol() == ARTIFACT_PROTOCOL && artifact.media_type() == ARTIFACT_MEDIA_TYPE)
        .then(|| artifact.as_text().map(HostDisplay::PlainText))
        .flatten()
}

fn apply(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
    }
}

fn main() {
    let mut engine = StreamEngine::builder()
        .custom_block(CustomBlockSpec::try_new("app.thinking/1", "thinking").unwrap())
        .build()
        .unwrap();
    let mut reducer = Reducer::new();

    apply(
        &mut reducer,
        engine
            .append("<thinking role=analysis>\nprivate reasoning\n")
            .unwrap(),
    );
    let provisional = reducer
        .document()
        .unwrap()
        .nodes()
        .find(|node| matches!(node.content, ContentKind::Custom { .. }))
        .unwrap();
    let node_id = provisional.id;
    assert_eq!(provisional.stability, NodeStability::Provisional);

    apply(&mut reducer, engine.append("</thinking>\n").unwrap());
    let document = reducer.document().unwrap();
    let stable = document.node(node_id).unwrap();
    assert_eq!(stable.stability, NodeStability::Stable);
    assert!(matches!(
        &stable.content,
        ContentKind::Custom {
            namespace,
            name,
            attributes,
            ..
        } if namespace == "app.thinking/1"
            && name == "thinking"
            && attributes.get("role").map(String::as_str) == Some("analysis")
    ));
    let canonical_before = document.snapshot();

    let processor = ThinkingProcessor::new();
    let mut artifacts = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    artifacts.begin_epoch(document.coordinate().epoch).unwrap();
    let request = artifacts
        .begin(
            document,
            processor.descriptor().clone(),
            node_id,
            ConfigurationVersion::new("app.thinking.default.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = request.key().slot().clone();
    assert_eq!(
        artifacts
            .complete(document, run_catching(&processor, &request))
            .unwrap(),
        CompletionOutcome::Applied
    );
    let display = dispatch_artifact(artifacts.artifact(&slot).unwrap()).unwrap();

    assert_eq!(display, HostDisplay::PlainText("private reasoning"));
    assert_eq!(reducer.document().unwrap().snapshot(), canonical_before);
    println!(
        "custom_node={} stability={:?} artifact={display:?} canonical_unchanged=true",
        node_id, stable.stability
    );
}
