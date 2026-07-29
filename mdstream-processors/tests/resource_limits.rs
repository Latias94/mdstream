use mdstream_processors::{
    ArtifactChangeKind, ArtifactHost, ArtifactReleaseReason, CompletionOutcome,
    ConfigurationVersion, ContentProcessor, ProcessingPolicy, ProcessorArtifact,
    ProcessorCapabilities, ProcessorDescriptor, ProcessorFailure, ProcessorFailureCode,
    ProcessorInput, ProcessorLimits, ProcessorLimitsError, ProcessorRequest, ProcessorResult,
    ProcessorSlotState, run_catching,
};
use mdstream_protocol::{
    ChangeId, ChangeImpact, ChangeSet, ChildList, ChildListOwner, CodeBlockSyntax, CodeFenceMarker,
    ContentKind, ContentNode, Epoch, NodeId, NodeStability, ProjectionOp, Reducer, SemanticText,
    SourceCursor, SourceDelta, SourceRange,
};

struct FixedProcessor {
    descriptor: ProcessorDescriptor,
    text: &'static str,
}

struct FailingProcessor {
    descriptor: ProcessorDescriptor,
    message: &'static str,
}

impl FailingProcessor {
    fn new(message: &'static str) -> Self {
        Self {
            descriptor: ProcessorDescriptor::new(
                "test.failure",
                "v1",
                ProcessorCapabilities::stable_only(),
            )
            .unwrap(),
            message,
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
        Err(ProcessorFailure::new(
            ProcessorFailureCode::Processor,
            self.message,
        ))
    }
}

impl FixedProcessor {
    fn new(text: &'static str) -> Self {
        Self {
            descriptor: ProcessorDescriptor::new(
                "test.fixed",
                "v1",
                ProcessorCapabilities::stable_only(),
            )
            .unwrap(),
            text,
        }
    }
}

impl ContentProcessor for FixedProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        &self.descriptor
    }

    fn process(
        &self,
        _request: &mdstream_processors::ProcessorRequest,
    ) -> Result<ProcessorArtifact, ProcessorFailure> {
        Ok(ProcessorArtifact::text("test.fixed.result/1", "text/plain", self.text).unwrap())
    }
}

fn document(epoch: u64, node_id: u128, source: &str) -> Reducer {
    document_with_info(epoch, node_id, source, "text")
}

fn document_with_info(epoch: u64, node_id: u128, source: &str, info: &str) -> Reducer {
    let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(source.len() as u64));
    let node = ContentNode::leaf(
        NodeId::new(node_id),
        NodeStability::Stable,
        range,
        ContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced {
                marker: CodeFenceMarker::Backtick,
                length: 3,
            },
            info: Some(info.to_string()),
            text: SemanticText::Source {},
        },
    );
    let roots = ChildList::new(vec![node.id]);
    let mut operations = vec![
        ProjectionOp::InsertNode { node },
        ProjectionOp::SpliceChildren {
            owner: ChildListOwner::Document,
            expected_version: ChildList::empty().version().clone(),
            start: 0,
            delete_count: 0,
            insert: roots.as_slice().to_vec(),
            new_version: roots.version().clone(),
        },
    ];
    if !source.is_empty() {
        operations.push(ProjectionOp::AdvanceProjection {
            expected_cursor: SourceCursor::new(0),
            new_cursor: SourceCursor::new(source.len() as u64),
        });
    }
    let change = ChangeSet::start_epoch(
        Epoch::new(epoch),
        ChangeId::new(format!("epoch:{epoch}:node:{node_id}")).unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        operations,
    )
    .unwrap();
    let mut reducer = Reducer::new();
    reducer.apply(change).unwrap();
    reducer
}

fn measured_input_bytes(document: &mdstream_protocol::Document, node_id: NodeId) -> usize {
    ProcessorInput::from_document(document, node_id)
        .unwrap()
        .byte_len()
}

const MAX_REQUEST_KEY_BYTES: usize = 8 + 16 + 8 + 128 + 71 + 71 + 128 + 128;
const MAX_PENDING_CHANGE_BYTES: usize = MAX_REQUEST_KEY_BYTES + 1;
const MAX_READY_CHANGE_BYTES: usize = MAX_REQUEST_KEY_BYTES + 1 + 8;
const MAX_FAILED_CHANGE_BYTES: usize = MAX_REQUEST_KEY_BYTES + 1 + 1;
const MAX_REMOVED_CHANGE_BYTES: usize = MAX_REQUEST_KEY_BYTES + 1 + 1 + 8;

