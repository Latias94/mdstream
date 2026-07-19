use std::io::{self, Write};

use mdstream_processors::{
    ArtifactChange, ArtifactChangeKind, CompletionOutcome, ProcessorArtifact, ProcessorFailure,
    ProcessorRequest, ProcessorRequestKey, ProcessorSlotState,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeImpact, ChildList, ContentNode, Coordinate, Document, DocumentLifecycle,
    Epoch, NodeId, RecoveryReason, ReducerStatus, RequestGeneration, SemanticResource, Sequence,
    SourceCursor, TransitionFacts,
};
use serde::Serialize;

use crate::{BindingError, errors::protocol_error};

pub const BINDING_SCHEMA: &str = "mdstream.bindings/0.4";
pub const TRANSITION_SCHEMA_DRAFT: &str = "mdstream.transitions/draft";

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingPayloadKind {
    Change = 1,
    Snapshot = 2,
    ReducerUpdate = 3,
    NodeView = 4,
    ResourceView = 5,
    ProcessorRequest = 6,
    ProcessorCompletion = 7,
    ArtifactChange = 8,
    ArtifactView = 9,
    PendingSourceView = 10,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingPayload {
    kind: BindingPayloadKind,
    bytes: Vec<u8>,
}

impl BindingPayload {
    pub const fn kind(&self) -> BindingPayloadKind {
        self.kind
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingOutput {
    payloads: Vec<BindingPayload>,
}

impl BindingOutput {
    pub fn payloads(&self) -> &[BindingPayload] {
        &self.payloads
    }

    pub fn into_payloads(self) -> Vec<BindingPayload> {
        self.payloads
    }

    pub fn is_empty(&self) -> bool {
        self.payloads.is_empty()
    }

    pub fn count(&self, kind: BindingPayloadKind) -> usize {
        self.payloads
            .iter()
            .filter(|payload| payload.kind == kind)
            .count()
    }

    pub(crate) fn push(&mut self, kind: BindingPayloadKind, bytes: Vec<u8>) {
        self.payloads.push(BindingPayload { kind, bytes });
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.payloads.extend(other.payloads);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindingMetrics {
    pub commands: u64,
    pub decoded_change_payloads: u64,
    pub decoded_snapshot_payloads: u64,
    pub change_payloads: u64,
    pub snapshot_payloads: u64,
    pub reducer_update_payloads: u64,
    pub processor_request_payloads: u64,
    pub processor_completion_payloads: u64,
    pub artifact_change_payloads: u64,
    pub artifact_view_payloads: u64,
    pub materialized_node_views: u64,
    pub materialized_resource_views: u64,
    pub materialized_pending_source_views: u64,
    pub encoded_payload_bytes: u64,
    pub pending_processor_requests: u64,
}

impl BindingMetrics {
    pub(crate) fn record(&mut self, kind: BindingPayloadKind, bytes: usize) {
        match kind {
            BindingPayloadKind::Change => {
                self.change_payloads = self.change_payloads.saturating_add(1)
            }
            BindingPayloadKind::Snapshot => {
                self.snapshot_payloads = self.snapshot_payloads.saturating_add(1)
            }
            BindingPayloadKind::ReducerUpdate => {
                self.reducer_update_payloads = self.reducer_update_payloads.saturating_add(1)
            }
            BindingPayloadKind::ProcessorRequest => {
                self.processor_request_payloads = self.processor_request_payloads.saturating_add(1)
            }
            BindingPayloadKind::ProcessorCompletion => {
                self.processor_completion_payloads =
                    self.processor_completion_payloads.saturating_add(1)
            }
            BindingPayloadKind::ArtifactChange => {
                self.artifact_change_payloads = self.artifact_change_payloads.saturating_add(1)
            }
            BindingPayloadKind::NodeView => {
                self.materialized_node_views = self.materialized_node_views.saturating_add(1)
            }
            BindingPayloadKind::ResourceView => {
                self.materialized_resource_views =
                    self.materialized_resource_views.saturating_add(1)
            }
            BindingPayloadKind::PendingSourceView => {
                self.materialized_pending_source_views =
                    self.materialized_pending_source_views.saturating_add(1)
            }
            BindingPayloadKind::ArtifactView => {
                self.artifact_view_payloads = self.artifact_view_payloads.saturating_add(1)
            }
        }
        self.encoded_payload_bytes = self
            .encoded_payload_bytes
            .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
    }
}

pub(crate) fn push_recorded(
    output: &mut BindingOutput,
    metrics: &mut BindingMetrics,
    kind: BindingPayloadKind,
    bytes: Vec<u8>,
) {
    metrics.record(kind, bytes.len());
    output.push(kind, bytes);
}

pub(crate) fn encode_reducer_update(
    outcome: &ApplyOutcome,
    status: &ReducerStatus,
    impact: &ChangeImpact,
    document: Option<&Document>,
    transition: Option<&TransitionFacts>,
    max_bytes: usize,
) -> Result<Vec<u8>, BindingError> {
    let document = document.map(|document| DocumentView {
        coordinate: document.coordinate(),
        lifecycle: document.lifecycle(),
        projection_cursor: document.projection_cursor(),
        roots: (impact.roots_changed || impact.full_replace).then_some(document.roots()),
    });
    encode_json_bounded(
        &ReducerUpdate {
            schema: BINDING_SCHEMA,
            kind: "reducer_update",
            outcome: OutcomeView::from(outcome),
            status: StatusView::from(status),
            impact: ImpactView::from(impact),
            document,
            transition: transition.map(|facts| TransitionView {
                schema: TRANSITION_SCHEMA_DRAFT,
                facts,
            }),
        },
        max_bytes,
        "bindings.reducer_update_bytes",
    )
}

pub(crate) fn encode_node_view(
    document: &Document,
    node: &ContentNode,
    max_bytes: usize,
) -> Result<Vec<u8>, BindingError> {
    let body_text = slice_range(document.source(), node.body)?;
    encode_json_bounded(
        &NodeView {
            schema: BINDING_SCHEMA,
            kind: "node_view",
            node,
            body_text,
        },
        max_bytes,
        "bindings.node_view_bytes",
    )
}

pub(crate) fn encode_resource_view(
    resource: &SemanticResource,
    max_bytes: usize,
) -> Result<Vec<u8>, BindingError> {
    encode_json_bounded(
        &ResourceView {
            schema: BINDING_SCHEMA,
            kind: "resource_view",
            resource,
        },
        max_bytes,
        "bindings.resource_view_bytes",
    )
}

pub(crate) fn encode_pending_source_view(
    document: &Document,
    max_bytes: usize,
) -> Result<Vec<u8>, BindingError> {
    encode_json_bounded(
        &PendingSourceView {
            schema: BINDING_SCHEMA,
            kind: "pending_source_view",
            range: document.pending_source_range(),
            text: document.pending_source(),
        },
        max_bytes,
        "bindings.pending_source_view_bytes",
    )
}

pub(crate) fn encode_processor_request(
    request: &ProcessorRequest,
    max_bytes: usize,
) -> Result<Vec<u8>, BindingError> {
    encode_json_bounded(
        &ProcessorRequestView {
            schema: BINDING_SCHEMA,
            kind: "processor_request",
            request_id: request.key().generation(),
            key: ProcessorKeyView::from(request.key()),
            input: ProcessorInputView {
                node: request.input().node(),
                body: request.input().body(),
                resource: request.input().resource(),
            },
        },
        max_bytes,
        "bindings.processor_request_bytes",
    )
}

pub(crate) fn encode_processor_completion(
    request_id: RequestGeneration,
    outcome: CompletionOutcome,
    max_bytes: usize,
) -> Result<Vec<u8>, BindingError> {
    encode_json_bounded(
        &ProcessorCompletionView {
            schema: BINDING_SCHEMA,
            kind: "processor_completion",
            request_id,
            outcome: match outcome {
                CompletionOutcome::Applied => "applied",
                CompletionOutcome::Stale => "stale",
            },
        },
        max_bytes,
        "bindings.processor_completion_bytes",
    )
}

pub(crate) fn encode_artifact_change(
    change: &ArtifactChange,
    max_bytes: usize,
) -> Result<Vec<u8>, BindingError> {
    encode_json_bounded(
        &ArtifactChangeView {
            schema: BINDING_SCHEMA,
            kind: "artifact_change",
            key: ProcessorKeyView::from(change.key()),
            change: ArtifactChangeKindView::from(change.kind()),
        },
        max_bytes,
        "bindings.artifact_change_bytes",
    )
}

pub(crate) fn encode_artifact_view(
    state: &ProcessorSlotState,
    max_bytes: usize,
) -> Result<Vec<u8>, BindingError> {
    let (state_name, artifact, failure) = match state {
        ProcessorSlotState::Pending { .. } => ("pending", None, None),
        ProcessorSlotState::Ready { artifact, .. } => {
            ("ready", Some(ArtifactView::from(artifact)), None)
        }
        ProcessorSlotState::Failed { failure, .. } => {
            ("failed", None, Some(FailureView::from(failure)))
        }
    };
    encode_json_bounded(
        &ArtifactStateView {
            schema: BINDING_SCHEMA,
            kind: "artifact_view",
            key: ProcessorKeyView::from(state.key()),
            state: state_name,
            artifact,
            failure,
        },
        max_bytes,
        "bindings.artifact_view_bytes",
    )
}

pub(crate) fn encode_json_bounded<T: Serialize>(
    value: &T,
    max_bytes: usize,
    field: &'static str,
) -> Result<Vec<u8>, BindingError> {
    let mut writer = BoundedWriter::new(max_bytes);
    let result = serde_json::to_writer(&mut writer, value);
    if writer.bytes.len() > max_bytes {
        return Err(BindingError::resource(field, max_bytes, writer.bytes.len()));
    }
    result.map_err(|error| BindingError::internal(format!("failed to encode {field}: {error}")))?;
    Ok(writer.bytes)
}

fn slice_range(source: &str, range: mdstream_protocol::SourceRange) -> Result<&str, BindingError> {
    range.validate(source).map_err(protocol_error)?;
    let start = usize::try_from(range.start.get())
        .map_err(|_| BindingError::internal("source range start does not fit usize"))?;
    let end = usize::try_from(range.end.get())
        .map_err(|_| BindingError::internal("source range end does not fit usize"))?;
    source
        .get(start..end)
        .ok_or_else(|| BindingError::internal("validated source range could not be sliced"))
}

struct BoundedWriter {
    bytes: Vec<u8>,
    max_retained: usize,
}

impl BoundedWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(max_bytes.saturating_add(1).min(512)),
            max_retained: max_bytes.saturating_add(1),
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let remaining = self.max_retained.saturating_sub(self.bytes.len());
        let accepted = remaining.min(buffer.len());
        self.bytes.extend_from_slice(&buffer[..accepted]);
        if accepted < buffer.len() {
            return Err(io::Error::other("binding output limit exceeded"));
        }
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Serialize)]
struct ReducerUpdate<'a> {
    schema: &'static str,
    kind: &'static str,
    outcome: OutcomeView,
    status: StatusView,
    impact: ImpactView<'a>,
    document: Option<DocumentView<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transition: Option<TransitionView<'a>>,
}

#[derive(Serialize)]
struct TransitionView<'a> {
    schema: &'static str,
    facts: &'a TransitionFacts,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum OutcomeView {
    Applied {
        coordinate: Coordinate,
    },
    Recovered {
        coordinate: Coordinate,
    },
    Idempotent,
    Stale {
        current: Coordinate,
        received_epoch: Epoch,
        received_sequence: Sequence,
    },
    RecoveryRequired {
        last_good: Coordinate,
        reason: RecoveryReason,
    },
}

impl From<&ApplyOutcome> for OutcomeView {
    fn from(outcome: &ApplyOutcome) -> Self {
        match outcome {
            ApplyOutcome::Applied { coordinate, .. } => Self::Applied {
                coordinate: coordinate.clone(),
            },
            ApplyOutcome::Recovered { coordinate, .. } => Self::Recovered {
                coordinate: coordinate.clone(),
            },
            ApplyOutcome::Idempotent => Self::Idempotent,
            ApplyOutcome::Stale {
                current,
                received_epoch,
                received_sequence,
            } => Self::Stale {
                current: current.clone(),
                received_epoch: *received_epoch,
                received_sequence: *received_sequence,
            },
            ApplyOutcome::RecoveryRequired { last_good, reason } => Self::RecoveryRequired {
                last_good: last_good.clone(),
                reason: reason.clone(),
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum StatusView {
    Uninitialized,
    Ready,
    NeedsSnapshot {
        last_good: Coordinate,
        reason: RecoveryReason,
    },
}

impl From<&ReducerStatus> for StatusView {
    fn from(status: &ReducerStatus) -> Self {
        match status {
            ReducerStatus::Uninitialized => Self::Uninitialized,
            ReducerStatus::Ready => Self::Ready,
            ReducerStatus::NeedsSnapshot { last_good, reason } => Self::NeedsSnapshot {
                last_good: last_good.clone(),
                reason: reason.clone(),
            },
        }
    }
}

#[derive(Serialize)]
struct ImpactView<'a> {
    changed_node_ids: &'a [mdstream_protocol::NodeId],
    removed_node_ids: &'a [mdstream_protocol::NodeId],
    changed_resource_ids: &'a [mdstream_protocol::ResourceId],
    removed_resource_ids: &'a [mdstream_protocol::ResourceId],
    source_changed: bool,
    projection_changed: bool,
    lifecycle_changed: bool,
    roots_changed: bool,
    full_replace: bool,
}

impl<'a> From<&'a ChangeImpact> for ImpactView<'a> {
    fn from(impact: &'a ChangeImpact) -> Self {
        Self {
            changed_node_ids: &impact.changed_nodes,
            removed_node_ids: &impact.removed_nodes,
            changed_resource_ids: &impact.changed_resources,
            removed_resource_ids: &impact.removed_resources,
            source_changed: impact.source_changed,
            projection_changed: impact.projection_changed,
            lifecycle_changed: impact.lifecycle_changed,
            roots_changed: impact.roots_changed,
            full_replace: impact.full_replace,
        }
    }
}

#[derive(Serialize)]
struct DocumentView<'a> {
    coordinate: &'a Coordinate,
    lifecycle: DocumentLifecycle,
    projection_cursor: SourceCursor,
    #[serde(skip_serializing_if = "Option::is_none")]
    roots: Option<&'a ChildList>,
}

#[derive(Serialize)]
struct NodeView<'a> {
    schema: &'static str,
    kind: &'static str,
    node: &'a ContentNode,
    body_text: &'a str,
}

#[derive(Serialize)]
struct ResourceView<'a> {
    schema: &'static str,
    kind: &'static str,
    resource: &'a SemanticResource,
}

