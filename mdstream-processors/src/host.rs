use mdstream_protocol::{ChangeImpact, Document, Epoch, NodeId, RequestGeneration};

use crate::{
    ArtifactChange, ArtifactChangeKind, ArtifactReleaseReason, CompletionError, CompletionOutcome,
    ConfigurationVersion, HostError, ProcessingPolicy, ProcessorArtifact, ProcessorDescriptor,
    ProcessorFailure, ProcessorFailureCode, ProcessorLimits, ProcessorLimitsError,
    ProcessorMetrics, ProcessorRequest, ProcessorRequestKey, ProcessorResult, ProcessorSlotKey,
    ProcessorSlotState,
    limits::check_limit,
    request::{CancellationToken, ProcessorInput, provisional_allowed},
    store::ArtifactStore,
};

#[cfg(test)]
use crate::request::BeginRequest;

/// Owns processor request freshness, cancellation, derived state, and budgets.
///
/// Processor execution is deliberately separate; every mutating operation is
/// a short synchronous transaction over host-owned state.
pub struct ArtifactHost {
    limits: ProcessorLimits,
    epoch: Option<Epoch>,
    next_generation: u64,
    store: ArtifactStore,
}

impl ArtifactHost {
    /// Creates a host after validating its cleanup queue limits.
    ///
    /// # Errors
    ///
    /// Returns [`ProcessorLimitsError`] when `limits` cannot guarantee that
    /// cleanup changes fit in the pending change queue.
    pub fn new(limits: ProcessorLimits) -> Result<Self, ProcessorLimitsError> {
        limits.validate()?;
        Ok(Self {
            limits,
            epoch: None,
            next_generation: 1,
            store: ArtifactStore::new(limits),
        })
    }

    pub fn begin_epoch(&mut self, epoch: Epoch) -> Result<(), HostError> {
        match self.epoch {
            Some(current) if epoch < current => {
                return Err(HostError::EpochRegression {
                    current,
                    requested: epoch,
                });
            }
            Some(current) if epoch == current => return Ok(()),
            Some(_) => self.store.clear(ArtifactReleaseReason::EpochReset)?,
            None => {}
        }
        self.epoch = Some(epoch);
        Ok(())
    }

    pub fn begin(
        &mut self,
        document: &Document,
        descriptor: ProcessorDescriptor,
        node_id: NodeId,
        configuration: ConfigurationVersion,
        policy: ProcessingPolicy,
    ) -> Result<ProcessorRequest, HostError> {
        let document_epoch = document.coordinate().epoch;
        let current = self.epoch.ok_or(HostError::EpochNotInitialized)?;
        if current != document_epoch {
            return Err(HostError::EpochMismatch {
                current,
                received: document_epoch,
            });
        }
        let input = ProcessorInput::view_document(document, node_id)?;
        if !provisional_allowed(input.node().stability, descriptor.capabilities(), policy) {
            return Err(HostError::ProvisionalProcessingDisabled(node_id));
        }
        let slot = ProcessorSlotKey::new(document_epoch, input.node().id, descriptor.id().clone());
        self.check_begin_limits(input.byte_len(), &slot)?;
        let generation = RequestGeneration::new(self.next_generation);
        let next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(HostError::RequestGenerationExhausted)?;
        let key = ProcessorRequestKey::new(
            slot,
            input.node().version.clone(),
            input.version().clone(),
            descriptor.version().clone(),
            configuration,
            generation,
        );
        let reservation = self.store.preflight_pending(&key, input.byte_len())?;
        let input = input.materialize();
        let cancellation = CancellationToken::default();
        let request = ProcessorRequest::new(key.clone(), input, cancellation.clone());
        self.store
            .commit_pending(reservation, key, request.input().byte_len(), cancellation);
        self.next_generation = next_generation;
        Ok(request)
    }

    #[cfg(test)]
    fn begin_prepared(
        &mut self,
        epoch: Epoch,
        begin: BeginRequest,
    ) -> Result<ProcessorRequest, HostError> {
        let slot =
            ProcessorSlotKey::new(epoch, begin.input.node().id, begin.descriptor.id().clone());
        self.check_begin_limits(begin.input.byte_len(), &slot)?;
        let generation = RequestGeneration::new(self.next_generation);
        let next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or(HostError::RequestGenerationExhausted)?;
        let key = ProcessorRequestKey::new(
            slot,
            begin.input.node().version.clone(),
            begin.input.version().clone(),
            begin.descriptor.version().clone(),
            begin.configuration,
            generation,
        );
        let cancellation = CancellationToken::default();
        let request = ProcessorRequest::new(key.clone(), begin.input, cancellation.clone());
        self.store
            .install_pending(key, request.input().byte_len(), cancellation)?;
        self.next_generation = next_generation;
        Ok(request)
    }

