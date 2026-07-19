use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use mdstream::{EngineOutput, StreamEngine};
use mdstream_processors::{
    ArtifactChange, ArtifactChangeKind, ArtifactHost, ArtifactReleaseReason, CompletionOutcome,
    ConfigurationVersion, ProcessingPolicy, ProcessorArtifact, ProcessorCapabilities,
    ProcessorDescriptor, ProcessorExpectation, ProcessorFailure, ProcessorFailureCode, ProcessorId,
    ProcessorRequest, ProcessorRequestKey, ProcessorResult, ProcessorSlotKey,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeImpact, ChangeSet, Document, NodeId, NodeVersion, ProtocolLimits, Reducer,
    ReducerStatus, RequestGeneration, ResourceId, Snapshot, TransitionError, TransitionOutcome,
    TransitionReducer, decode_change_json, decode_snapshot_json, encode_change_json,
    encode_snapshot_json,
};

use crate::{
    BindingError, BindingMetrics, BindingOutput, BindingPayloadKind, BindingStatus,
    commands::{
        EngineCommand, ProcessorCompletion, ReducerCommand, decode_engine_command,
        decode_processor_completion, decode_reducer_command, parse_decimal_id, processing_policy,
    },
    errors::{check_size, engine_error, host_error, identifier_error, protocol_error},
    options::{BindingOptions, WireLimits},
    wire::{
        encode_artifact_change, encode_artifact_view, encode_node_view, encode_pending_source_view,
        encode_processor_completion, encode_processor_request, encode_reducer_update,
        encode_resource_view, push_recorded,
    },
};

#[derive(Debug)]
pub struct EngineSession {
    engine: StreamEngine,
    protocol_limits: ProtocolLimits,
    wire_limits: WireLimits,
    metrics: BindingMetrics,
}

impl EngineSession {
    pub fn new(options_json: &[u8]) -> Result<Self, BindingError> {
        let options = BindingOptions::parse(options_json)?;
        let (engine, protocol_limits, wire_limits) = options.into_engine()?;
        Ok(Self {
            engine,
            protocol_limits,
            wire_limits,
            metrics: BindingMetrics::default(),
        })
    }

    pub fn append(&mut self, chunk: &[u8]) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        check_size(
            "bindings.append_bytes",
            chunk,
            self.wire_limits.max_command_bytes,
        )?;
        let chunk = std::str::from_utf8(chunk).map_err(|error| {
            BindingError::new(
                BindingStatus::Utf8,
                "bindings.invalid_utf8",
                format!("append input is not UTF-8: {error}"),
            )
        })?;
        self.append_text(chunk)
    }

    fn append_text(&mut self, chunk: &str) -> Result<BindingOutput, BindingError> {
        let output = self.engine.append(chunk).map_err(engine_error)?;
        self.encode_engine_output(output)
    }

    pub fn finish(&mut self) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let output = self.engine.finish().map_err(engine_error)?;
        self.encode_engine_output(output)
    }

    pub fn reset(&mut self) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let output = self.engine.reset().map_err(engine_error)?;
        self.encode_engine_output(output)
    }

    pub fn snapshot(&mut self) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let Some(snapshot) = self.engine.snapshot() else {
            return Ok(BindingOutput::default());
        };
        let bytes = encode_snapshot_json(
            &snapshot,
            self.wire_limits.max_encoded_snapshot_bytes,
            self.protocol_limits,
        )
        .map_err(protocol_error)?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::Snapshot,
            bytes,
        );
        Ok(output)
    }

    pub fn execute(&mut self, command_json: &[u8]) -> Result<BindingOutput, BindingError> {
        match decode_engine_command(command_json, self.wire_limits.max_command_bytes)? {
            EngineCommand::Append { chunk, .. } => {
                self.metrics.commands = self.metrics.commands.saturating_add(1);
                self.append_text(&chunk)
            }
            EngineCommand::Finish { .. } => self.finish(),
            EngineCommand::Reset { .. } => self.reset(),
            EngineCommand::Snapshot { .. } => self.snapshot(),
        }
    }

    pub const fn metrics(&self) -> BindingMetrics {
        self.metrics
    }

    fn encode_engine_output(
        &mut self,
        engine_output: EngineOutput,
    ) -> Result<BindingOutput, BindingError> {
        let mut output = BindingOutput::default();
        for change in engine_output.into_changes() {
            let bytes = encode_change_json(
                &change,
                self.wire_limits.max_encoded_change_bytes,
                self.protocol_limits,
            )
            .map_err(protocol_error)?;
            push_recorded(
                &mut output,
                &mut self.metrics,
                BindingPayloadKind::Change,
                bytes,
            );
        }
        Ok(output)
    }
}