fn maximal_descriptor() -> ProcessorDescriptor {
    ProcessorDescriptor::new(
        "p".repeat(128),
        "v".repeat(128),
        ProcessorCapabilities::stable_only(),
    )
    .unwrap()
}

fn maximal_configuration() -> ConfigurationVersion {
    ConfigurationVersion::new("c".repeat(128)).unwrap()
}

fn host_with_pending_byte_limit(limit: usize) -> ArtifactHost {
    ArtifactHost::new(ProcessorLimits {
        max_slots: 1,
        max_pending_changes: 2,
        max_pending_change_bytes: limit,
        ..ProcessorLimits::default()
    })
    .unwrap()
}

fn begin_maximal_request(
    host: &mut ArtifactHost,
    document: &mdstream_protocol::Document,
) -> ProcessorRequest {
    host.begin(
        document,
        maximal_descriptor(),
        NodeId::new(41),
        maximal_configuration(),
        ProcessingPolicy::StableOnly,
    )
    .unwrap()
}

#[test]
fn cleanup_count_capacity_is_validated_at_construction() {
    let invalid = ProcessorLimits {
        max_slots: 2,
        max_pending_changes: 1,
        ..ProcessorLimits::default()
    };
    let expected = ProcessorLimitsError::PendingChangesTooSmall {
        required: 2,
        actual: 1,
    };

    assert_eq!(invalid.validate(), Err(expected));
    assert_eq!(ArtifactHost::new(invalid).err(), Some(expected));

    let exact = ProcessorLimits {
        max_slots: 2,
        max_pending_changes: 2,
        ..ProcessorLimits::default()
    };
    exact.validate().unwrap();
    ArtifactHost::new(exact).unwrap();
}

#[test]
fn cleanup_byte_capacity_is_validated_at_construction() {
    let required = 2 * MAX_REMOVED_CHANGE_BYTES;
    let invalid = ProcessorLimits {
        max_slots: 2,
        max_pending_change_bytes: required - 1,
        ..ProcessorLimits::default()
    };
    let expected = ProcessorLimitsError::PendingChangeBytesTooSmall {
        required,
        actual: required - 1,
    };

    assert_eq!(invalid.validate(), Err(expected));
    assert_eq!(ArtifactHost::new(invalid).err(), Some(expected));

    let exact = ProcessorLimits {
        max_slots: 2,
        max_pending_change_bytes: required,
        ..ProcessorLimits::default()
    };
    exact.validate().unwrap();
    ArtifactHost::new(exact).unwrap();
}

#[test]
fn artifact_change_byte_costs_match_independent_goldens() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let descriptor = maximal_descriptor();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let first = host
        .begin(
            document,
            descriptor.clone(),
            NodeId::new(41),
            maximal_configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let pending = host.take_changes();
    assert_eq!(
        pending[0].checked_byte_len(),
        Some(MAX_PENDING_CHANGE_BYTES)
    );

    let artifact = ProcessorArtifact::text("test.result/1", "text/plain", "ready").unwrap();
    host.complete(
        document,
        ProcessorResult::success(first.key().clone(), artifact),
    )
    .unwrap();
    let ready = host.take_changes();
    assert_eq!(ready[0].checked_byte_len(), Some(MAX_READY_CHANGE_BYTES));

    let second = host
        .begin(
            document,
            descriptor,
            NodeId::new(41),
            maximal_configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let replacement = host.take_changes();
    assert_eq!(replacement.len(), 2);
    assert_eq!(
        replacement[0].checked_byte_len(),
        Some(MAX_REMOVED_CHANGE_BYTES)
    );
    assert_eq!(
        replacement[1].checked_byte_len(),
        Some(MAX_PENDING_CHANGE_BYTES)
    );

    host.complete(
        document,
        ProcessorResult::failure(
            second.key().clone(),
            ProcessorFailure::new(ProcessorFailureCode::Processor, "failed"),
        ),
    )
    .unwrap();
    let failed = host.take_changes();
    assert_eq!(failed[0].checked_byte_len(), Some(MAX_FAILED_CHANGE_BYTES));
}

#[test]
fn ready_change_bytes_accept_exact_and_reject_minus_one_atomically() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let limit = MAX_PENDING_CHANGE_BYTES + MAX_READY_CHANGE_BYTES;

    let mut exact = host_with_pending_byte_limit(limit);
    exact.begin_epoch(Epoch::new(7)).unwrap();
    let exact_request = begin_maximal_request(&mut exact, document);
    let artifact = ProcessorArtifact::text("test.result/1", "text/plain", "ready").unwrap();
    assert_eq!(
        exact
            .complete(
                document,
                ProcessorResult::success(exact_request.key().clone(), artifact.clone()),
            )
            .unwrap(),
        CompletionOutcome::Applied
    );
    assert_eq!(exact.metrics().pending_changes, 2);
    assert_eq!(exact.metrics().pending_change_bytes, limit);
    assert_eq!(exact.take_changes().len(), 2);

    let mut rejected = host_with_pending_byte_limit(limit - 1);
    rejected.begin_epoch(Epoch::new(7)).unwrap();
    let rejected_request = begin_maximal_request(&mut rejected, document);
    let state_before = rejected
        .state(rejected_request.key().slot())
        .unwrap()
        .clone();
    let metrics_before = rejected.metrics();
    let error = rejected
        .complete(
            document,
            ProcessorResult::success(rejected_request.key().clone(), artifact),
        )
        .unwrap_err();
    assert!(matches!(
        error.error(),
        mdstream_processors::HostError::LimitExceeded {
            field: "processor.pending_change_bytes",
            limit: seen_limit,
            actual,
        } if *seen_limit == limit - 1 && *actual == limit
    ));
    assert_eq!(
        rejected.state(rejected_request.key().slot()),
        Some(&state_before)
    );
    assert_eq!(rejected.metrics(), metrics_before);
    assert!(!rejected_request.is_cancelled());

    let retry = error.into_result();
    assert_eq!(rejected.take_changes().len(), 1);
    assert_eq!(
        rejected.complete(document, retry).unwrap(),
        CompletionOutcome::Applied
    );
    let changes = rejected.take_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].checked_byte_len(), Some(MAX_READY_CHANGE_BYTES));
}