    fn check_begin_limits(
        &self,
        input_bytes: usize,
        slot: &ProcessorSlotKey,
    ) -> Result<(), HostError> {
        check_limit(
            "processor.input_bytes",
            self.limits.max_input_bytes,
            input_bytes,
        )?;
        let metrics = self.store.metrics();
        let slots = metrics
            .slots
            .checked_add(usize::from(!self.store.contains_slot(slot)))
            .ok_or(HostError::CounterOverflow("processor.slots"))?;
        check_limit("processor.slots", self.limits.max_slots, slots)?;
        let in_flight_jobs = metrics
            .in_flight_jobs
            .checked_add(1)
            .ok_or(HostError::CounterOverflow("processor.in_flight_jobs"))?;
        check_limit(
            "processor.in_flight_jobs",
            self.limits.max_in_flight_jobs,
            in_flight_jobs,
        )?;
        let in_flight_input_bytes = metrics
            .in_flight_input_bytes
            .checked_add(input_bytes)
            .ok_or(HostError::CounterOverflow(
                "processor.in_flight_input_bytes",
            ))?;
        check_limit(
            "processor.in_flight_input_bytes",
            self.limits.max_in_flight_input_bytes,
            in_flight_input_bytes,
        )
    }

    fn completion_plan(
        &self,
        outcome: &Result<ProcessorArtifact, ProcessorFailure>,
    ) -> CompletionPlan {
        match outcome {
            Ok(artifact) => {
                let Some(artifact_bytes) = artifact.checked_byte_len() else {
                    return CompletionPlan::ReplacementFailure(resource_limit_failure(
                        "processor.artifact_bytes",
                        self.limits.max_artifact_bytes,
                        usize::MAX,
                        self.limits.max_error_bytes,
                    ));
                };
                let metrics = self.store.metrics();
                let retained_artifacts = metrics.retained_artifacts.checked_add(1);
                let retained_artifact_bytes =
                    metrics.retained_artifact_bytes.checked_add(artifact_bytes);
                let violation = if artifact_bytes > self.limits.max_artifact_bytes {
                    Some((
                        "processor.artifact_bytes",
                        self.limits.max_artifact_bytes,
                        artifact_bytes,
                    ))
                } else if retained_artifacts
                    .is_none_or(|actual| actual > self.limits.max_retained_artifacts)
                {
                    Some((
                        "processor.retained_artifacts",
                        self.limits.max_retained_artifacts,
                        retained_artifacts.unwrap_or(usize::MAX),
                    ))
                } else if retained_artifact_bytes
                    .is_none_or(|actual| actual > self.limits.max_retained_artifact_bytes)
                {
                    Some((
                        "processor.retained_artifact_bytes",
                        self.limits.max_retained_artifact_bytes,
                        retained_artifact_bytes.unwrap_or(usize::MAX),
                    ))
                } else {
                    None
                };
                if let Some((field, limit, actual)) = violation {
                    CompletionPlan::ReplacementFailure(resource_limit_failure(
                        field,
                        limit,
                        actual,
                        self.limits.max_error_bytes,
                    ))
                } else {
                    CompletionPlan::Ready { artifact_bytes }
                }
            }
            Err(failure) if failure.message().len() > self.limits.max_error_bytes => {
                CompletionPlan::ReplacementFailure(resource_limit_failure(
                    "processor.error_bytes",
                    self.limits.max_error_bytes,
                    failure.message().len(),
                    self.limits.max_error_bytes,
                ))
            }
            Err(failure) => CompletionPlan::OriginalFailure {
                code: failure.code(),
            },
        }
    }