enum SessionReducer {
    Plain(Reducer),
    Captured(TransitionReducer),
}

impl SessionReducer {
    fn status(&self) -> ReducerStatus {
        match self {
            Self::Plain(reducer) => reducer.status(),
            Self::Captured(reducer) => reducer.status(),
        }
    }

    fn document(&self) -> Option<&Document> {
        match self {
            Self::Plain(reducer) => reducer.document(),
            Self::Captured(reducer) => reducer.document(),
        }
    }

    fn apply(&mut self, change: ChangeSet) -> Result<TransitionOutcome, BindingError> {
        match self {
            Self::Plain(reducer) => reducer
                .apply(change)
                .map(|outcome| TransitionOutcome {
                    outcome,
                    facts: None,
                })
                .map_err(protocol_error),
            Self::Captured(reducer) => reducer.apply(change).map_err(transition_error),
        }
    }

    fn recover_snapshot(&mut self, snapshot: Snapshot) -> Result<TransitionOutcome, BindingError> {
        match self {
            Self::Plain(reducer) => reducer
                .recover_snapshot(snapshot)
                .map(|outcome| TransitionOutcome {
                    outcome,
                    facts: None,
                })
                .map_err(protocol_error),
            Self::Captured(reducer) => reducer.recover_snapshot(snapshot).map_err(transition_error),
        }
    }
}

fn transition_error(error: TransitionError) -> BindingError {
    match error {
        TransitionError::Protocol(error) => protocol_error(error),
        TransitionError::ContinuityOverflow => {
            BindingError::internal("transition continuity generation overflowed")
        }
    }
}

pub struct ReducerSession {
    reducer: SessionReducer,
    host: ArtifactHost,
    protocol_limits: ProtocolLimits,
    wire_limits: WireLimits,
    pending_requests: BTreeMap<RequestGeneration, ProcessorRequestKey>,
    last_issued_generation: Option<RequestGeneration>,
    metrics: BindingMetrics,
}

impl fmt::Debug for ReducerSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReducerSession")
            .field("status", &self.reducer.status())
            .field("processor_metrics", &self.host.metrics())
            .field("pending_requests", &self.pending_requests.len())
            .field("metrics", &self.metrics)
            .finish_non_exhaustive()
    }
}

impl ReducerSession {
    pub fn new(options_json: &[u8]) -> Result<Self, BindingError> {
        let options = BindingOptions::parse(options_json)?;
        let capture_transitions = options.capture_transitions();
        let (reducer, host, protocol_limits, wire_limits) = options.into_reducer()?;
        let reducer = if capture_transitions {
            SessionReducer::Captured(TransitionReducer::with_limits(protocol_limits))
        } else {
            SessionReducer::Plain(reducer)
        };
        Ok(Self {
            reducer,
            host,
            protocol_limits,
            wire_limits,
            pending_requests: BTreeMap::new(),
            last_issued_generation: None,
            metrics: BindingMetrics::default(),
        })
    }