#[derive(Serialize)]
struct PendingSourceView<'a> {
    schema: &'static str,
    kind: &'static str,
    range: mdstream_protocol::SourceRange,
    text: &'a str,
}

#[derive(Serialize)]
struct ProcessorRequestView<'a> {
    schema: &'static str,
    kind: &'static str,
    request_id: RequestGeneration,
    key: ProcessorKeyView<'a>,
    input: ProcessorInputView<'a>,
}

#[derive(Serialize)]
struct ProcessorInputView<'a> {
    node: &'a ContentNode,
    body: &'a str,
    resource: Option<&'a SemanticResource>,
}

#[derive(Serialize)]
struct ProcessorKeyView<'a> {
    epoch: Epoch,
    node_id: NodeId,
    processor_id: &'a str,
    node_version: &'a str,
    input_version: &'a str,
    processor_version: &'a str,
    configuration_version: &'a str,
    generation: RequestGeneration,
}

impl<'a> From<&'a ProcessorRequestKey> for ProcessorKeyView<'a> {
    fn from(key: &'a ProcessorRequestKey) -> Self {
        Self {
            epoch: key.slot().epoch(),
            node_id: key.slot().node_id(),
            processor_id: key.slot().processor_id().as_str(),
            node_version: key.node_version().as_str(),
            input_version: key.input_version().as_str(),
            processor_version: key.processor_version().as_str(),
            configuration_version: key.configuration_version().as_str(),
            generation: key.generation(),
        }
    }
}