#[test]
fn failed_change_bytes_accept_exact_and_reject_minus_one_atomically() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let limit = MAX_PENDING_CHANGE_BYTES + MAX_FAILED_CHANGE_BYTES;
    let failure = ProcessorFailure::new(ProcessorFailureCode::Processor, "failed");

    let mut exact = host_with_pending_byte_limit(limit);
    exact.begin_epoch(Epoch::new(7)).unwrap();
    let exact_request = begin_maximal_request(&mut exact, document);
    assert_eq!(
        exact
            .complete(
                document,
                ProcessorResult::failure(exact_request.key().clone(), failure.clone()),
            )
            .unwrap(),
        CompletionOutcome::Applied
    );
    assert_eq!(exact.metrics().pending_changes, 2);
    assert_eq!(exact.metrics().pending_change_bytes, limit);
    assert_eq!(exact.take_changes().len(), 2);

    let mut rejected = host_with_pending_byte_limit(limit - 1);
    rejected.begin_epoch(Epoch::new(7)).unwrap();
    let rejected_request = begin_maximal_request(&mut rejected, document);
    let state_before = rejected
        .state(rejected_request.key().slot())
        .unwrap()
        .clone();
    let metrics_before = rejected.metrics();
    let error = rejected
        .complete(
            document,
            ProcessorResult::failure(rejected_request.key().clone(), failure),
        )
        .unwrap_err();
    assert!(matches!(
        error.error(),
        mdstream_processors::HostError::LimitExceeded {
            field: "processor.pending_change_bytes",
            limit: seen_limit,
            actual,
        } if *seen_limit == limit - 1 && *actual == limit
    ));
    assert_eq!(
        rejected.state(rejected_request.key().slot()),
        Some(&state_before)
    );
    assert_eq!(rejected.metrics(), metrics_before);
    assert!(!rejected_request.is_cancelled());

    let retry = error.into_result();
    assert_eq!(rejected.take_changes().len(), 1);
    assert_eq!(
        rejected.complete(document, retry).unwrap(),
        CompletionOutcome::Applied
    );
    let changes = rejected.take_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].checked_byte_len(), Some(MAX_FAILED_CHANGE_BYTES));
    assert!(matches!(
        rejected.state(rejected_request.key().slot()),
        Some(ProcessorSlotState::Failed { .. })
    ));
}