    pub fn apply_change(&mut self, change_json: &[u8]) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let change = decode_change_json(
            change_json,
            self.wire_limits.max_encoded_change_bytes,
            self.protocol_limits,
        )
        .map_err(protocol_error)?;
        self.metrics.decoded_change_payloads =
            self.metrics.decoded_change_payloads.saturating_add(1);
        let outcome = self.reducer.apply(change)?;
        self.finish_reducer_transition(outcome)
    }

    pub fn recover_snapshot(
        &mut self,
        snapshot_json: &[u8],
    ) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let snapshot = decode_snapshot_json(
            snapshot_json,
            self.wire_limits.max_encoded_snapshot_bytes,
            self.protocol_limits,
        )
        .map_err(protocol_error)?;
        self.metrics.decoded_snapshot_payloads =
            self.metrics.decoded_snapshot_payloads.saturating_add(1);
        let outcome = self.reducer.recover_snapshot(snapshot)?;
        self.finish_reducer_transition(outcome)
    }

    pub fn snapshot(&mut self) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let Some(document) = self.reducer.document() else {
            return Ok(BindingOutput::default());
        };
        let bytes = encode_snapshot_json(
            &document.snapshot(),
            self.wire_limits.max_encoded_snapshot_bytes,
            self.protocol_limits,
        )
        .map_err(protocol_error)?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::Snapshot,
            bytes,
        );
        Ok(output)
    }

    pub fn node_view(&mut self, node_id: NodeId) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let document = self.reducer.document().ok_or_else(|| {
            BindingError::new(
                BindingStatus::InvalidArgument,
                "bindings.document_uninitialized",
                "canonical document is not initialized",
            )
        })?;
        let node = document.node(node_id).ok_or_else(|| {
            BindingError::new(
                BindingStatus::InvalidArgument,
                "bindings.node_not_found",
                format!("node {node_id} was not found"),
            )
        })?;
        let bytes = encode_node_view(document, node, self.wire_limits.max_view_bytes)?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::NodeView,
            bytes,
        );
        Ok(output)
    }

    pub fn resource_view(
        &mut self,
        resource_id: ResourceId,
    ) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let resource = self
            .reducer
            .document()
            .and_then(|document| document.resource(resource_id))
            .ok_or_else(|| {
                BindingError::new(
                    BindingStatus::InvalidArgument,
                    "bindings.resource_not_found",
                    format!("resource {resource_id} was not found"),
                )
            })?;
        let bytes = encode_resource_view(resource, self.wire_limits.max_view_bytes)?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::ResourceView,
            bytes,
        );
        Ok(output)
    }

    pub fn pending_source_view(&mut self) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let Some(document) = self.reducer.document() else {
            return Ok(BindingOutput::default());
        };
        if document.pending_source().is_empty() {
            return Ok(BindingOutput::default());
        }
        let bytes = encode_pending_source_view(document, self.wire_limits.max_view_bytes)?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::PendingSourceView,
            bytes,
        );
        Ok(output)
    }

    pub fn begin_native_processor(
        &mut self,
        descriptor: ProcessorDescriptor,
        node_id: NodeId,
        configuration: ConfigurationVersion,
        policy: ProcessingPolicy,
    ) -> Result<(ProcessorRequest, BindingOutput), BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let document = self.reducer.document().ok_or_else(|| {
            BindingError::new(
                BindingStatus::Processor,
                "processor.epoch_not_initialized",
                "processor document is not initialized",
            )
        })?;
        let request = self
            .host
            .begin(document, descriptor, node_id, configuration, policy)
            .map_err(host_error)?;
        let output = self.record_begun_processor(&request)?;
        Ok((request, output))
    }

    pub fn begin_native_processor_if_current(
        &mut self,
        expectation: ProcessorExpectation,
        descriptor: ProcessorDescriptor,
        configuration: ConfigurationVersion,
        policy: ProcessingPolicy,
    ) -> Result<(Option<ProcessorRequest>, BindingOutput), BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let document = self.reducer.document().ok_or_else(|| {
            BindingError::new(
                BindingStatus::Processor,
                "processor.epoch_not_initialized",
                "processor document is not initialized",
            )
        })?;
        let Some(request) = self
            .host
            .begin_if_current(document, expectation, descriptor, configuration, policy)
            .map_err(host_error)?
        else {
            return Ok((None, BindingOutput::default()));
        };
        let output = self.record_begun_processor(&request)?;
        Ok((Some(request), output))
    }

    fn record_begun_processor(
        &mut self,
        request: &ProcessorRequest,
    ) -> Result<BindingOutput, BindingError> {
        let generation = request.key().generation();
        self.pending_requests
            .insert(generation, request.key().clone());
        self.sync_pending_request_metric();
        self.last_issued_generation = Some(
            self.last_issued_generation
                .map_or(generation, |current| current.max(generation)),
        );

        let request_bytes =
            encode_processor_request(request, self.wire_limits.max_processor_payload_bytes)?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::ProcessorRequest,
            request_bytes,
        );
        output.extend(self.drain_artifact_changes()?);
        Ok(output)
    }

    pub fn begin_processor(
        &mut self,
        node_id: NodeId,
        processor_id: String,
        processor_version: String,
        configuration_version: String,
        accepts_provisional: bool,
        allow_provisional: bool,
    ) -> Result<BindingOutput, BindingError> {
        let capabilities = if accepts_provisional {
            ProcessorCapabilities::with_provisional()
        } else {
            ProcessorCapabilities::stable_only()
        };
        let descriptor = ProcessorDescriptor::new(processor_id, processor_version, capabilities)
            .map_err(identifier_error)?;
        let configuration =
            ConfigurationVersion::new(configuration_version).map_err(identifier_error)?;
        let (_, output) = self.begin_native_processor(
            descriptor,
            node_id,
            configuration,
            processing_policy(allow_provisional),
        )?;
        Ok(output)
    }

    pub fn begin_processor_if_current(
        &mut self,
        expectation: ProcessorExpectation,
        processor_id: String,
        processor_version: String,
        configuration_version: String,
        accepts_provisional: bool,
        allow_provisional: bool,
    ) -> Result<BindingOutput, BindingError> {
        let capabilities = if accepts_provisional {
            ProcessorCapabilities::with_provisional()
        } else {
            ProcessorCapabilities::stable_only()
        };
        let descriptor = ProcessorDescriptor::new(processor_id, processor_version, capabilities)
            .map_err(identifier_error)?;
        let configuration =
            ConfigurationVersion::new(configuration_version).map_err(identifier_error)?;
        let (_, output) = self.begin_native_processor_if_current(
            expectation,
            descriptor,
            configuration,
            processing_policy(allow_provisional),
        )?;
        Ok(output)
    }

    pub fn complete_native_processor(
        &mut self,
        result: ProcessorResult,
    ) -> Result<(CompletionOutcome, BindingOutput), BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let request_key = result.key().clone();
        let request_id = request_key.generation();
        let document = self.reducer.document().ok_or_else(|| {
            BindingError::new(
                BindingStatus::Processor,
                "processor.epoch_not_initialized",
                "processor document is not initialized",
            )
        })?;
        let outcome = self
            .host
            .complete(document, result)
            .map_err(|error| host_error(error.error().clone()))?;
        if self.pending_requests.get(&request_id) == Some(&request_key) {
            self.pending_requests.remove(&request_id);
            self.sync_pending_request_metric();
        }
        let output = self.processor_completion_output(request_id, outcome)?;
        Ok((outcome, output))
    }

    pub fn complete_processor_text(
        &mut self,
        request_id: RequestGeneration,
        protocol: String,
        media_type: String,
        text: String,
    ) -> Result<BindingOutput, BindingError> {
        self.complete_foreign_processor(
            request_id,
            ProcessorCompletion::Text {
                protocol,
                media_type,
                text,
            },
        )
    }

    pub fn complete_processor_binary(
        &mut self,
        request_id: RequestGeneration,
        protocol: String,
        media_type: String,
        bytes: Vec<u8>,
    ) -> Result<BindingOutput, BindingError> {
        self.complete_foreign_processor(
            request_id,
            ProcessorCompletion::Binary {
                protocol,
                media_type,
                bytes,
            },
        )
    }

    pub fn fail_processor(
        &mut self,
        request_id: RequestGeneration,
        code: ProcessorFailureCode,
        message: String,
    ) -> Result<BindingOutput, BindingError> {
        self.complete_foreign_processor(request_id, ProcessorCompletion::Failure { code, message })
    }

    pub fn cancel_processor(
        &mut self,
        request_id: RequestGeneration,
    ) -> Result<BindingOutput, BindingError> {
        self.cancel_foreign_processor(request_id)
    }

    pub fn artifact_view(
        &mut self,
        slot: &ProcessorSlotKey,
    ) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let Some(state) = self.host.state(slot) else {
            return Ok(BindingOutput::default());
        };
        let bytes = encode_artifact_view(state, self.wire_limits.max_view_bytes)?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::ArtifactView,
            bytes,
        );
        Ok(output)
    }

    pub fn artifact_view_for(
        &mut self,
        epoch: mdstream_protocol::Epoch,
        node_id: NodeId,
        processor_id: String,
    ) -> Result<BindingOutput, BindingError> {
        let slot = ProcessorSlotKey::new(
            epoch,
            node_id,
            ProcessorId::new(processor_id).map_err(identifier_error)?,
        );
        self.artifact_view(&slot)
    }

    pub fn execute(&mut self, command_json: &[u8]) -> Result<BindingOutput, BindingError> {
        match decode_reducer_command(command_json, self.wire_limits.max_command_bytes)? {
            ReducerCommand::ApplyChange { change, .. } => {
                self.apply_change(change.get().as_bytes())
            }
            ReducerCommand::RecoverSnapshot { snapshot, .. } => {
                self.recover_snapshot(snapshot.get().as_bytes())
            }
            ReducerCommand::Snapshot { .. } => self.snapshot(),
            ReducerCommand::NodeView { node_id, .. } => {
                self.node_view(parse_decimal_id(&node_id, "node_id")?)
            }
            ReducerCommand::ResourceView { resource_id, .. } => {
                self.resource_view(parse_decimal_id(&resource_id, "resource_id")?)
            }
            ReducerCommand::PendingSourceView { .. } => self.pending_source_view(),
            ReducerCommand::BeginProcessor {
                node_id,
                processor_id,
                processor_version,
                configuration_version,
                accepts_provisional,
                allow_provisional,
                ..
            } => self.begin_processor(
                parse_decimal_id(&node_id, "node_id")?,
                processor_id,
                processor_version,
                configuration_version,
                accepts_provisional,
                allow_provisional,
            ),
            ReducerCommand::BeginProcessorIfCurrent {
                expected_epoch,
                node_id,
                expected_node_version,
                processor_id,
                processor_version,
                configuration_version,
                accepts_provisional,
                allow_provisional,
                ..
            } => self.begin_processor_if_current(
                ProcessorExpectation::new(
                    parse_decimal_id(&expected_epoch, "expected_epoch")?,
                    parse_decimal_id(&node_id, "node_id")?,
                    NodeVersion::new(expected_node_version).map_err(|error| {
                        BindingError::new(
                            BindingStatus::InvalidArgument,
                            "processor.invalid_node_version",
                            error.to_string(),
                        )
                    })?,
                ),
                processor_id,
                processor_version,
                configuration_version,
                accepts_provisional,
                allow_provisional,
            ),
            ReducerCommand::CompleteProcessor {
                request_id,
                outcome,
                ..
            } => self.complete_foreign_processor(
                parse_decimal_id(&request_id, "request_id")?,
                decode_processor_completion(outcome, self.wire_limits.max_processor_payload_bytes)?,
            ),
            ReducerCommand::CancelProcessor { request_id, .. } => {
                self.cancel_foreign_processor(parse_decimal_id(&request_id, "request_id")?)
            }
            ReducerCommand::ArtifactView {
                epoch,
                node_id,
                processor_id,
                ..
            } => self.artifact_view_for(
                parse_decimal_id(&epoch, "epoch")?,
                parse_decimal_id(&node_id, "node_id")?,
                processor_id,
            ),
        }
    }

    pub fn status(&self) -> ReducerStatus {
        self.reducer.status()
    }

    pub const fn metrics(&self) -> BindingMetrics {
        self.metrics
    }

    pub fn processor_metrics(&self) -> mdstream_processors::ProcessorMetrics {
        self.host.metrics()
    }

    fn finish_reducer_transition(
        &mut self,
        transition: TransitionOutcome,
    ) -> Result<BindingOutput, BindingError> {
        let TransitionOutcome { outcome, facts } = transition;
        let empty_impact = ChangeImpact::default();
        let impact = match &outcome {
            ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => impact,
            ApplyOutcome::Idempotent
            | ApplyOutcome::Stale { .. }
            | ApplyOutcome::RecoveryRequired { .. } => &empty_impact,
        };

        if matches!(
            outcome,
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ) {
            let document = self.reducer.document().ok_or_else(|| {
                BindingError::internal("state-changing reducer outcome omitted its document")
            })?;
            self.host.reconcile(document, impact).map_err(host_error)?;
            if impact.full_replace {
                self.retire_pending_requests();
            }
        }

        let update = encode_reducer_update(
            &outcome,
            &self.reducer.status(),
            impact,
            self.reducer.document(),
            facts.as_ref(),
            self.wire_limits.max_reducer_update_bytes,
        )?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::ReducerUpdate,
            update,
        );
        output.extend(self.drain_artifact_changes()?);
        Ok(output)
    }

    fn complete_foreign_processor(
        &mut self,
        request_id: RequestGeneration,
        completion: ProcessorCompletion,
    ) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let Some(key) = self.pending_requests.get(&request_id).cloned() else {
            if self
                .last_issued_generation
                .is_some_and(|last| request_id <= last)
            {
                return self.processor_completion_output(request_id, CompletionOutcome::Stale);
            }
            return Err(BindingError::command(format!(
                "processor request {request_id} was not issued by this session"
            )));
        };

        let result = match completion {
            ProcessorCompletion::Text {
                protocol,
                media_type,
                text,
            } => ProcessorResult::success(
                key,
                ProcessorArtifact::text(protocol, media_type, text).map_err(identifier_error)?,
            ),
            ProcessorCompletion::Binary {
                protocol,
                media_type,
                bytes,
            } => ProcessorResult::success(
                key,
                ProcessorArtifact::binary(protocol, media_type, bytes).map_err(identifier_error)?,
            ),
            ProcessorCompletion::Failure { code, message } => {
                ProcessorResult::failure(key, ProcessorFailure::new(code, message))
            }
        };
        let document = self.reducer.document().ok_or_else(|| {
            BindingError::new(
                BindingStatus::Processor,
                "processor.epoch_not_initialized",
                "processor document is not initialized",
            )
        })?;
        let outcome = self
            .host
            .complete(document, result)
            .map_err(|error| host_error(error.error().clone()))?;
        self.pending_requests.remove(&request_id);
        self.sync_pending_request_metric();
        self.processor_completion_output(request_id, outcome)
    }

    fn cancel_foreign_processor(
        &mut self,
        request_id: RequestGeneration,
    ) -> Result<BindingOutput, BindingError> {
        self.metrics.commands = self.metrics.commands.saturating_add(1);
        let Some(key) = self.pending_requests.get(&request_id).cloned() else {
            return self.processor_completion_output(request_id, CompletionOutcome::Stale);
        };
        let cancelled = self.host.cancel(&key).map_err(host_error)?;
        self.pending_requests.remove(&request_id);
        self.sync_pending_request_metric();
        let outcome = if cancelled {
            CompletionOutcome::Applied
        } else {
            CompletionOutcome::Stale
        };
        self.processor_completion_output(request_id, outcome)
    }

    fn processor_completion_output(
        &mut self,
        request_id: RequestGeneration,
        outcome: CompletionOutcome,
    ) -> Result<BindingOutput, BindingError> {
        let bytes = encode_processor_completion(
            request_id,
            outcome,
            self.wire_limits.max_processor_payload_bytes,
        )?;
        let mut output = BindingOutput::default();
        push_recorded(
            &mut output,
            &mut self.metrics,
            BindingPayloadKind::ProcessorCompletion,
            bytes,
        );
        output.extend(self.drain_artifact_changes()?);
        Ok(output)
    }

    fn drain_artifact_changes(&mut self) -> Result<BindingOutput, BindingError> {
        let changes = self.host.take_changes();
        self.retire_invalidated_requests(&changes);
        let mut output = BindingOutput::default();
        for change in changes {
            let bytes = encode_artifact_change(&change, self.wire_limits.max_artifact_event_bytes)?;
            push_recorded(
                &mut output,
                &mut self.metrics,
                BindingPayloadKind::ArtifactChange,
                bytes,
            );
        }
        Ok(output)
    }

    fn retire_pending_requests(&mut self) {
        self.pending_requests.clear();
        self.sync_pending_request_metric();
    }

    fn retire_invalidated_requests(&mut self, changes: &[ArtifactChange]) {
        let mut retired_slots = BTreeSet::new();
        let mut retired_keys = BTreeSet::new();
        for change in changes {
            let ArtifactChangeKind::Removed { reason, .. } = change.kind() else {
                continue;
            };
            match reason {
                ArtifactReleaseReason::Replaced => {}
                ArtifactReleaseReason::Cancelled => {
                    retired_keys.insert(change.key().clone());
                }
                ArtifactReleaseReason::NodeChanged
                | ArtifactReleaseReason::NodeRemoved
                | ArtifactReleaseReason::EpochReset => {
                    retired_slots.insert(change.key().slot().clone());
                }
            }
        }
        if retired_slots.is_empty() && retired_keys.is_empty() {
            return;
        }
        self.pending_requests
            .retain(|_, key| !retired_slots.contains(key.slot()) && !retired_keys.contains(key));
        self.sync_pending_request_metric();
    }

    fn sync_pending_request_metric(&mut self) {
        self.metrics.pending_processor_requests =
            u64::try_from(self.pending_requests.len()).unwrap_or(u64::MAX);
    }
}
