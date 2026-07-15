use std::fmt;

use mdstream_protocol::{Epoch, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierErrorKind {
    Empty,
    TooLong,
    InvalidCharacter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierError {
    field: &'static str,
    kind: IdentifierErrorKind,
}

impl IdentifierError {
    pub(crate) const fn new(field: &'static str, kind: IdentifierErrorKind) -> Self {
        Self { field, kind }
    }

    pub const fn field(&self) -> &'static str {
        self.field
    }

    pub const fn kind(&self) -> IdentifierErrorKind {
        self.kind
    }
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            IdentifierErrorKind::Empty => "cannot be empty",
            IdentifierErrorKind::TooLong => "cannot exceed 128 bytes",
            IdentifierErrorKind::InvalidCharacter => "contains an unsupported character",
        };
        write!(formatter, "{} {reason}", self.field)
    }
}

impl std::error::Error for IdentifierError {}

pub(crate) fn validate_identifier(
    field: &'static str,
    value: &str,
    allow_slash: bool,
) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::new(field, IdentifierErrorKind::Empty));
    }
    if value.len() > 128 {
        return Err(IdentifierError::new(field, IdentifierErrorKind::TooLong));
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'.' | b'_' | b':' | b'-' | b'+')
            || (allow_slash && byte == b'/')
    }) {
        return Err(IdentifierError::new(
            field,
            IdentifierErrorKind::InvalidCharacter,
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    EpochNotInitialized,
    EpochRegression {
        current: Epoch,
        requested: Epoch,
    },
    EpochMismatch {
        current: Epoch,
        received: Epoch,
    },
    NodeNotFound(NodeId),
    InvalidBodyRange(NodeId),
    ProvisionalProcessingDisabled(NodeId),
    LimitExceeded {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    RequestGenerationExhausted,
    CounterOverflow(&'static str),
}

impl HostError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::EpochNotInitialized => "epoch_not_initialized",
            Self::EpochRegression { .. } => "epoch_regression",
            Self::EpochMismatch { .. } => "epoch_mismatch",
            Self::NodeNotFound(_) => "node_not_found",
            Self::InvalidBodyRange(_) => "invalid_body_range",
            Self::ProvisionalProcessingDisabled(_) => "provisional_processing_disabled",
            Self::LimitExceeded { .. } => "resource_limit_exceeded",
            Self::RequestGenerationExhausted => "request_generation_exhausted",
            Self::CounterOverflow(_) => "counter_overflow",
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EpochNotInitialized => formatter.write_str("processor epoch is not initialized"),
            Self::EpochRegression { current, requested } => {
                write!(
                    formatter,
                    "processor epoch {requested} precedes current epoch {current}"
                )
            }
            Self::EpochMismatch { current, received } => {
                write!(
                    formatter,
                    "processor epoch {received} does not match {current}"
                )
            }
            Self::NodeNotFound(node_id) => {
                write!(formatter, "processor node {node_id} was not found")
            }
            Self::InvalidBodyRange(node_id) => {
                write!(
                    formatter,
                    "processor node {node_id} has an invalid body range"
                )
            }
            Self::ProvisionalProcessingDisabled(node_id) => write!(
                formatter,
                "processor node {node_id} is provisional without explicit capability and policy"
            ),
            Self::LimitExceeded {
                field,
                limit,
                actual,
            } => write!(formatter, "{field} limit {limit} exceeded by {actual}"),
            Self::RequestGenerationExhausted => {
                formatter.write_str("processor request generation exhausted")
            }
            Self::CounterOverflow(field) => write!(formatter, "{field} counter overflowed"),
        }
    }
}

impl std::error::Error for HostError {}