#[test]
fn removed_change_bytes_accept_exact_and_reject_minus_one_atomically() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let limit = MAX_PENDING_CHANGE_BYTES + MAX_REMOVED_CHANGE_BYTES;

    let mut exact = host_with_pending_byte_limit(limit);
    exact.begin_epoch(Epoch::new(7)).unwrap();
    let exact_request = begin_maximal_request(&mut exact, document);
    assert!(exact.cancel(exact_request.key()).unwrap());
    assert!(exact_request.is_cancelled());
    assert!(exact.state(exact_request.key().slot()).is_none());
    assert_eq!(exact.metrics().pending_changes, 2);
    assert_eq!(exact.metrics().pending_change_bytes, limit);
    assert_eq!(exact.take_changes().len(), 2);

    let mut rejected = host_with_pending_byte_limit(limit - 1);
    rejected.begin_epoch(Epoch::new(7)).unwrap();
    let rejected_request = begin_maximal_request(&mut rejected, document);
    let state_before = rejected
        .state(rejected_request.key().slot())
        .unwrap()
        .clone();
    let metrics_before = rejected.metrics();
    assert!(matches!(
        rejected.cancel(rejected_request.key()),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.pending_change_bytes",
            limit: seen_limit,
            actual,
        }) if seen_limit == limit - 1 && actual == limit
    ));
    assert_eq!(
        rejected.state(rejected_request.key().slot()),
        Some(&state_before)
    );
    assert_eq!(rejected.metrics(), metrics_before);
    assert!(!rejected_request.is_cancelled());

    assert_eq!(rejected.take_changes().len(), 1);
    assert!(rejected.cancel(rejected_request.key()).unwrap());
    assert!(rejected_request.is_cancelled());
    assert!(rejected.state(rejected_request.key().slot()).is_none());
    let changes = rejected.take_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(
        changes[0].checked_byte_len(),
        Some(MAX_REMOVED_CHANGE_BYTES)
    );
}

#[test]
fn aggregate_artifact_budget_fails_without_evicting_existing_state_and_can_retry() {
    let reducer_one = document(7, 41, "one");
    let reducer_two = document(7, 42, "two");
    let document_one = reducer_one.document().unwrap();
    let document_two = reducer_two.document().unwrap();
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_retained_artifacts: 1,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let first = host
        .begin(
            document_one,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document_one, run_catching(&processor, &first))
        .unwrap();
    let first_artifact = host.artifact(first.key().slot()).unwrap().clone();

    let second = host
        .begin(
            document_two,
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(
        host.complete(document_two, run_catching(&processor, &second))
            .unwrap(),
        CompletionOutcome::Applied
    );
    assert_eq!(host.artifact(first.key().slot()), Some(&first_artifact));
    assert!(host.artifact(second.key().slot()).is_none());
    match host.state(second.key().slot()) {
        Some(ProcessorSlotState::Failed { failure, .. }) => {
            assert_eq!(failure.code(), ProcessorFailureCode::ResourceLimit)
        }
        state => panic!("expected aggregate artifact failure, got {state:?}"),
    }
    assert_eq!(host.metrics().retained_artifacts, 1);

    host.remove_node(Epoch::new(7), NodeId::new(41)).unwrap();
    let retry = host
        .begin(
            document_two,
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document_two, run_catching(&processor, &retry))
        .unwrap();
    assert!(host.artifact(retry.key().slot()).is_some());
    assert_eq!(host.metrics().retained_artifacts, 1);
}

#[test]
fn input_limit_accepts_exact_bytes_and_rejects_plus_one_atomically() {
    let reducer_exact = document(7, 41, "12345");
    let reducer_too_large = document(7, 41, "123456");
    let document_exact = reducer_exact.document().unwrap();
    let document_too_large = reducer_too_large.document().unwrap();
    let exact_bytes = measured_input_bytes(document_exact, NodeId::new(41));
    let too_large_bytes = measured_input_bytes(document_too_large, NodeId::new(41));
    assert_eq!(too_large_bytes, exact_bytes + 1);
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_input_bytes: exact_bytes,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let first = host
        .begin(
            document_exact,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(first.input().byte_len(), exact_bytes);
    host.complete(document_exact, run_catching(&processor, &first))
        .unwrap();
    host.take_changes();
    let before_state = host.state(first.key().slot()).unwrap().clone();
    let before_metrics = host.metrics();

    assert!(matches!(
        host.begin(
            document_too_large,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        ),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.input_bytes",
            limit,
            actual,
        }) if limit == exact_bytes && actual == too_large_bytes
    ));
    assert_eq!(host.state(first.key().slot()), Some(&before_state));
    assert_eq!(host.metrics(), before_metrics);
    assert!(host.take_changes().is_empty());

    let retry = host
        .begin(
            document_exact,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(
        retry.key().generation().get(),
        first.key().generation().get() + 1
    );
}

#[test]
fn input_cost_includes_owned_node_metadata_when_body_is_empty() {
    let small = document_with_info(7, 41, "", "x");
    let large_info = "x".repeat(4096);
    let large = document_with_info(7, 41, "", &large_info);
    let node_id = NodeId::new(41);
    let small_bytes = measured_input_bytes(small.document().unwrap(), node_id);
    let large_bytes = measured_input_bytes(large.document().unwrap(), node_id);

    assert!(large_bytes >= small_bytes + large_info.len() - 1);
    assert!(large_bytes > 0);

    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut exact = ArtifactHost::new(ProcessorLimits {
        max_input_bytes: large_bytes,
        ..ProcessorLimits::default()
    })
    .unwrap();
    exact.begin_epoch(Epoch::new(7)).unwrap();
    exact
        .begin(
            large.document().unwrap(),
            processor.descriptor().clone(),
            node_id,
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();

    let mut plus_one = ArtifactHost::new(ProcessorLimits {
        max_input_bytes: large_bytes - 1,
        ..ProcessorLimits::default()
    })
    .unwrap();
    plus_one.begin_epoch(Epoch::new(7)).unwrap();
    assert!(matches!(
        plus_one.begin(
            large.document().unwrap(),
            processor.descriptor().clone(),
            node_id,
            configuration,
            ProcessingPolicy::StableOnly,
        ),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.input_bytes",
            limit,
            actual,
        }) if limit + 1 == large_bytes && actual == large_bytes
    ));
}

