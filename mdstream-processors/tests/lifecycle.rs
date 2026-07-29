use mdstream_processors::{
    ArtifactChangeKind, ArtifactHost, ArtifactReleaseReason, CompletionOutcome,
    ConfigurationVersion, ContentProcessor, HostError, ProcessingPolicy, ProcessorArtifact,
    ProcessorCapabilities, ProcessorDescriptor, ProcessorExpectation, ProcessorFailure,
    ProcessorFailureCode, ProcessorInput, ProcessorLimits, ProcessorSlotState, run_catching,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeImpact, ChangeSet, ChildList, ChildListOwner, CodeBlockSyntax,
    CodeFenceMarker, ContentKind, ContentNode, DocumentLifecycle, Epoch, NodeId, NodeStability,
    ProjectionOp, Reducer, SemanticText, Sequence, SourceCursor, SourceDelta, SourceRange,
};
use std::collections::BTreeMap;

struct EchoProcessor {
    descriptor: ProcessorDescriptor,
}

struct FailingProcessor {
    descriptor: ProcessorDescriptor,
    panic: bool,
}

struct DescriptorPanicProcessor;

impl ContentProcessor for DescriptorPanicProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        panic!("intentional descriptor panic");
    }

    fn process(
        &self,
        _request: &mdstream_processors::ProcessorRequest,
    ) -> Result<ProcessorArtifact, ProcessorFailure> {
        unreachable!()
    }
}

impl FailingProcessor {
    fn new(id: &str, panic: bool) -> Self {
        Self {
            descriptor: ProcessorDescriptor::new(id, "v1", ProcessorCapabilities::stable_only())
                .unwrap(),
            panic,
        }
    }
}

impl ContentProcessor for FailingProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        &self.descriptor
    }

    fn process(
        &self,
        _request: &mdstream_processors::ProcessorRequest,
    ) -> Result<ProcessorArtifact, ProcessorFailure> {
        assert!(!self.panic, "intentional processor panic");
        Err(ProcessorFailure::new(
            ProcessorFailureCode::Processor,
            "processor rejected the input",
        ))
    }
}

impl EchoProcessor {
    fn new() -> Self {
        Self::with_version("v1")
    }

    fn with_version(version: &str) -> Self {
        Self::with_identity("test.echo", version, ProcessorCapabilities::stable_only())
    }

    fn with_capabilities(version: &str, capabilities: ProcessorCapabilities) -> Self {
        Self::with_identity("test.echo", version, capabilities)
    }

    fn with_identity(id: &str, version: &str, capabilities: ProcessorCapabilities) -> Self {
        Self {
            descriptor: ProcessorDescriptor::new(id, version, capabilities).unwrap(),
        }
    }
}

impl ContentProcessor for EchoProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        &self.descriptor
    }

    fn process(
        &self,
        _request: &mdstream_processors::ProcessorRequest,
    ) -> Result<ProcessorArtifact, mdstream_processors::ProcessorFailure> {
        Ok(ProcessorArtifact::text("test.echo.result/1", "text/plain", "rendered").unwrap())
    }
}

fn document_with_code(epoch: u64, node_id: u128, source: &str) -> Reducer {
    document_with_code_stability(epoch, node_id, source, NodeStability::Stable)
}

fn document_with_code_stability(
    epoch: u64,
    node_id: u128,
    source: &str,
    stability: NodeStability,
) -> Reducer {
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(source.len() as u64));
    let node = ContentNode::leaf(
        NodeId::new(node_id),
        stability,
        range,
        ContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced {
                marker: CodeFenceMarker::Backtick,
                length: 3,
            },
            info: Some("text".to_string()),
            text: SemanticText::Source {},
        },
    );
    let roots = ChildList::new(vec![node.id]);
    let change = ChangeSet::start_epoch(
        Epoch::new(epoch),
        ChangeId::new(format!("epoch:{epoch}")).unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        vec![
            ProjectionOp::InsertNode { node },
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
        ],
    )
    .unwrap();
    let mut reducer = Reducer::new();
    reducer.apply(change).unwrap();
    reducer
}

