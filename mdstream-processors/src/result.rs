use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::{
    CitationArtifact, ContentProcessor, IdentifierError, ProcessorRequest, ProcessorRequestKey,
    validate_identifier,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorFailureCode {
    Processor,
    Panic,
    InvalidRequest,
    Cancelled,
    UnsupportedContent,
    UnresolvedContext,
    InvalidContext,
    ResourceLimit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorFailure {
    code: ProcessorFailureCode,
    message: String,
}

impl ProcessorFailure {
    pub fn new(code: ProcessorFailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub const fn code(&self) -> ProcessorFailureCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArtifactPayload {
    Text(String),
    Binary(Vec<u8>),
    Citation(CitationArtifact),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorArtifact {
    protocol: String,
    media_type: String,
    payload: ArtifactPayload,
}

impl ProcessorArtifact {
    pub fn text(
        protocol: impl Into<String>,
        media_type: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<Self, IdentifierError> {
        Self::new(
            protocol.into(),
            media_type.into(),
            ArtifactPayload::Text(text.into()),
        )
    }

    pub fn binary(
        protocol: impl Into<String>,
        media_type: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, IdentifierError> {
        Self::new(
            protocol.into(),
            media_type.into(),
            ArtifactPayload::Binary(bytes.into()),
        )
    }

    fn new(
        protocol: String,
        media_type: String,
        payload: ArtifactPayload,
    ) -> Result<Self, IdentifierError> {
        validate_identifier("artifact.protocol", &protocol, true)?;
        validate_identifier("artifact.media_type", &media_type, true)?;
        Ok(Self {
            protocol,
            media_type,
            payload,
        })
    }

    pub(crate) fn citation(artifact: CitationArtifact) -> Self {
        Self {
            protocol: crate::CITATION_ARTIFACT_PROTOCOL.to_string(),
            media_type: "application/vnd.mdstream.citation".to_string(),
            payload: ArtifactPayload::Citation(artifact),
        }
    }

    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    pub fn as_text(&self) -> Option<&str> {
        match &self.payload {
            ArtifactPayload::Text(text) => Some(text),
            ArtifactPayload::Binary(_) | ArtifactPayload::Citation(_) => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &self.payload {
            ArtifactPayload::Text(text) => Some(text.as_bytes()),
            ArtifactPayload::Binary(bytes) => Some(bytes),
            ArtifactPayload::Citation(_) => None,
        }
    }

    pub fn as_citation(&self) -> Option<&CitationArtifact> {
        match &self.payload {
            ArtifactPayload::Citation(artifact) => Some(artifact),
            ArtifactPayload::Text(_) | ArtifactPayload::Binary(_) => None,
        }
    }

    pub fn byte_len(&self) -> usize {
        self.checked_byte_len().unwrap_or(usize::MAX)
    }

    /// Returns the deterministic logical artifact size, or `None` on overflow.
    pub fn checked_byte_len(&self) -> Option<usize> {
        let payload_bytes = match &self.payload {
            ArtifactPayload::Text(text) => text.len(),
            ArtifactPayload::Binary(bytes) => bytes.len(),
            ArtifactPayload::Citation(artifact) => artifact.checked_byte_len()?,
        };
        checked_artifact_len(self.protocol.len(), self.media_type.len(), payload_bytes)
    }
}

fn checked_artifact_len(
    protocol_bytes: usize,
    media_type_bytes: usize,
    payload_bytes: usize,
) -> Option<usize> {
    protocol_bytes
        .checked_add(media_type_bytes)?
        .checked_add(payload_bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorResult {
    key: ProcessorRequestKey,
    outcome: Result<ProcessorArtifact, ProcessorFailure>,
}

impl ProcessorResult {
    pub fn success(key: ProcessorRequestKey, artifact: ProcessorArtifact) -> Self {
        Self {
            key,
            outcome: Ok(artifact),
        }
    }

    pub fn failure(key: ProcessorRequestKey, failure: ProcessorFailure) -> Self {
        Self {
            key,
            outcome: Err(failure),
        }
    }

    pub fn key(&self) -> &ProcessorRequestKey {
        &self.key
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ProcessorRequestKey,
        Result<ProcessorArtifact, ProcessorFailure>,
    ) {
        (self.key, self.outcome)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionOutcome {
    Applied,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactReleaseReason {
    Replaced,
    Cancelled,
    NodeChanged,
    NodeRemoved,
    EpochReset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactChangeKind {
    Pending,
    Ready {
        artifact_bytes: usize,
    },
    Failed {
        code: ProcessorFailureCode,
    },
    Removed {
        reason: ArtifactReleaseReason,
        released_artifact_bytes: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactChange {
    key: ProcessorRequestKey,
    kind: ArtifactChangeKind,
}

impl ArtifactChange {
    pub(crate) fn new(key: ProcessorRequestKey, kind: ArtifactChangeKind) -> Self {
        Self { key, kind }
    }

    pub fn key(&self) -> &ProcessorRequestKey {
        &self.key
    }

    pub fn kind(&self) -> &ArtifactChangeKind {
        &self.kind
    }
}

/// Executes every processor-owned trait call behind an unwind boundary.
///
/// This contains panics only when compiled with `panic = "unwind"`. It is not
/// a sandbox and must run outside reducer, FFI, and artifact-host critical
/// sections.
pub fn run_catching(
    processor: &dyn ContentProcessor,
    request: &ProcessorRequest,
) -> ProcessorResult {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let descriptor = processor.descriptor();
        if descriptor.id() != request.key().slot().processor_id()
            || descriptor.version() != request.key().processor_version()
        {
            return Err(ProcessorFailure::new(
                ProcessorFailureCode::InvalidRequest,
                "processor descriptor does not match the request key",
            ));
        }
        if request.is_cancelled() {
            return Err(ProcessorFailure::new(
                ProcessorFailureCode::Cancelled,
                "processor request cancelled",
            ));
        }
        processor.process(request)
    }));
    match outcome {
        Ok(Ok(artifact)) => ProcessorResult::success(request.key().clone(), artifact),
        Ok(Err(failure)) => ProcessorResult::failure(request.key().clone(), failure),
        Err(_) => ProcessorResult::failure(
            request.key().clone(),
            ProcessorFailure::new(ProcessorFailureCode::Panic, "processor panicked"),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessorSlotState {
    Pending {
        key: ProcessorRequestKey,
    },
    Ready {
        key: ProcessorRequestKey,
        artifact: ProcessorArtifact,
    },
    Failed {
        key: ProcessorRequestKey,
        failure: ProcessorFailure,
    },
}

impl ProcessorSlotState {
    pub fn key(&self) -> &ProcessorRequestKey {
        match self {
            Self::Pending { key } | Self::Ready { key, .. } | Self::Failed { key, .. } => key,
        }
    }

    pub fn artifact(&self) -> Option<&ProcessorArtifact> {
        match self {
            Self::Ready { artifact, .. } => Some(artifact),
            Self::Pending { .. } | Self::Failed { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::checked_artifact_len;

    #[test]
    fn artifact_cost_reports_integer_overflow() {
        assert_eq!(checked_artifact_len(usize::MAX, 1, 0), None);
        assert_eq!(checked_artifact_len(1, 2, 3), Some(6));
    }
}
