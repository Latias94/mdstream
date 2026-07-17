use std::path::PathBuf;

use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::{NormalizedSnapshot, load_fixture};
use mdstream_merman::{DEFAULT_CONFIGURATION_VERSION, MERMAID_ARTIFACT_PROTOCOL, MermaidProcessor};
use mdstream_processors::{
    ArtifactHost, CompletionOutcome, ConfigurationVersion, ContentProcessor, ProcessingPolicy,
    ProcessorLimits, ProcessorResult, run_catching,
};
use mdstream_protocol::{ApplyOutcome, ContentKind, Reducer};

#[test]
fn real_merman_adopts_the_shared_headless_fixture() {
    let fixture = load_fixture(fixture_path()).unwrap();
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    for chunk in fixture
        .schedule("adversarial")
        .unwrap()
        .slices(&fixture.source)
        .unwrap()
    {
        apply(&mut reducer, &mut host, engine.append(chunk).unwrap());
    }
    apply(&mut reducer, &mut host, engine.finish().unwrap());

    let document = reducer.document().unwrap();
    assert_eq!(
        NormalizedSnapshot::from(document.snapshot()),
        fixture.expected.normalized_snapshot.unwrap()
    );
    let mermaid_id = document
        .nodes()
        .find(|node| {
            matches!(
                &node.content,
                ContentKind::CodeBlock { info: Some(info), .. }
                    if info.eq_ignore_ascii_case("mermaid")
            )
        })
        .unwrap()
        .id;
    let canonical_before = document.snapshot();
    let processor = MermaidProcessor::default();
    let configuration = ConfigurationVersion::new(DEFAULT_CONFIGURATION_VERSION).unwrap();
    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            mermaid_id,
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = request.key().slot().clone();
    assert_eq!(
        host.complete(document, run_catching(&processor, &request))
            .unwrap(),
        CompletionOutcome::Applied
    );
    let artifact = host.artifact(&slot).unwrap();
    assert_eq!(artifact.protocol(), MERMAID_ARTIFACT_PROTOCOL);
    assert!(artifact.as_text().unwrap().starts_with("<svg"));
    let late_artifact = artifact.clone();
    assert_eq!(reducer.document().unwrap().snapshot(), canonical_before);

    let late = host
        .begin(
            reducer.document().unwrap(),
            processor.descriptor().clone(),
            mermaid_id,
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let late_result = ProcessorResult::success(late.key().clone(), late_artifact);
    apply(&mut reducer, &mut host, engine.reset().unwrap());
    assert_eq!(
        host.complete(reducer.document().unwrap(), late_result)
            .unwrap(),
        CompletionOutcome::Stale
    );
    assert!(host.artifact(&slot).is_none());
}

fn apply(reducer: &mut Reducer, host: &mut ArtifactHost, output: EngineOutput) {
    for change in output.into_changes() {
        let outcome = reducer.apply(change).unwrap();
        let impact = match outcome {
            ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => impact,
            other => panic!("producer emitted a non-continuous change: {other:?}"),
        };
        host.reconcile(reducer.document().unwrap(), &impact)
            .unwrap();
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("conformance/fixtures/adoption/headless-rich-content.json")
}