#[test]
fn artifact_limit_accepts_exact_bytes_and_rejects_plus_one_before_retention() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let exact_processor = FixedProcessor::new("abc");
    let too_large_processor = FixedProcessor::new("abcd");
    let exact_artifact =
        ProcessorArtifact::text("test.fixed.result/1", "text/plain", "abc").unwrap();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_artifact_bytes: exact_artifact.byte_len(),
        max_error_bytes: 0,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let exact = host
        .begin(
            document,
            exact_processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document, run_catching(&exact_processor, &exact))
        .unwrap();
    assert_eq!(host.artifact(exact.key().slot()), Some(&exact_artifact));

    let too_large = host
        .begin(
            document,
            too_large_processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document, run_catching(&too_large_processor, &too_large))
        .unwrap();
    assert!(host.artifact(too_large.key().slot()).is_none());
    match host.state(too_large.key().slot()) {
        Some(ProcessorSlotState::Failed { failure, .. }) => {
            assert_eq!(failure.code(), ProcessorFailureCode::ResourceLimit);
            assert!(failure.message().is_empty());
        }
        state => panic!("expected artifact limit failure, got {state:?}"),
    }
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(host.metrics().retained_artifacts, 0);
    assert_eq!(host.metrics().retained_artifact_bytes, 0);
}

#[test]
fn stale_oversized_result_checks_freshness_before_artifact_limits() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let current_processor = FixedProcessor::new("abc");
    let stale_processor = FixedProcessor::new("abcd");
    let current_artifact =
        ProcessorArtifact::text("test.fixed.result/1", "text/plain", "abc").unwrap();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_artifact_bytes: current_artifact.byte_len(),
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let stale = host
        .begin(
            document,
            stale_processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let stale_result = run_catching(&stale_processor, &stale);
    let current = host
        .begin(
            document,
            current_processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document, run_catching(&current_processor, &current))
        .unwrap();
    let retained_bytes = host.metrics().retained_artifact_bytes;
    assert_eq!(host.metrics().in_flight_jobs, 1);

    assert_eq!(
        host.complete(document, stale_result).unwrap(),
        CompletionOutcome::Stale
    );
    assert_eq!(host.artifact(current.key().slot()), Some(&current_artifact));
    assert_eq!(host.metrics().retained_artifacts, 1);
    assert_eq!(host.metrics().retained_artifact_bytes, retained_bytes);
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(host.metrics().stale_results, 1);
}

#[test]
fn slot_limit_counts_retained_derived_state_and_rejects_new_slots_atomically() {
    let reducer_one = document(7, 41, "one");
    let reducer_two = document(7, 42, "two");
    let document_one = reducer_one.document().unwrap();
    let document_two = reducer_two.document().unwrap();
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_slots: 1,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let first = host
        .begin(
            document_one,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document_one, run_catching(&processor, &first))
        .unwrap();
    host.take_changes();
    let before_state = host.state(first.key().slot()).unwrap().clone();
    let before_metrics = host.metrics();

    assert!(matches!(
        host.begin(
            document_two,
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        ),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.slots",
            limit: 1,
            actual: 2,
        })
    ));
    assert_eq!(host.state(first.key().slot()), Some(&before_state));
    assert_eq!(host.metrics(), before_metrics);
    assert!(host.take_changes().is_empty());

    let replacement = host
        .begin(
            document_one,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(host.metrics().slots, 1);
    assert_eq!(
        replacement.key().generation().get(),
        first.key().generation().get() + 1
    );
}