    pub fn complete(
        &mut self,
        document: &Document,
        result: ProcessorResult,
    ) -> Result<CompletionOutcome, CompletionError> {
        let (key, outcome) = result.into_parts();
        if !self.store.has_lease(&key) {
            if let Err(error) = self.store.record_stale_result() {
                return Err(CompletionError::new(
                    error,
                    ProcessorResult::from_parts(key, outcome),
                ));
            }
            return Ok(CompletionOutcome::Stale);
        }
        if !self.store.current_pending(&key) {
            if let Err(error) = self.store.settle_stale(&key) {
                return Err(CompletionError::new(
                    error,
                    ProcessorResult::from_parts(key, outcome),
                ));
            }
            return Ok(CompletionOutcome::Stale);
        }
        match request_document_state(document, &key) {
            RequestDocumentState::Matching => {}
            RequestDocumentState::NodeRemoved => {
                if let Err(error) = self
                    .store
                    .remove_slot_for_stale_result(key.slot(), ArtifactReleaseReason::NodeRemoved)
                {
                    return Err(CompletionError::new(
                        error,
                        ProcessorResult::from_parts(key, outcome),
                    ));
                }
                return Ok(CompletionOutcome::Stale);
            }
            RequestDocumentState::NodeChanged => {
                if let Err(error) = self
                    .store
                    .remove_slot_for_stale_result(key.slot(), ArtifactReleaseReason::NodeChanged)
                {
                    return Err(CompletionError::new(
                        error,
                        ProcessorResult::from_parts(key, outcome),
                    ));
                }
                return Ok(CompletionOutcome::Stale);
            }
            RequestDocumentState::EpochMismatch(received) => {
                return Err(CompletionError::new(
                    HostError::EpochMismatch {
                        current: key.slot().epoch(),
                        received,
                    },
                    ProcessorResult::from_parts(key, outcome),
                ));
            }
        }

        let plan = self.completion_plan(&outcome);
        let reservation = match self.store.preflight_completion(
            &key,
            plan.change_kind(),
            plan.retained_artifact_bytes(),
        ) {
            Ok(reservation) => reservation,
            Err(error) => {
                return Err(CompletionError::new(
                    error,
                    ProcessorResult::from_parts(key, outcome),
                ));
            }
        };
        match (outcome, plan) {
            (Ok(artifact), CompletionPlan::Ready { .. }) => {
                self.store.commit_artifact(reservation, key, artifact);
            }
            (_, CompletionPlan::ReplacementFailure(failure))
            | (Err(failure), CompletionPlan::OriginalFailure { .. }) => {
                self.store.commit_failure(reservation, key, failure);
            }
            (Ok(_), CompletionPlan::OriginalFailure { .. })
            | (Err(_), CompletionPlan::Ready { .. }) => {
                unreachable!("completion plan is derived from the processor outcome")
            }
        }
        Ok(CompletionOutcome::Applied)
    }

    pub fn remove_node(&mut self, epoch: Epoch, node_id: NodeId) -> Result<(), HostError> {
        let current = self.epoch.ok_or(HostError::EpochNotInitialized)?;
        if current != epoch {
            return Err(HostError::EpochMismatch {
                current,
                received: epoch,
            });
        }
        self.store
            .remove_node(epoch, node_id, ArtifactReleaseReason::NodeRemoved)?;
        Ok(())
    }

    pub fn reconcile(
        &mut self,
        document: &Document,
        impact: &ChangeImpact,
    ) -> Result<(), HostError> {
        let document_epoch = document.coordinate().epoch;
        match self.epoch {
            Some(current) if current == document_epoch => {}
            Some(current) if document_epoch < current => {
                return Err(HostError::EpochRegression {
                    current,
                    requested: document_epoch,
                });
            }
            Some(_) | None => self.begin_epoch(document_epoch)?,
        }
        if impact.full_replace {
            self.store.clear(ArtifactReleaseReason::NodeChanged)?;
            return Ok(());
        }
        self.store
            .remove_nodes(document_epoch, &impact.removed_nodes, &impact.changed_nodes)?;
        Ok(())
    }

    pub fn cancel(&mut self, key: &ProcessorRequestKey) -> Result<bool, HostError> {
        self.store.cancel(key)
    }

    pub fn take_changes(&mut self) -> Vec<ArtifactChange> {
        self.store.take_changes()
    }

    pub fn state(&self, slot: &ProcessorSlotKey) -> Option<&ProcessorSlotState> {
        self.store.state(slot)
    }

    pub fn artifact(&self, slot: &ProcessorSlotKey) -> Option<&crate::ProcessorArtifact> {
        self.store.artifact(slot)
    }

    pub fn metrics(&self) -> ProcessorMetrics {
        self.store.metrics()
    }
}

enum CompletionPlan {
    Ready { artifact_bytes: usize },
    ReplacementFailure(ProcessorFailure),
    OriginalFailure { code: ProcessorFailureCode },
}

impl CompletionPlan {
    fn change_kind(&self) -> ArtifactChangeKind {
        match self {
            Self::Ready { artifact_bytes } => ArtifactChangeKind::Ready {
                artifact_bytes: *artifact_bytes,
            },
            Self::ReplacementFailure(failure) => ArtifactChangeKind::Failed {
                code: failure.code(),
            },
            Self::OriginalFailure { code } => ArtifactChangeKind::Failed { code: *code },
        }
    }

