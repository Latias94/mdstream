use mdstream_processors::{
    ArtifactHost, CitationArtifact, CitationProcessor, CompletionOutcome, ConfigurationVersion,
    ContentProcessor, ProcessingPolicy, ProcessorFailureCode, ProcessorLimits, ProcessorSlotState,
    run_catching,
};
use mdstream_protocol::{
    ChangeId, ChangeSet, ChildList, ChildListOwner, CitationProtocol, ContentKind, ContentNode,
    Epoch, NodeId, NodeStability, ProjectionOp, Reducer, ResourceId, SemanticResource,
    SemanticResourceKind, SourceCursor, SourceDelta, SourceRange,
};

fn citation_document(resolved: bool) -> Reducer {
    citation_document_with_destination(resolved, "https://example.test/paper")
}

fn citation_document_with_destination(resolved: bool, destination: &str) -> Reducer {
    let source = "[@paper]";
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(source.len() as u64));
    let resource = SemanticResource::new(
        ResourceId::new(9),
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_string(),
            destination: destination.to_string(),
            title: Some("Streaming Systems".to_string()),
        },
    );
    let citation = ContentNode::leaf(
        NodeId::new(42),
        NodeStability::Stable,
        range,
        ContentKind::CitationReference {
            key: "paper".to_string(),
            target: resolved.then(|| resource.reference()),
        },
    );
    let paragraph = ContentNode::new(
        NodeId::new(41),
        NodeStability::Stable,
        range,
        range,
        Vec::new(),
        ContentKind::Paragraph {},
    );
    let roots = ChildList::new(vec![paragraph.id]);
    let mut operations = vec![
        ProjectionOp::InsertNode {
            node: citation.clone(),
        },
        ProjectionOp::InsertNode {
            node: paragraph.clone(),
        },
        ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Node {
                node_id: paragraph.id,
            },
            expected_version: paragraph.children.version().clone(),
            start: 0,
            delete_count: 0,
            insert: vec![citation.id],
            new_version: paragraph.children.version_after_append(&[citation.id]),
        },
        ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Document,
            expected_version: ChildList::empty().version().clone(),
            start: 0,
            delete_count: 0,
            insert: roots.as_slice().to_vec(),
            new_version: roots.version().clone(),
        },
        ProjectionOp::AdvanceProjection {
            expected_cursor: SourceCursor::new(0),
            new_cursor: SourceCursor::new(source.len() as u64),
        },
    ];
    if resolved {
        operations.insert(0, ProjectionOp::InsertResource { resource });
    }
    let change = ChangeSet::start_epoch(
        Epoch::new(7),
        ChangeId::new(if resolved {
            "citation:resolved"
        } else {
            "citation:unresolved"
        })
        .unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        operations,
    )
    .unwrap();
    let mut reducer = Reducer::new();
    reducer.apply(change).unwrap();
    reducer
}

#[test]
fn citation_processor_resolves_typed_context_through_the_artifact_host() {
    let reducer = citation_document(true);
    let document = reducer.document().unwrap();
    let canonical_before = document.snapshot();
    let processor = CitationProcessor::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(42),
            ConfigurationVersion::new("bibliography.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(
        host.complete(document, run_catching(&processor, &request))
            .unwrap(),
        CompletionOutcome::Applied
    );

    assert_eq!(
        host.artifact(request.key().slot())
            .and_then(|artifact| artifact.as_citation()),
        Some(&CitationArtifact::new(
            "paper",
            "https://example.test/paper",
            Some("Streaming Systems".to_string()),
        ))
    );
    assert_eq!(document.snapshot(), canonical_before);
}

#[test]
fn citation_resource_refresh_and_cancellation_follow_host_freshness() {
    let reducer_a = citation_document_with_destination(true, "https://example.test/a");
    let reducer_b = citation_document_with_destination(true, "https://example.test/b");
    let document_a = reducer_a.document().unwrap();
    let document_b = reducer_b.document().unwrap();
    let processor = CitationProcessor::new();
    let configuration = ConfigurationVersion::new("bibliography.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let stale = host
        .begin(
            document_a,
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let stale_result = run_catching(&processor, &stale);
    let current = host
        .begin(
            document_b,
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert!(stale.is_cancelled());
    assert_ne!(stale.key().input_version(), current.key().input_version());
    assert_eq!(
        host.complete(document_b, stale_result).unwrap(),
        CompletionOutcome::Stale
    );
    assert_eq!(
        host.state(current.key().slot()).unwrap().key(),
        current.key()
    );

    assert_eq!(
        host.complete(document_b, run_catching(&processor, &current))
            .unwrap(),
        CompletionOutcome::Applied
    );
    assert_eq!(
        host.artifact(current.key().slot())
            .and_then(|artifact| artifact.as_citation())
            .map(CitationArtifact::destination),
        Some("https://example.test/b")
    );

    let cancelled = host
        .begin(
            document_b,
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let late_result = run_catching(&processor, &cancelled);
    assert!(host.cancel(cancelled.key()).unwrap());
    assert!(cancelled.is_cancelled());
    assert_eq!(
        host.complete(document_b, late_result).unwrap(),
        CompletionOutcome::Stale
    );
    assert!(host.state(cancelled.key().slot()).is_none());
}

#[test]
fn unresolved_citation_is_a_structured_derived_failure() {
    let reducer = citation_document(false);
    let document = reducer.document().unwrap();
    let processor = CitationProcessor::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(42),
            ConfigurationVersion::new("bibliography.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();

    assert_eq!(
        host.complete(document, run_catching(&processor, &request))
            .unwrap(),
        CompletionOutcome::Applied
    );
    match host.state(request.key().slot()) {
        Some(ProcessorSlotState::Failed { failure, .. }) => {
            assert_eq!(failure.code(), ProcessorFailureCode::UnresolvedContext)
        }
        state => panic!("expected unresolved citation failure, got {state:?}"),
    }
    assert!(host.artifact(request.key().slot()).is_none());
}