#[test]
fn superseded_jobs_keep_in_flight_budget_until_their_lease_is_settled() {
    let reducer = document(7, 41, "123");
    let document = reducer.document().unwrap();
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let input_bytes = measured_input_bytes(document, NodeId::new(41));
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_in_flight_jobs: 2,
        max_in_flight_input_bytes: input_bytes.checked_mul(2).unwrap(),
        ..ProcessorLimits::default()
    })
    .unwrap();
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
    let second = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert!(first.is_cancelled());
    assert!(!second.is_cancelled());
    assert_eq!(host.metrics().in_flight_jobs, 2);
    assert_eq!(
        host.metrics().in_flight_input_bytes,
        input_bytes.checked_mul(2).unwrap()
    );
    host.take_changes();
    let before_state = host.state(second.key().slot()).unwrap().clone();
    let before_metrics = host.metrics();

    assert!(matches!(
        host.begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        ),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.in_flight_jobs",
            limit: 2,
            actual: 3,
        })
    ));
    assert_eq!(host.state(second.key().slot()), Some(&before_state));
    assert_eq!(host.metrics(), before_metrics);
    assert!(!second.is_cancelled());
    assert!(host.take_changes().is_empty());

    assert!(host.cancel(first.key()).unwrap());
    let retry = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert!(second.is_cancelled());
    assert_eq!(host.metrics().in_flight_jobs, 2);
    assert_eq!(
        retry.key().generation().get(),
        second.key().generation().get() + 1
    );
}

#[test]
fn aggregate_in_flight_input_bytes_accept_exact_total_and_reject_next_input() {
    let reducer_two = document(7, 41, "12");
    let reducer_three = document(7, 42, "123");
    let reducer_one = document(7, 43, "1");
    let first_bytes = measured_input_bytes(reducer_two.document().unwrap(), NodeId::new(41));
    let second_bytes = measured_input_bytes(reducer_three.document().unwrap(), NodeId::new(42));
    let third_bytes = measured_input_bytes(reducer_one.document().unwrap(), NodeId::new(43));
    let aggregate_limit = first_bytes.checked_add(second_bytes).unwrap();
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_in_flight_jobs: 3,
        max_in_flight_input_bytes: aggregate_limit,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let first = host
        .begin(
            reducer_two.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.begin(
        reducer_three.document().unwrap(),
        processor.descriptor().clone(),
        NodeId::new(42),
        configuration.clone(),
        ProcessingPolicy::StableOnly,
    )
    .unwrap();
    assert_eq!(host.metrics().in_flight_input_bytes, aggregate_limit);
    let before = host.metrics();

    assert!(matches!(
        host.begin(
            reducer_one.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(43),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        ),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.in_flight_input_bytes",
            limit,
            actual,
        }) if limit == aggregate_limit && actual == aggregate_limit + third_bytes
    ));
    assert_eq!(host.metrics(), before);

    assert!(host.cancel(first.key()).unwrap());
    host.begin(
        reducer_one.document().unwrap(),
        processor.descriptor().clone(),
        NodeId::new(43),
        configuration,
        ProcessingPolicy::StableOnly,
    )
    .unwrap();
    assert_eq!(
        host.metrics().in_flight_input_bytes,
        second_bytes + third_bytes
    );
}