#[derive(Serialize)]
struct ProcessorCompletionView {
    schema: &'static str,
    kind: &'static str,
    request_id: RequestGeneration,
    outcome: &'static str,
}

#[derive(Serialize)]
struct ArtifactChangeView<'a> {
    schema: &'static str,
    kind: &'static str,
    key: ProcessorKeyView<'a>,
    change: ArtifactChangeKindView,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum ArtifactChangeKindView {
    Pending,
    Ready {
        artifact_bytes: String,
    },
    Failed {
        code: &'static str,
    },
    Removed {
        reason: &'static str,
        released_artifact_bytes: String,
    },
}

impl From<&ArtifactChangeKind> for ArtifactChangeKindView {
    fn from(kind: &ArtifactChangeKind) -> Self {
        match kind {
            ArtifactChangeKind::Pending => Self::Pending,
            ArtifactChangeKind::Ready { artifact_bytes } => Self::Ready {
                artifact_bytes: artifact_bytes.to_string(),
            },
            ArtifactChangeKind::Failed { code } => Self::Failed {
                code: code.as_str(),
            },
            ArtifactChangeKind::Removed {
                reason,
                released_artifact_bytes,
            } => Self::Removed {
                reason: match reason {
                    mdstream_processors::ArtifactReleaseReason::Replaced => "replaced",
                    mdstream_processors::ArtifactReleaseReason::Cancelled => "cancelled",
                    mdstream_processors::ArtifactReleaseReason::NodeChanged => "node_changed",
                    mdstream_processors::ArtifactReleaseReason::NodeRemoved => "node_removed",
                    mdstream_processors::ArtifactReleaseReason::EpochReset => "epoch_reset",
                },
                released_artifact_bytes: released_artifact_bytes.to_string(),
            },
        }
    }
}