    const fn retained_artifact_bytes(&self) -> Option<usize> {
        match self {
            Self::Ready { artifact_bytes } => Some(*artifact_bytes),
            Self::ReplacementFailure(_) | Self::OriginalFailure { .. } => None,
        }
    }
}

enum RequestDocumentState {
    Matching,
    NodeRemoved,
    NodeChanged,
    EpochMismatch(Epoch),
}

fn request_document_state(document: &Document, key: &ProcessorRequestKey) -> RequestDocumentState {
    if document.coordinate().epoch != key.slot().epoch() {
        return RequestDocumentState::EpochMismatch(document.coordinate().epoch);
    }
    let Some(node) = document.node(key.slot().node_id()) else {
        return RequestDocumentState::NodeRemoved;
    };
    if node.version != *key.node_version() {
        return RequestDocumentState::NodeChanged;
    }
    if ProcessorInput::version_from_document(document, key.slot().node_id())
        .is_ok_and(|version| &version == key.input_version())
    {
        RequestDocumentState::Matching
    } else {
        RequestDocumentState::NodeChanged
    }
}

fn resource_limit_failure(
    field: &'static str,
    limit: usize,
    actual: usize,
    max_error_bytes: usize,
) -> ProcessorFailure {
    ProcessorFailure::new(
        ProcessorFailureCode::ResourceLimit,
        truncate_message(
            HostError::LimitExceeded {
                field,
                limit,
                actual,
            }
            .to_string(),
            max_error_bytes,
        ),
    )
}

fn truncate_message(mut message: String, max_bytes: usize) -> String {
    if message.len() <= max_bytes {
        return message;
    }
    let mut boundary = max_bytes;
    while !message.is_char_boundary(boundary) {
        boundary -= 1;
    }
    message.truncate(boundary);
    message
}

#[cfg(test)]
mod tests {
    use mdstream_protocol::{
        CodeBlockSyntax, CodeFenceMarker, ContentKind, ContentNode, NodeStability, SourceCursor,
        SourceRange,
    };

    use super::*;
    use crate::{ProcessorArtifact, ProcessorCapabilities};

    #[test]
    fn request_generation_exhaustion_preserves_host_state() {
        let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
        host.epoch = Some(Epoch::new(7));
        let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(0));
        let begin = || BeginRequest {
            descriptor: ProcessorDescriptor::new(
                "test.echo",
                "v1",
                ProcessorCapabilities::stable_only(),
            )
            .unwrap(),
            configuration: ConfigurationVersion::new("config.v1").unwrap(),
            input: ProcessorInput::from_parts(
                ContentNode::leaf(
                    NodeId::new(41),
                    NodeStability::Stable,
                    range,
                    ContentKind::CodeBlock {
                        syntax: CodeBlockSyntax::Fenced {
                            marker: CodeFenceMarker::Backtick,
                            length: 3,
                        },
                        info: None,
                        text: mdstream_protocol::SemanticText::Source {},
                    },
                ),
                "",
                None,
            )
            .unwrap(),
        };
        let current = host.begin_prepared(Epoch::new(7), begin()).unwrap();
        let artifact =
            ProcessorArtifact::text("test.echo.result/1", "text/plain", "ready").unwrap();
        let reservation = host
            .store
            .preflight_completion(
                current.key(),
                ArtifactChangeKind::Ready {
                    artifact_bytes: artifact.byte_len(),
                },
                Some(artifact.byte_len()),
            )
            .unwrap();
        host.store
            .commit_artifact(reservation, current.key().clone(), artifact);
        host.take_changes();
        host.next_generation = u64::MAX;
        let before_state = host.state(current.key().slot()).unwrap().clone();
        let before = host.metrics();

        assert!(matches!(
            host.begin_prepared(Epoch::new(7), begin()),
            Err(HostError::RequestGenerationExhausted)
        ));
        assert_eq!(host.state(current.key().slot()), Some(&before_state));
        assert!(!current.is_cancelled());
        assert_eq!(host.metrics(), before);
        assert_eq!(host.next_generation, u64::MAX);
        assert!(host.take_changes().is_empty());
    }

    #[test]
    fn retained_failure_messages_respect_utf8_and_zero_byte_limits() {
        assert_eq!(truncate_message("éé".to_string(), 3), "é");
        assert_eq!(truncate_message("message".to_string(), 0), "");
    }
}