fn document_with_custom_child(epoch: u64, root_id: u128, child_id: u128) -> Reducer {
    let source = "x";
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(1));
    let child = ContentNode::leaf(
        NodeId::new(child_id),
        NodeStability::Stable,
        range,
        ContentKind::Text {
            text: SemanticText::Source {},
        },
    );
    let root = ContentNode::new(
        NodeId::new(root_id),
        NodeStability::Stable,
        range,
        range,
        Vec::new(),
        ContentKind::Custom {
            namespace: "test.content".to_string(),
            name: "container".to_string(),
            opaque: false,
            attributes: BTreeMap::new(),
        },
    );
    let roots = ChildList::new(vec![root.id]);
    let change = ChangeSet::start_epoch(
        Epoch::new(epoch),
        ChangeId::new(format!("epoch:{epoch}:child:{child_id}")).unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        vec![
            ProjectionOp::InsertNode { node: child },
            ProjectionOp::InsertNode { node: root.clone() },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node { node_id: root.id },
                expected_version: root.children.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![NodeId::new(child_id)],
                new_version: root.children.version_after_append(&[NodeId::new(child_id)]),
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
                new_cursor: SourceCursor::new(1),
            },
        ],
    )
    .unwrap();
    let mut reducer = Reducer::new();
    reducer.apply(change).unwrap();
    reducer
}

