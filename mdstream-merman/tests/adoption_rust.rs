mod support;

use std::path::PathBuf;

use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::{NormalizedSnapshot, load_fixture};
use mdstream_merman::{
    DEFAULT_CONFIGURATION_VERSION, MERMAID_ARTIFACT_PROTOCOL, MERMAID_MEDIA_TYPE, MermaidProcessor,
};
use mdstream_processors::{
    ArtifactHost, CompletionOutcome, ConfigurationVersion, ContentProcessor, ProcessingPolicy,
    ProcessorArtifact, ProcessorLimits, ProcessorResult, run_catching,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, ContentKind, Reducer, SourceDelta, TransitionFacts,
    TransitionReducer,
};

use support::{EPOCH, NODE_ID, mermaid_document};

#[derive(Debug, PartialEq, Eq)]
enum MermaidDisplayHandoff<'a> {
    SanitizeOrIsolate(&'a str),
}

fn mermaid_display_handoff(artifact: &ProcessorArtifact) -> Option<MermaidDisplayHandoff<'_>> {
    (artifact.protocol() == MERMAID_ARTIFACT_PROTOCOL
        && artifact.media_type() == MERMAID_MEDIA_TYPE)
        .then(|| {
            artifact
                .as_text()
                .map(MermaidDisplayHandoff::SanitizeOrIsolate)
        })
        .flatten()
}

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
    assert!(matches!(
        mermaid_display_handoff(artifact),
        Some(MermaidDisplayHandoff::SanitizeOrIsolate(svg)) if svg.starts_with("<svg")
    ));
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

#[test]
fn recovery_keeps_same_floor_work_and_replaces_advanced_processor_generation() {
    let producer = mermaid_document("flowchart LR\nInput --> A");
    let initial_snapshot = producer.document().unwrap().snapshot();
    let mut reducer = TransitionReducer::new();
    let bootstrap = reducer.recover_snapshot(initial_snapshot.clone()).unwrap();
    assert!(matches!(
        bootstrap.facts,
        Some(TransitionFacts::FullReplace { .. })
    ));

    let processor = MermaidProcessor::default();
    let configuration = ConfigurationVersion::new(DEFAULT_CONFIGURATION_VERSION).unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(EPOCH).unwrap();
    let same_floor_request = host
        .begin(
            reducer.document().unwrap(),
            processor.descriptor().clone(),
            NODE_ID,
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let same_floor_result = run_catching(&processor, &same_floor_request);
    let issued_before_same_floor = host.metrics().issued_requests;

    let coordinate = reducer.document().unwrap().coordinate().clone();
    let gap = ChangeSet::new(
        coordinate.epoch,
        coordinate.sequence.checked_add(2).unwrap(),
        ChangeId::new("adoption:recovery-gap").unwrap(),
        SourceDelta::append(coordinate.source_cursor, "gap"),
        Vec::new(),
    )
    .unwrap();
    let gap_report = reducer.apply(gap.clone()).unwrap();
    assert!(matches!(
        gap_report.outcome,
        ApplyOutcome::RecoveryRequired { .. }
    ));
    assert!(gap_report.facts.is_none());

    let same_floor = reducer.recover_snapshot(initial_snapshot).unwrap();
    assert!(same_floor.facts.is_none());
    let same_floor_impact = match same_floor.outcome {
        ApplyOutcome::Recovered { impact, .. } => impact,
        outcome => panic!("same-floor recovery must recover readiness: {outcome:?}"),
    };
    host.reconcile(reducer.document().unwrap(), &same_floor_impact)
        .unwrap();
    assert!(!same_floor_request.is_cancelled());
    assert_eq!(host.metrics().issued_requests, issued_before_same_floor);
    assert_eq!(
        host.complete(reducer.document().unwrap(), same_floor_result)
            .unwrap(),
        CompletionOutcome::Applied
    );

    let pending_before_advanced = host
        .begin(
            reducer.document().unwrap(),
            processor.descriptor().clone(),
            NODE_ID,
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let pending_result = run_catching(&processor, &pending_before_advanced);
    let issued_before_advanced = host.metrics().issued_requests;

    let mut advanced_producer = mermaid_document("flowchart LR\nInput --> A");
    let advanced_coordinate = advanced_producer.document().unwrap().coordinate().clone();
    advanced_producer
        .apply(
            ChangeSet::new(
                advanced_coordinate.epoch,
                advanced_coordinate.sequence.checked_add(1).unwrap(),
                ChangeId::new("adoption:advanced-snapshot").unwrap(),
                SourceDelta::append(advanced_coordinate.source_cursor, "\n"),
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(matches!(
        reducer.apply(gap).unwrap().outcome,
        ApplyOutcome::RecoveryRequired { .. }
    ));
    let advanced = reducer
        .recover_snapshot(advanced_producer.document().unwrap().snapshot())
        .unwrap();
    assert!(matches!(
        advanced.facts,
        Some(TransitionFacts::FullReplace { .. })
    ));
    let advanced_impact = match advanced.outcome {
        ApplyOutcome::Recovered { impact, .. } => impact,
        outcome => panic!("advanced snapshot must recover: {outcome:?}"),
    };
    assert!(advanced_impact.full_replace);

    let canonical_after_recovery = reducer.document().unwrap().snapshot();
    let transition_metrics_after_recovery = reducer.transition_metrics();
    host.reconcile(reducer.document().unwrap(), &advanced_impact)
        .unwrap();
    assert!(pending_before_advanced.is_cancelled());
    assert_eq!(host.metrics().issued_requests, issued_before_advanced);
    assert_eq!(host.metrics().slots, 0);
    assert_eq!(
        host.complete(reducer.document().unwrap(), pending_result)
            .unwrap(),
        CompletionOutcome::Stale
    );

    let rescanned = host
        .begin(
            reducer.document().unwrap(),
            processor.descriptor().clone(),
            NODE_ID,
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_ne!(
        rescanned.key().generation(),
        pending_before_advanced.key().generation()
    );
    assert_eq!(host.metrics().issued_requests, issued_before_advanced + 1);
    let rescanned_slot = rescanned.key().slot().clone();
    assert_eq!(
        host.complete(
            reducer.document().unwrap(),
            run_catching(&processor, &rescanned),
        )
        .unwrap(),
        CompletionOutcome::Applied
    );
    assert!(matches!(
        mermaid_display_handoff(host.artifact(&rescanned_slot).unwrap()),
        Some(MermaidDisplayHandoff::SanitizeOrIsolate(svg)) if svg.starts_with("<svg")
    ));
    assert_eq!(
        reducer.document().unwrap().snapshot(),
        canonical_after_recovery
    );
    assert_eq!(
        reducer.transition_metrics(),
        transition_metrics_after_recovery
    );
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
