use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Epoch, NodeId, ResourceId, Sequence, SourceCursor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Stable machine-readable classification for [`ProtocolError`].
pub enum ProtocolErrorCode {
    UnsupportedSchema,
    InvalidChange,
    InvalidSnapshot,
    InvalidRange,
    CursorOverflow,
    MetadataOverflow,
    SequenceOverflow,
    SourceTooLarge,
    TooManyNodes,
    TooManyOperations,
    ValueTooLarge,
    MissingNode,
    MissingResource,
    DuplicateNode,
    DuplicateResource,
    ReusedNodeId,
    ReusedResourceId,
    VersionMismatch,
    ResourceVersionMismatch,
    IllegalLifecycle,
    NeedsSnapshot,
    SnapshotNotAllowed,
    InvalidEpochStart,
    StaleSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Typed failure returned by protocol validation and canonical reduction.
pub enum ProtocolError {
    UnsupportedSchema(String),
    InvalidChange(String),
    InvalidSnapshot(String),
    InvalidRange {
        start: SourceCursor,
        end: SourceCursor,
    },
    CursorOverflow,
    MetadataOverflow,
    SequenceOverflow,
    SourceTooLarge {
        limit: usize,
        actual: usize,
    },
    TooManyNodes {
        limit: usize,
        actual: usize,
    },
    TooManyOperations {
        limit: usize,
        actual: usize,
    },
    ValueTooLarge {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    MissingNode(NodeId),
    MissingResource(ResourceId),
    DuplicateNode(NodeId),
    DuplicateResource(ResourceId),
    ReusedNodeId(NodeId),
    ReusedResourceId(ResourceId),
    VersionMismatch(NodeId),
    ResourceVersionMismatch(ResourceId),
    IllegalLifecycle(String),
    NeedsSnapshot,
    SnapshotNotAllowed,
    InvalidEpochStart {
        current: Option<Epoch>,
        received: Epoch,
    },
    StaleSnapshot {
        floor: Sequence,
        received: Sequence,
    },
}

impl ProtocolError {
    pub const fn code(&self) -> ProtocolErrorCode {
        match self {
            Self::UnsupportedSchema(_) => ProtocolErrorCode::UnsupportedSchema,
            Self::InvalidChange(_) => ProtocolErrorCode::InvalidChange,
            Self::InvalidSnapshot(_) => ProtocolErrorCode::InvalidSnapshot,
            Self::InvalidRange { .. } => ProtocolErrorCode::InvalidRange,
            Self::CursorOverflow => ProtocolErrorCode::CursorOverflow,
            Self::MetadataOverflow => ProtocolErrorCode::MetadataOverflow,
            Self::SequenceOverflow => ProtocolErrorCode::SequenceOverflow,
            Self::SourceTooLarge { .. } => ProtocolErrorCode::SourceTooLarge,
            Self::TooManyNodes { .. } => ProtocolErrorCode::TooManyNodes,
            Self::TooManyOperations { .. } => ProtocolErrorCode::TooManyOperations,
            Self::ValueTooLarge { .. } => ProtocolErrorCode::ValueTooLarge,
            Self::MissingNode(_) => ProtocolErrorCode::MissingNode,
            Self::MissingResource(_) => ProtocolErrorCode::MissingResource,
            Self::DuplicateNode(_) => ProtocolErrorCode::DuplicateNode,
            Self::DuplicateResource(_) => ProtocolErrorCode::DuplicateResource,
            Self::ReusedNodeId(_) => ProtocolErrorCode::ReusedNodeId,
            Self::ReusedResourceId(_) => ProtocolErrorCode::ReusedResourceId,
            Self::VersionMismatch(_) => ProtocolErrorCode::VersionMismatch,
            Self::ResourceVersionMismatch(_) => ProtocolErrorCode::ResourceVersionMismatch,
            Self::IllegalLifecycle(_) => ProtocolErrorCode::IllegalLifecycle,
            Self::NeedsSnapshot => ProtocolErrorCode::NeedsSnapshot,
            Self::SnapshotNotAllowed => ProtocolErrorCode::SnapshotNotAllowed,
            Self::InvalidEpochStart { .. } => ProtocolErrorCode::InvalidEpochStart,
            Self::StaleSnapshot { .. } => ProtocolErrorCode::StaleSnapshot,
        }
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(value) => write!(formatter, "unsupported schema: {value}"),
            Self::InvalidChange(message) => write!(formatter, "invalid change set: {message}"),
            Self::InvalidSnapshot(message) => write!(formatter, "invalid snapshot: {message}"),
            Self::InvalidRange { start, end } => {
                write!(formatter, "invalid source range {start}..{end}")
            }
            Self::CursorOverflow => formatter.write_str("source cursor overflow"),
            Self::MetadataOverflow => formatter.write_str("metadata byte accounting overflow"),
            Self::SequenceOverflow => formatter.write_str("sequence overflow"),
            Self::SourceTooLarge { limit, actual } => {
                write!(formatter, "source uses {actual} bytes, limit is {limit}")
            }
            Self::TooManyNodes { limit, actual } => {
                write!(formatter, "document has {actual} nodes, limit is {limit}")
            }
            Self::TooManyOperations { limit, actual } => {
                write!(
                    formatter,
                    "change has {actual} operations, limit is {limit}"
                )
            }
            Self::ValueTooLarge {
                field,
                limit,
                actual,
            } => write!(formatter, "{field} uses {actual} bytes, limit is {limit}"),
            Self::MissingNode(id) => write!(formatter, "node {id} does not exist"),
            Self::MissingResource(id) => write!(formatter, "resource {id} does not exist"),
            Self::DuplicateNode(id) => write!(formatter, "node {id} appears more than once"),
            Self::DuplicateResource(id) => {
                write!(formatter, "resource {id} appears more than once")
            }
            Self::ReusedNodeId(id) => {
                write!(formatter, "node {id} is below the allocation high-water")
            }
            Self::ReusedResourceId(id) => {
                write!(
                    formatter,
                    "resource {id} is below the allocation high-water"
                )
            }
            Self::VersionMismatch(id) => write!(formatter, "node {id} version does not match"),
            Self::ResourceVersionMismatch(id) => {
                write!(formatter, "resource {id} version does not match")
            }
            Self::IllegalLifecycle(message) => {
                write!(formatter, "illegal lifecycle transition: {message}")
            }
            Self::NeedsSnapshot => formatter.write_str("reducer requires snapshot recovery"),
            Self::SnapshotNotAllowed => formatter
                .write_str("snapshot replacement is only allowed during bootstrap or recovery"),
            Self::InvalidEpochStart { current, received } => match current {
                Some(current) => write!(
                    formatter,
                    "epoch start {received} does not legally follow current epoch {current}"
                ),
                None => write!(formatter, "epoch start {received} is not a valid bootstrap"),
            },
            Self::StaleSnapshot { floor, received } => write!(
                formatter,
                "snapshot sequence {received} is below recovery floor {floor}"
            ),
        }
    }
}

impl std::error::Error for ProtocolError {}