#[test]
fn aggregate_artifact_bytes_accept_exact_total_and_reject_next_artifact() {
    let reducer_one = document(7, 41, "one");
    let reducer_two = document(7, 42, "two");
    let reducer_three = document(7, 43, "three");
    let processor = FixedProcessor::new("artifact");
    let artifact =
        ProcessorArtifact::text("test.fixed.result/1", "text/plain", "artifact").unwrap();
    let aggregate_limit = artifact.byte_len().checked_mul(2).unwrap();
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_retained_artifact_bytes: aggregate_limit,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    for (reducer, node_id) in [(&reducer_one, 41_u128), (&reducer_two, 42_u128)] {
        let request = host
            .begin(
                reducer.document().unwrap(),
                processor.descriptor().clone(),
                NodeId::new(node_id),
                configuration.clone(),
                ProcessingPolicy::StableOnly,
            )
            .unwrap();
        host.complete(
            reducer.document().unwrap(),
            run_catching(&processor, &request),
        )
        .unwrap();
    }
    assert_eq!(host.metrics().retained_artifact_bytes, aggregate_limit);

    let third = host
        .begin(
            reducer_three.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(43),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(
        reducer_three.document().unwrap(),
        run_catching(&processor, &third),
    )
    .unwrap();
    assert!(host.artifact(third.key().slot()).is_none());
    match host.state(third.key().slot()) {
        Some(ProcessorSlotState::Failed { failure, .. }) => {
            assert_eq!(failure.code(), ProcessorFailureCode::ResourceLimit);
            assert!(failure.message().contains("retained_artifact_bytes"));
        }
        state => panic!("expected retained artifact byte failure, got {state:?}"),
    }
    assert_eq!(host.metrics().retained_artifacts, 2);
    assert_eq!(host.metrics().retained_artifact_bytes, aggregate_limit);

    host.remove_node(Epoch::new(7), NodeId::new(41)).unwrap();
    let retry = host
        .begin(
            reducer_three.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(43),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(
        reducer_three.document().unwrap(),
        run_catching(&processor, &retry),
    )
    .unwrap();
    assert!(host.artifact(retry.key().slot()).is_some());
    assert_eq!(host.metrics().retained_artifacts, 2);
    assert_eq!(host.metrics().retained_artifact_bytes, aggregate_limit);
}

#[test]
fn processor_failure_messages_are_bounded_before_derived_state_retention() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let exact = FailingProcessor::new("12345");
    let too_large = FailingProcessor::new("123456");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_error_bytes: 5,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let exact_request = host
        .begin(
            document,
            exact.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document, run_catching(&exact, &exact_request))
        .unwrap();
    match host.state(exact_request.key().slot()) {
        Some(ProcessorSlotState::Failed { failure, .. }) => {
            assert_eq!(failure.code(), ProcessorFailureCode::Processor);
            assert_eq!(failure.message(), "12345");
        }
        state => panic!("expected exact-size processor failure, got {state:?}"),
    }

    let too_large_request = host
        .begin(
            document,
            too_large.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.complete(document, run_catching(&too_large, &too_large_request))
        .unwrap();
    match host.state(too_large_request.key().slot()) {
        Some(ProcessorSlotState::Failed { failure, .. }) => {
            assert_eq!(failure.code(), ProcessorFailureCode::ResourceLimit);
            assert!(failure.message().len() <= 5);
            assert!(!failure.message().contains("123456"));
        }
        state => panic!("expected bounded processor failure, got {state:?}"),
    }
    assert_eq!(host.metrics().in_flight_jobs, 0);
    assert_eq!(host.metrics().slots, 1);
}

#[test]
fn replacement_reserves_two_change_events_before_cancelling_the_current_request() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_slots: 1,
        max_pending_changes: 1,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();

    let current = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(host.take_changes().len(), 1);
    let state_before = host.state(current.key().slot()).unwrap().clone();
    let metrics_before = host.metrics();

    assert!(matches!(
        host.begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        ),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.pending_changes",
            limit: 1,
            actual: 2,
        })
    ));
    assert!(!current.is_cancelled());
    assert_eq!(host.state(current.key().slot()), Some(&state_before));
    assert_eq!(host.metrics(), metrics_before);
    assert!(host.take_changes().is_empty());

    assert!(host.cancel(current.key()).unwrap());
    assert_eq!(host.take_changes().len(), 1);
    let retry = host
        .begin(
            document,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(
        retry.key().generation().get(),
        current.key().generation().get() + 1
    );
}

#[test]
fn completion_change_backpressure_returns_a_retryable_result_without_settling_the_lease() {
    let reducer = document(7, 41, "input");
    let document = reducer.document().unwrap();
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_slots: 1,
        max_pending_changes: 1,
        ..ProcessorLimits::default()
    })
    .unwrap();
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
    let state_before = host.state(request.key().slot()).unwrap().clone();
    let metrics_before = host.metrics();

    let error = host
        .complete(document, run_catching(&processor, &request))
        .unwrap_err();
    assert!(matches!(
        error.error(),
        mdstream_processors::HostError::LimitExceeded {
            field: "processor.pending_changes",
            limit: 1,
            actual: 2,
        }
    ));
    assert_eq!(host.state(request.key().slot()), Some(&state_before));
    assert_eq!(host.metrics(), metrics_before);
    assert!(!request.is_cancelled());

    let retry = error.into_result();
    assert_eq!(host.take_changes().len(), 1);
    assert_eq!(
        host.complete(document, retry).unwrap(),
        CompletionOutcome::Applied
    );
    assert!(host.artifact(request.key().slot()).is_some());
    assert_eq!(host.metrics().in_flight_jobs, 0);
}