#[derive(Serialize)]
struct ArtifactStateView<'a> {
    schema: &'static str,
    kind: &'static str,
    key: ProcessorKeyView<'a>,
    state: &'static str,
    artifact: Option<ArtifactView<'a>>,
    failure: Option<FailureView<'a>>,
}

#[derive(Serialize)]
struct ArtifactView<'a> {
    protocol: &'a str,
    media_type: &'a str,
    payload: ArtifactPayloadView<'a>,
}

impl<'a> From<&'a ProcessorArtifact> for ArtifactView<'a> {
    fn from(artifact: &'a ProcessorArtifact) -> Self {
        let payload = if let Some(citation) = artifact.as_citation() {
            ArtifactPayloadView::Citation {
                key: citation.key(),
                destination: citation.destination(),
                title: citation.title(),
            }
        } else if let Some(text) = artifact.as_text() {
            ArtifactPayloadView::Text { text }
        } else {
            ArtifactPayloadView::Binary {
                bytes: artifact.as_bytes().unwrap_or_default(),
            }
        };
        Self {
            protocol: artifact.protocol(),
            media_type: artifact.media_type(),
            payload,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum ArtifactPayloadView<'a> {
    Text {
        text: &'a str,
    },
    Binary {
        bytes: &'a [u8],
    },
    Citation {
        key: &'a str,
        destination: &'a str,
        title: Option<&'a str>,
    },
}

#[derive(Serialize)]
struct FailureView<'a> {
    code: &'static str,
    message: &'a str,
}

impl<'a> From<&'a ProcessorFailure> for FailureView<'a> {
    fn from(failure: &'a ProcessorFailure) -> Self {
        Self {
            code: failure.code().as_str(),
            message: failure.message(),
        }
    }
}