#[test]
fn conditional_begin_rejects_changed_child_structure_with_stable_node_version() {
    let root_id = NodeId::new(41);
    let child_id = NodeId::new(42);
    let mut reducer = document_with_custom_child(7, root_id.get(), child_id.get());
    let document = reducer.document().unwrap();
    let expected_node_version = document.node(root_id).unwrap().version.clone();
    let expected_input_version = ProcessorInput::from_document(document, root_id)
        .unwrap()
        .version()
        .clone();
    let current_children = document.node(root_id).unwrap().children.clone();
    let child_version = document.node(child_id).unwrap().version.clone();
    let change = ChangeSet::new(
        Epoch::new(7),
        Sequence::new(1),
        ChangeId::new("epoch:7:remove-child").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(1)),
        vec![
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node { node_id: root_id },
                expected_version: current_children.version().clone(),
                start: 0,
                delete_count: 1,
                insert: Vec::new(),
                new_version: ChildList::empty().version().clone(),
            },
            ProjectionOp::RemoveNode {
                node_id: child_id,
                expected_version: child_version,
            },
        ],
    )
    .unwrap();
    reducer.apply(change).unwrap();
    let document = reducer.document().unwrap();
    assert_eq!(
        document.node(root_id).unwrap().version,
        expected_node_version
    );

    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let request = host
        .begin_if_current(
            document,
            ProcessorExpectation::new(Epoch::new(7), root_id, expected_input_version),
            EchoProcessor::new().descriptor().clone(),
            ConfigurationVersion::new("config.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();

    assert!(request.is_none());
    assert_eq!(host.metrics().issued_requests, 0);
    assert_eq!(host.metrics().slots, 0);
}

#[test]
fn matching_result_installs_one_artifact_without_changing_canonical_state() {
    let reducer = document_with_code(7, 41, "hello");
    let document = reducer.document().unwrap();
    let canonical_before = document.snapshot();
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(document.coordinate().epoch).unwrap();

    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = request.key().slot().clone();
    assert_eq!(host.metrics().in_flight_jobs, 1);
    assert_eq!(host.metrics().retained_artifacts, 0);

    let result = run_catching(&processor, &request);
    assert_eq!(
        host.complete(document, result).unwrap(),
        CompletionOutcome::Applied
    );

    assert_eq!(host.artifact(&slot).unwrap().as_text(), Some("rendered"));
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(host.metrics().retained_artifacts, 1);
    assert_eq!(document.snapshot(), canonical_before);
}

#[test]
fn replacement_releases_artifact_and_generation_closes_the_a_b_a_race() {
    let reducer_a = document_with_code(7, 41, "A");
    let reducer_b = document_with_code(7, 41, "B");
    let document_a = reducer_a.document().unwrap();
    let document_b = reducer_b.document().unwrap();
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let installed = host
        .begin(
            document_a,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = installed.key().slot().clone();
    host.complete(document_a, run_catching(&processor, &installed))
        .unwrap();
    assert!(host.artifact(&slot).is_some());

    let first_a = host
        .begin(
            document_a,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let first_a_result = run_catching(&processor, &first_a);
    assert!(host.artifact(&slot).is_none());
    assert_eq!(host.metrics().released_artifacts, 1);

    let request_b = host
        .begin(
            document_b,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert!(first_a.is_cancelled());

    let second_a = host
        .begin(
            document_a,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert!(request_b.is_cancelled());
    assert_eq!(first_a.key().node_version(), second_a.key().node_version());
    assert_eq!(
        first_a.key().input_version(),
        second_a.key().input_version()
    );
    assert_ne!(first_a.key().generation(), second_a.key().generation());
    assert_eq!(host.metrics().in_flight_jobs, 3);

    assert_eq!(
        host.complete(document_a, first_a_result).unwrap(),
        CompletionOutcome::Stale
    );
    assert_eq!(host.state(&slot).unwrap().key(), second_a.key());
    assert!(host.artifact(&slot).is_none());

    assert_eq!(
        host.complete(document_a, run_catching(&processor, &second_a))
            .unwrap(),
        CompletionOutcome::Applied
    );
    assert_eq!(host.artifact(&slot).unwrap().as_text(), Some("rendered"));
}

#[test]
fn node_removal_wins_pending_and_ready_result_races() {
    let reducer = document_with_code(7, 41, "hello");
    let document = reducer.document().unwrap();
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let pending = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let pending_result = run_catching(&processor, &pending);
    let slot = pending.key().slot().clone();
    host.take_changes();
    host.remove_node(Epoch::new(7), NodeId::new(41)).unwrap();
    assert_eq!(
        host.take_changes()[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeRemoved,
            released_artifact_bytes: 0,
        }
    );
    assert!(pending.is_cancelled());
    assert!(host.state(&slot).is_none());
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(
        host.complete(document, pending_result).unwrap(),
        CompletionOutcome::Stale
    );

    let ready = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document, run_catching(&processor, &ready))
        .unwrap();
    assert_eq!(host.metrics().retained_artifacts, 1);
    host.take_changes();
    host.remove_node(Epoch::new(7), NodeId::new(41)).unwrap();
    assert_eq!(
        host.take_changes()[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeRemoved,
            released_artifact_bytes: 36,
        }
    );
    assert!(host.state(&slot).is_none());
    assert_eq!(host.metrics().retained_artifacts, 0);
    assert_eq!(host.metrics().retained_artifact_bytes, 0);
    assert_eq!(host.metrics().released_artifacts, 1);
}

#[test]
fn epoch_reset_drains_ready_and_in_flight_state_without_reusing_generation() {
    let reducer_ready = document_with_code(7, 41, "ready");
    let reducer_pending = document_with_code(7, 42, "pending");
    let document_ready = reducer_ready.document().unwrap();
    let document_pending = reducer_pending.document().unwrap();
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let ready = host
        .begin(
            document_ready,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document_ready, run_catching(&processor, &ready))
        .unwrap();
    let pending = host
        .begin(
            document_pending,
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let pending_result = run_catching(&processor, &pending);
    let last_old_generation = pending.key().generation();

    host.take_changes();
    host.begin_epoch(Epoch::new(8)).unwrap();
    let reset_changes = host.take_changes();
    assert_eq!(reset_changes.len(), 2);
    assert!(reset_changes.iter().all(|change| matches!(
        change.kind(),
        ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::EpochReset,
            ..
        }
    )));
    assert!(pending.is_cancelled());
    assert_eq!(host.metrics().slots, 0);
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(host.metrics().in_flight_input_bytes, 0);
    assert_eq!(host.metrics().retained_artifacts, 0);
    assert_eq!(host.metrics().retained_artifact_bytes, 0);
    assert_eq!(host.metrics().released_artifacts, 1);
    assert_eq!(
        host.complete(document_pending, pending_result).unwrap(),
        CompletionOutcome::Stale
    );

    let before_idempotent_reset = host.metrics();
    host.begin_epoch(Epoch::new(8)).unwrap();
    assert_eq!(host.metrics(), before_idempotent_reset);

    let reducer_new = document_with_code(8, 41, "new epoch");
    let request_new = host
        .begin(
            reducer_new.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert!(request_new.key().generation() > last_old_generation);
}

#[test]
fn processor_and_configuration_versions_supersede_and_duplicate_completion_is_stale() {
    let reducer = document_with_code(7, 41, "hello");
    let document = reducer.document().unwrap();
    let processor_v1 = EchoProcessor::with_version("v1");
    let processor_v2 = EchoProcessor::with_version("v2");
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let request_v1 = host
        .begin(
            document,
            processor_v1.descriptor().clone(),
            NodeId::new(41),
            ConfigurationVersion::new("config.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let result_v1 = run_catching(&processor_v1, &request_v1);

    let request_v2 = host
        .begin(
            document,
            processor_v2.descriptor().clone(),
            NodeId::new(41),
            ConfigurationVersion::new("config.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let result_v2_config_v1 = run_catching(&processor_v2, &request_v2);
    assert_eq!(
        host.complete(document, result_v1).unwrap(),
        CompletionOutcome::Stale
    );

    let current = host
        .begin(
            document,
            processor_v2.descriptor().clone(),
            NodeId::new(41),
            ConfigurationVersion::new("config.v2").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(
        host.complete(document, result_v2_config_v1).unwrap(),
        CompletionOutcome::Stale
    );

    let current_result = run_catching(&processor_v2, &current);
    let duplicate = current_result.clone();
    assert_eq!(
        host.complete(document, current_result).unwrap(),
        CompletionOutcome::Applied
    );
    let slot = current.key().slot();
    let installed = host.artifact(slot).unwrap().clone();
    assert_eq!(
        host.complete(document, duplicate).unwrap(),
        CompletionOutcome::Stale
    );
    assert_eq!(host.artifact(slot), Some(&installed));
    assert_eq!(host.metrics().accepted_results, 1);
    assert_eq!(host.metrics().stale_results, 3);
}

#[test]
fn direct_child_identity_changes_processor_input_without_changing_projection_version() {
    let reducer_left = document_with_custom_child(7, 41, 42);
    let reducer_right = document_with_custom_child(7, 41, 43);
    let document_left = reducer_left.document().unwrap();
    let document_right = reducer_right.document().unwrap();
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let left = host
        .begin(
            document_left,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let left_result = run_catching(&processor, &left);
    let right = host
        .begin(
            document_right,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();

    assert_eq!(left.key().node_version(), right.key().node_version());
    assert_ne!(left.key().input_version(), right.key().input_version());
    assert_eq!(
        host.complete(document_right, left_result).unwrap(),
        CompletionOutcome::Stale
    );
    assert_eq!(host.state(right.key().slot()).unwrap().key(), right.key());
}

#[test]
fn provisional_processing_requires_capability_and_explicit_policy() {
    let reducer = document_with_code_stability(7, 41, "pending", NodeStability::Provisional);
    let document = reducer.document().unwrap();
    let stable_only = EchoProcessor::new();
    let provisional =
        EchoProcessor::with_capabilities("v1", ProcessorCapabilities::with_provisional());
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    for (descriptor, policy) in [
        (
            stable_only.descriptor().clone(),
            ProcessingPolicy::StableOnly,
        ),
        (
            provisional.descriptor().clone(),
            ProcessingPolicy::StableOnly,
        ),
        (
            stable_only.descriptor().clone(),
            ProcessingPolicy::AllowProvisional,
        ),
    ] {
        assert!(matches!(
            host.begin(
                document,
                descriptor,
                NodeId::new(41),
                configuration.clone(),
                policy,
            ),
            Err(mdstream_processors::HostError::ProvisionalProcessingDisabled(node_id))
                if node_id == NodeId::new(41)
        ));
        assert_eq!(host.metrics(), Default::default());
    }

    let request = host
        .begin(
            document,
            provisional.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::AllowProvisional,
        )
        .unwrap();
    assert_eq!(request.key().generation().get(), 1);
    assert_eq!(host.metrics().in_flight_jobs, 1);
}

#[test]
#[cfg(panic = "unwind")]
fn processor_failure_and_panic_become_derived_state_and_host_remains_usable() {
    let reducer = document_with_code(7, 41, "hello");
    let document = reducer.document().unwrap();
    let canonical_before = document.snapshot();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let failure = FailingProcessor::new("test.failure", false);
    let panic = FailingProcessor::new("test.panic", true);
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    for (processor, expected_code) in [
        (&failure, ProcessorFailureCode::Processor),
        (&panic, ProcessorFailureCode::Panic),
    ] {
        let request = host
            .begin(
                document,
                processor.descriptor().clone(),
                NodeId::new(41),
                configuration.clone(),
                ProcessingPolicy::StableOnly,
            )
            .unwrap();
        assert_eq!(
            host.complete(document, run_catching(processor, &request))
                .unwrap(),
            CompletionOutcome::Applied
        );
        match host.state(request.key().slot()) {
            Some(ProcessorSlotState::Failed { failure, .. }) => {
                assert_eq!(failure.code(), expected_code)
            }
            state => panic!("expected a structured processor failure, got {state:?}"),
        }
    }

    let echo = EchoProcessor::new();
    let request = host
        .begin(
            document,
            echo.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document, run_catching(&echo, &request))
        .unwrap();
    assert_eq!(
        host.artifact(request.key().slot()).unwrap().as_text(),
        Some("rendered")
    );
    assert_eq!(document.snapshot(), canonical_before);
}

#[test]
#[cfg(panic = "unwind")]
fn processor_descriptor_panic_is_contained_by_the_execution_adapter() {
    let reducer = document_with_code(7, 41, "hello");
    let document = reducer.document().unwrap();
    let request_processor = EchoProcessor::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let request = host
        .begin(
            document,
            request_processor.descriptor().clone(),
            NodeId::new(41),
            ConfigurationVersion::new("config.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();

    assert_eq!(
        host.complete(document, run_catching(&DescriptorPanicProcessor, &request))
            .unwrap(),
        CompletionOutcome::Applied
    );
    match host.state(request.key().slot()) {
        Some(ProcessorSlotState::Failed { failure, .. }) => {
            assert_eq!(failure.code(), ProcessorFailureCode::Panic)
        }
        state => panic!("expected a contained descriptor panic, got {state:?}"),
    }
}

#[test]
fn document_finish_does_not_wait_for_or_cancel_pending_processors() {
    let mut reducer = document_with_code(7, 41, "hello");
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let request = host
        .begin(
            reducer.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let result = run_catching(&processor, &request);

    let coordinate = reducer.document().unwrap().coordinate().clone();
    let finish = ChangeSet::new(
        coordinate.epoch,
        Sequence::new(1),
        ChangeId::new("finish").unwrap(),
        SourceDelta::unchanged(coordinate.source_cursor),
        vec![ProjectionOp::FinishDocument],
    )
    .unwrap();
    let impact = match reducer.apply(finish).unwrap() {
        ApplyOutcome::Applied { impact, .. } => impact,
        outcome => panic!("expected finalization to apply, got {outcome:?}"),
    };
    let finalized = reducer.document().unwrap();
    assert_eq!(finalized.lifecycle(), DocumentLifecycle::Finalized);
    host.reconcile(finalized, &impact).unwrap();

    assert!(!request.is_cancelled());
    assert_eq!(host.metrics().in_flight_jobs, 1);
    assert_eq!(
        host.complete(finalized, result).unwrap(),
        CompletionOutcome::Applied
    );
    assert!(host.artifact(request.key().slot()).is_some());
}

#[test]
fn reconcile_invalidates_ready_and_pending_state_for_changed_nodes() {
    let reducer_a = document_with_code(7, 41, "A");
    let reducer_b = document_with_code(7, 41, "B");
    let document_a = reducer_a.document().unwrap();
    let document_b = reducer_b.document().unwrap();
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let impact = ChangeImpact {
        changed_nodes: vec![NodeId::new(41)],
        projection_changed: true,
        ..ChangeImpact::default()
    };

    let ready = host
        .begin(
            document_a,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document_a, run_catching(&processor, &ready))
        .unwrap();
    host.take_changes();
    host.reconcile(document_b, &impact).unwrap();
    assert_eq!(
        host.take_changes()[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeChanged,
            released_artifact_bytes: 36,
        }
    );
    assert!(host.state(ready.key().slot()).is_none());
    assert_eq!(host.metrics().retained_artifacts, 0);
    assert_eq!(host.metrics().released_artifacts, 1);

    let pending = host
        .begin(
            document_a,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let result = run_catching(&processor, &pending);
    host.take_changes();
    host.reconcile(document_b, &impact).unwrap();
    assert_eq!(
        host.take_changes()[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeChanged,
            released_artifact_bytes: 0,
        }
    );
    assert!(pending.is_cancelled());
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(
        host.complete(document_b, result).unwrap(),
        CompletionOutcome::Stale
    );
}

#[test]
fn reconcile_preserves_artifacts_and_leases_when_reparenting_keeps_processor_input_current() {
    let left_id = NodeId::new(41);
    let right_id = NodeId::new(42);
    let child_id = NodeId::new(43);
    let left = ContentNode::leaf(
        left_id,
        NodeStability::Stable,
        SourceRange::new(SourceCursor::new(0), SourceCursor::new(1)),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let right = ContentNode::leaf(
        right_id,
        NodeStability::Stable,
        SourceRange::new(SourceCursor::new(1), SourceCursor::new(2)),
        ContentKind::BlockQuote {
            style: Default::default(),
        },
    );
    let child = ContentNode::leaf(
        child_id,
        NodeStability::Stable,
        SourceRange::new(SourceCursor::new(1), SourceCursor::new(1)),
        ContentKind::Paragraph {},
    );
    let roots = ChildList::new(vec![left_id, right_id]);
    let left_children = ChildList::new(vec![child_id]);
    let mut reducer = Reducer::new();
    reducer
        .apply(
            ChangeSet::start_epoch(
                Epoch::new(7),
                ChangeId::new("reparent:start").unwrap(),
                None,
                SourceDelta::append(SourceCursor::new(0), "ab"),
                vec![
                    ProjectionOp::InsertNode { node: left },
                    ProjectionOp::InsertNode { node: right },
                    ProjectionOp::InsertNode { node: child },
                    ProjectionOp::SpliceChildren {
                        owner: ChildListOwner::Node { node_id: left_id },
                        expected_version: ChildList::empty().version().clone(),
                        start: 0,
                        delete_count: 0,
                        insert: vec![child_id],
                        new_version: left_children.version().clone(),
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
                        new_cursor: SourceCursor::new(2),
                    },
                ],
            )
            .unwrap(),
        )
        .unwrap();

    let ready_processor = EchoProcessor::new();
    let pending_processor = FailingProcessor::new("test.reparent.pending", false);
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let initial_document = reducer.document().unwrap();
    let ready = host
        .begin(
            initial_document,
            ready_processor.descriptor().clone(),
            child_id,
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(initial_document, run_catching(&ready_processor, &ready))
        .unwrap();
    let pending = host
        .begin(
            initial_document,
            pending_processor.descriptor().clone(),
            child_id,
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let pending_result = run_catching(&pending_processor, &pending);
    let initial_input_version = pending.input().version().clone();
    host.take_changes();

    let right_children = ChildList::new(vec![child_id]);
    let impact = match reducer
        .apply(
            ChangeSet::new(
                Epoch::new(7),
                Sequence::new(1),
                ChangeId::new("reparent:left-to-right").unwrap(),
                SourceDelta::unchanged(SourceCursor::new(2)),
                vec![
                    ProjectionOp::SpliceChildren {
                        owner: ChildListOwner::Node { node_id: left_id },
                        expected_version: left_children.version().clone(),
                        start: 0,
                        delete_count: 1,
                        insert: Vec::new(),
                        new_version: ChildList::empty().version().clone(),
                    },
                    ProjectionOp::SpliceChildren {
                        owner: ChildListOwner::Node { node_id: right_id },
                        expected_version: ChildList::empty().version().clone(),
                        start: 0,
                        delete_count: 0,
                        insert: vec![child_id],
                        new_version: right_children.version().clone(),
                    },
                ],
            )
            .unwrap(),
        )
        .unwrap()
    {
        ApplyOutcome::Applied { impact, .. } => impact,
        outcome => panic!("expected reparenting to apply, got {outcome:?}"),
    };
    assert!(impact.changed_nodes.contains(&child_id));
    let reparented = reducer.document().unwrap();
    assert_eq!(
        ProcessorInput::from_document(reparented, child_id)
            .unwrap()
            .version(),
        &initial_input_version
    );

    host.reconcile(reparented, &impact).unwrap();

    assert!(host.take_changes().is_empty());
    assert!(host.artifact(ready.key().slot()).is_some());
    assert!(!pending.is_cancelled());
    assert_eq!(host.metrics().retained_artifacts, 1);
    assert_eq!(host.metrics().in_flight_jobs, 1);
    assert_eq!(
        host.complete(reparented, pending_result).unwrap(),
        CompletionOutcome::Applied
    );
}

#[test]
fn same_epoch_full_replace_invalidates_all_derived_state() {
    let mut producer = document_with_code(7, 41, "hello");
    let mut consumer = document_with_code(7, 41, "hello");
    let document = consumer.document().unwrap();
    let ready_processor = EchoProcessor::new();
    let pending_processor = FailingProcessor::new("test.pending", false);
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let ready = host
        .begin(
            document,
            ready_processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document, run_catching(&ready_processor, &ready))
        .unwrap();
    let pending = host
        .begin(
            document,
            pending_processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let pending_result = run_catching(&pending_processor, &pending);
    host.take_changes();

    let coordinate = producer.document().unwrap().coordinate().clone();
    producer
        .apply(
            ChangeSet::new(
                coordinate.epoch,
                Sequence::new(1),
                ChangeId::new("append:producer").unwrap(),
                SourceDelta::append(coordinate.source_cursor, "!"),
                vec![],
            )
            .unwrap(),
        )
        .unwrap();
    assert!(matches!(
        consumer
            .apply(
                ChangeSet::new(
                    coordinate.epoch,
                    Sequence::new(2),
                    ChangeId::new("append:gap").unwrap(),
                    SourceDelta::append(coordinate.source_cursor, "?"),
                    vec![],
                )
                .unwrap(),
            )
            .unwrap(),
        ApplyOutcome::RecoveryRequired { .. }
    ));
    let impact = match consumer
        .recover_snapshot(producer.document().unwrap().snapshot())
        .unwrap()
    {
        ApplyOutcome::Recovered { impact, .. } => impact,
        outcome => panic!("expected snapshot recovery, got {outcome:?}"),
    };
    assert!(impact.full_replace);
    assert!(impact.changed_nodes.is_empty());
    assert!(impact.source_changed);
    let recovered = consumer.document().unwrap();
    host.reconcile(recovered, &impact).unwrap();

    assert!(pending.is_cancelled());
    assert_eq!(host.metrics().slots, 0);
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(host.metrics().retained_artifacts, 0);
    let changes = host.take_changes();
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().all(|change| matches!(
        change.kind(),
        ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeChanged,
            ..
        }
    )));
    let mut released_bytes = changes
        .iter()
        .map(|change| match change.kind() {
            ArtifactChangeKind::Removed {
                released_artifact_bytes,
                ..
            } => *released_artifact_bytes,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    released_bytes.sort_unstable();
    assert_eq!(released_bytes, vec![0, 36]);
    assert_eq!(
        host.complete(recovered, pending_result).unwrap(),
        CompletionOutcome::Stale
    );
}

#[test]
fn cancellation_is_key_scoped_idempotent_and_rejects_late_results() {
    let reducer = document_with_code(7, 41, "hello");
    let document = reducer.document().unwrap();
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let result = run_catching(&processor, &request);

    assert!(host.cancel(request.key()).unwrap());
    assert!(request.is_cancelled());
    assert!(host.state(request.key().slot()).is_none());
    assert_eq!(host.metrics().in_flight_jobs, 0);
    let after_cancel = host.metrics();
    assert!(!host.cancel(request.key()).unwrap());
    assert_eq!(host.metrics(), after_cancel);
    assert_eq!(
        host.complete(document, result).unwrap(),
        CompletionOutcome::Stale
    );
}

#[test]
fn artifact_changes_expose_state_without_copying_payloads_into_canonical_ir() {
    let reducer = document_with_code(7, 41, "hello");
    let document = reducer.document().unwrap();
    let canonical_before = document.snapshot();
    let processor = EchoProcessor::new();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let first = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let changes = host.take_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].key(), first.key());
    assert_eq!(changes[0].kind(), &ArtifactChangeKind::Pending);

    host.complete(document, run_catching(&processor, &first))
        .unwrap();
    let changes = host.take_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].key(), first.key());
    assert_eq!(
        changes[0].kind(),
        &ArtifactChangeKind::Ready { artifact_bytes: 36 }
    );

    let second = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let changes = host.take_changes();
    assert_eq!(changes.len(), 2);
    assert_eq!(
        changes[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::Replaced,
            released_artifact_bytes: 36,
        }
    );
    assert_eq!(changes[1].kind(), &ArtifactChangeKind::Pending);
    assert_eq!(changes[1].key(), second.key());

    assert!(host.cancel(second.key()).unwrap());
    let changes = host.take_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::Cancelled,
            released_artifact_bytes: 0,
        }
    );
    assert_eq!(document.snapshot(), canonical_before);
    assert_no_artifact_fields(&serde_json::to_value(document.snapshot()).unwrap());
}

#[test]
fn completion_revalidates_the_document_when_reconcile_was_not_called() {
    let reducer_a = document_with_code(7, 41, "A");
    let reducer_b = document_with_code(7, 41, "B");
    let document_a = reducer_a.document().unwrap();
    let document_b = reducer_b.document().unwrap();
    let processor = EchoProcessor::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let request = host
        .begin(
            document_a,
            processor.descriptor().clone(),
            NodeId::new(41),
            ConfigurationVersion::new("config.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let result = run_catching(&processor, &request);
    host.take_changes();

    assert_eq!(
        host.complete(document_b, result).unwrap(),
        CompletionOutcome::Stale
    );
    assert!(host.state(request.key().slot()).is_none());
    assert_eq!(host.metrics().slots, 0);
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(host.metrics().stale_results, 1);
    assert_eq!(
        host.take_changes()[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeChanged,
            released_artifact_bytes: 0,
        }
    );
}

#[test]
fn completion_reports_node_removal_when_the_current_document_omits_the_node() {
    let reducer = document_with_code(7, 41, "A");
    let without_node = document_with_code(7, 42, "A");
    let document = reducer.document().unwrap();
    let processor = EchoProcessor::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            ConfigurationVersion::new("config.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let result = run_catching(&processor, &request);
    host.take_changes();

    assert_eq!(
        host.complete(without_node.document().unwrap(), result)
            .unwrap(),
        CompletionOutcome::Stale
    );
    assert_eq!(
        host.take_changes()[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeRemoved,
            released_artifact_bytes: 0,
        }
    );
}

#[test]
fn completion_with_the_wrong_document_epoch_is_retryable() {
    let reducer_old = document_with_code(7, 41, "A");
    let reducer_other = document_with_code(8, 41, "A");
    let document_old = reducer_old.document().unwrap();
    let processor = EchoProcessor::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let request = host
        .begin(
            document_old,
            processor.descriptor().clone(),
            NodeId::new(41),
            ConfigurationVersion::new("config.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let result = run_catching(&processor, &request);
    host.take_changes();
    let before_state = host.state(request.key().slot()).unwrap().clone();
    let before_metrics = host.metrics();

    let error = host
        .complete(reducer_other.document().unwrap(), result)
        .unwrap_err();
    assert!(matches!(
        error.error(),
        HostError::EpochMismatch {
            current,
            received,
        } if *current == Epoch::new(7) && *received == Epoch::new(8)
    ));
    assert_eq!(host.state(request.key().slot()), Some(&before_state));
    assert_eq!(host.metrics(), before_metrics);
    assert!(!request.is_cancelled());
    assert!(host.take_changes().is_empty());
    assert_eq!(
        host.complete(document_old, error.into_result()).unwrap(),
        CompletionOutcome::Applied
    );
}

#[test]
fn stale_completion_only_invalidates_its_processor_slot() {
    let reducer_a = document_with_code(7, 41, "A");
    let reducer_b = document_with_code(7, 41, "B");
    let document_a = reducer_a.document().unwrap();
    let document_b = reducer_b.document().unwrap();
    let stale_processor =
        EchoProcessor::with_identity("test.stale", "v1", ProcessorCapabilities::stable_only());
    let current_processor =
        EchoProcessor::with_identity("test.current", "v1", ProcessorCapabilities::stable_only());
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let stale = host
        .begin(
            document_a,
            stale_processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let stale_result = run_catching(&stale_processor, &stale);
    let current = host
        .begin(
            document_b,
            current_processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document_b, run_catching(&current_processor, &current))
        .unwrap();
    let current_artifact = host.artifact(current.key().slot()).unwrap().clone();
    host.take_changes();

    assert_eq!(
        host.complete(document_b, stale_result).unwrap(),
        CompletionOutcome::Stale
    );
    assert!(host.state(stale.key().slot()).is_none());
    assert_eq!(host.artifact(current.key().slot()), Some(&current_artifact));
    let changes = host.take_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].key(), stale.key());
    assert_eq!(
        changes[0].kind(),
        &ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeChanged,
            released_artifact_bytes: 0,
        }
    );
}

fn assert_no_artifact_fields(value: &serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (name, value) in fields {
                assert!(
                    !name.contains("artifact"),
                    "canonical snapshot contains artifact field `{name}`"
                );
                assert_no_artifact_fields(value);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                assert_no_artifact_fields(value);
            }
        }
        _ => {}
    }
}