#[test]
fn multi_node_reconcile_reserves_the_whole_change_batch_before_mutation() {
    let reducer_one = document(7, 41, "one");
    let reducer_two = document(7, 42, "two");
    let document_one = reducer_one.document().unwrap();
    let document_two = reducer_two.document().unwrap();
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_slots: 2,
        max_pending_changes: 2,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let first = host
        .begin(
            document_one,
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.take_changes();
    let second = host
        .begin(
            document_two,
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let first_state = host.state(first.key().slot()).unwrap().clone();
    let second_state = host.state(second.key().slot()).unwrap().clone();
    let metrics_before = host.metrics();
    let impact = ChangeImpact {
        removed_nodes: vec![NodeId::new(41), NodeId::new(42)],
        changed_nodes: vec![NodeId::new(41), NodeId::new(42)],
        projection_changed: true,
        ..ChangeImpact::default()
    };

    assert!(matches!(
        host.reconcile(document_one, &impact),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.pending_changes",
            limit: 2,
            actual: 3,
        })
    ));
    assert!(!first.is_cancelled());
    assert!(!second.is_cancelled());
    assert_eq!(host.state(first.key().slot()), Some(&first_state));
    assert_eq!(host.state(second.key().slot()), Some(&second_state));
    assert_eq!(host.metrics(), metrics_before);
    assert_eq!(host.take_changes().len(), 1);

    host.reconcile(document_one, &impact).unwrap();
    assert!(first.is_cancelled());
    assert!(second.is_cancelled());
    assert!(host.state(first.key().slot()).is_none());
    assert!(host.state(second.key().slot()).is_none());
    let changes = host.take_changes();
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().all(|change| matches!(
        change.kind(),
        ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::NodeRemoved,
            ..
        }
    )));
}

#[test]
fn epoch_reset_backpressure_is_atomic_and_retryable_after_drain() {
    let reducer_one = document(7, 41, "one");
    let reducer_two = document(7, 42, "two");
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_slots: 2,
        max_pending_changes: 2,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    let first = host
        .begin(
            reducer_one.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(41),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    host.take_changes();
    let second = host
        .begin(
            reducer_two.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(42),
            configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let first_state = host.state(first.key().slot()).unwrap().clone();
    let second_state = host.state(second.key().slot()).unwrap().clone();
    let metrics_before = host.metrics();

    assert!(matches!(
        host.begin_epoch(Epoch::new(8)),
        Err(mdstream_processors::HostError::LimitExceeded {
            field: "processor.pending_changes",
            limit: 2,
            actual: 3,
        })
    ));
    assert_eq!(host.state(first.key().slot()), Some(&first_state));
    assert_eq!(host.state(second.key().slot()), Some(&second_state));
    assert_eq!(host.metrics(), metrics_before);
    assert!(!first.is_cancelled());
    assert!(!second.is_cancelled());

    assert_eq!(host.take_changes().len(), 1);
    host.begin_epoch(Epoch::new(8)).unwrap();
    assert!(first.is_cancelled());
    assert!(second.is_cancelled());
    assert!(host.state(first.key().slot()).is_none());
    assert!(host.state(second.key().slot()).is_none());
    let changes = host.take_changes();
    assert_eq!(changes.len(), 2);
    assert!(changes.iter().all(|change| matches!(
        change.kind(),
        ArtifactChangeKind::Removed {
            reason: ArtifactReleaseReason::EpochReset,
            ..
        }
    )));
    assert!(matches!(
        host.begin_epoch(Epoch::new(7)),
        Err(mdstream_processors::HostError::EpochRegression {
            current,
            requested,
        }) if current == Epoch::new(8) && requested == Epoch::new(7)
    ));
}

fn issued_request_store_visits(request_count: u128) -> u64 {
    let processor = FixedProcessor::new("artifact");
    let configuration = ConfigurationVersion::new("config.v1").unwrap();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_slots: usize::try_from(request_count).unwrap(),
        max_in_flight_jobs: usize::try_from(request_count).unwrap(),
        max_in_flight_input_bytes: usize::MAX,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(Epoch::new(7)).unwrap();
    for node_id in 1..=request_count {
        let reducer = document(7, node_id, "x");
        host.begin(
            reducer.document().unwrap(),
            processor.descriptor().clone(),
            NodeId::new(node_id),
            configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
        host.take_changes();
    }
    host.metrics().store_entry_visits
}

#[test]
fn issued_request_store_work_is_near_linear() {
    let small = issued_request_store_visits(32);
    let large = issued_request_store_visits(64);
    assert!(
        large.saturating_mul(100) <= small.saturating_mul(225),
        "doubling requests grew store visits from {small} to {large}"
    );
}
