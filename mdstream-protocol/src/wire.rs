use std::{
    fmt,
    io::{self, Write},
};

use serde::{Deserialize, Serialize};

use crate::{
    ChildList, ContentNode, Coordinate, DocumentLifecycle, NodeId, PayloadDigest, ProtocolError,
    ProtocolLimits, ResourceId, SemanticResource, SnapshotDigest,
};

pub const PROTOCOL_SCHEMA: &str = "mdstream.content/0.4-draft.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SchemaVersion(String);

impl SchemaVersion {
    pub fn current() -> Self {
        Self(PROTOCOL_SCHEMA.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn ensure_supported(&self) -> Result<(), ProtocolError> {
        if self.0 == PROTOCOL_SCHEMA {
            Ok(())
        } else {
            Err(ProtocolError::UnsupportedSchema(self.0.clone()))
        }
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolMaturity {
    Draft,
    Candidate,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// A complete, integrity-checked canonical reducer checkpoint.
///
/// The digest covers every field except itself. It detects corruption but is
/// not an authentication mechanism; transports that cross trust boundaries
/// must provide their own authenticity guarantees.
pub struct Snapshot {
    schema: SchemaVersion,
    maturity: ProtocolMaturity,
    digest: SnapshotDigest,
    coordinate: Coordinate,
    last_payload_digest: PayloadDigest,
    lifecycle: DocumentLifecycle,
    source: String,
    roots: ChildList,
    nodes: Vec<ContentNode>,
    resources: Vec<SemanticResource>,
    next_node_id: NodeId,
    next_resource_id: ResourceId,
}

impl Snapshot {
    pub fn schema(&self) -> &SchemaVersion {
        &self.schema
    }

    pub const fn maturity(&self) -> ProtocolMaturity {
        self.maturity
    }

    pub fn digest(&self) -> &SnapshotDigest {
        &self.digest
    }

    pub fn coordinate(&self) -> &Coordinate {
        &self.coordinate
    }

    pub fn last_payload_digest(&self) -> &PayloadDigest {
        &self.last_payload_digest
    }

    pub const fn lifecycle(&self) -> DocumentLifecycle {
        self.lifecycle
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn roots(&self) -> &ChildList {
        &self.roots
    }

    pub fn nodes(&self) -> &[ContentNode] {
        &self.nodes
    }

    pub fn resources(&self) -> &[SemanticResource] {
        &self.resources
    }

    pub const fn next_node_id(&self) -> NodeId {
        self.next_node_id
    }

    pub const fn next_resource_id(&self) -> ResourceId {
        self.next_resource_id
    }

    /// Recomputes the digest over the canonical snapshot contents.
    pub fn derived_digest(&self) -> SnapshotDigest {
        derive_snapshot_digest(SnapshotDigestView {
            schema: &self.schema,
            maturity: self.maturity,
            coordinate: &self.coordinate,
            last_payload_digest: &self.last_payload_digest,
            lifecycle: self.lifecycle,
            source: &self.source,
            roots: &self.roots,
            nodes: &self.nodes,
            resources: &self.resources,
            next_node_id: self.next_node_id,
            next_resource_id: self.next_resource_id,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_canonical_parts(
        coordinate: Coordinate,
        last_payload_digest: PayloadDigest,
        lifecycle: DocumentLifecycle,
        source: String,
        roots: ChildList,
        nodes: Vec<ContentNode>,
        resources: Vec<SemanticResource>,
        next_node_id: NodeId,
        next_resource_id: ResourceId,
    ) -> Self {
        let schema = SchemaVersion::current();
        let maturity = ProtocolMaturity::Draft;
        let digest = derive_snapshot_digest(SnapshotDigestView {
            schema: &schema,
            maturity,
            coordinate: &coordinate,
            last_payload_digest: &last_payload_digest,
            lifecycle,
            source: &source,
            roots: &roots,
            nodes: &nodes,
            resources: &resources,
            next_node_id,
            next_resource_id,
        });
        Self {
            schema,
            maturity,
            digest,
            coordinate,
            last_payload_digest,
            lifecycle,
            source,
            roots,
            nodes,
            resources,
            next_node_id,
            next_resource_id,
        }
    }
}

#[derive(Serialize)]
struct SnapshotDigestView<'a> {
    schema: &'a SchemaVersion,
    maturity: ProtocolMaturity,
    coordinate: &'a Coordinate,
    last_payload_digest: &'a PayloadDigest,
    lifecycle: DocumentLifecycle,
    source: &'a str,
    roots: &'a ChildList,
    nodes: &'a [ContentNode],
    resources: &'a [SemanticResource],
    next_node_id: NodeId,
    next_resource_id: ResourceId,
}

fn derive_snapshot_digest(view: SnapshotDigestView<'_>) -> SnapshotDigest {
    SnapshotDigest::digest_json(&view)
}

/// Validates and encodes a change without retaining more than `limit + 1`
/// output bytes when the encoded-size budget is exceeded.
pub fn encode_change_json(
    value: &crate::ChangeSet,
    max_encoded_bytes: usize,
    limits: ProtocolLimits,
) -> Result<Vec<u8>, ProtocolError> {
    value.validate_complete(limits)?;
    encode_bounded(value, max_encoded_bytes, "encoded_change")
}

/// Decodes and validates a draft change from canonical JSON.
pub fn decode_change_json(
    bytes: &[u8],
    max_encoded_bytes: usize,
    limits: ProtocolLimits,
) -> Result<crate::ChangeSet, ProtocolError> {
    check_encoded_size(bytes, max_encoded_bytes, "encoded_change")?;
    let change: crate::ChangeSet = serde_json::from_slice(bytes)
        .map_err(|error| ProtocolError::InvalidChange(error.to_string()))?;
    change.validate_complete(limits)?;
    Ok(change)
}

/// Validates and bounded-encodes a canonical snapshot as JSON.
pub fn encode_snapshot_json(
    value: &Snapshot,
    max_encoded_bytes: usize,
    limits: ProtocolLimits,
) -> Result<Vec<u8>, ProtocolError> {
    crate::validate_snapshot(value, limits)?;
    encode_bounded(value, max_encoded_bytes, "encoded_snapshot")
}

/// Decodes a snapshot and verifies its digest and canonical invariants.
pub fn decode_snapshot_json(
    bytes: &[u8],
    max_encoded_bytes: usize,
    limits: ProtocolLimits,
) -> Result<Snapshot, ProtocolError> {
    check_encoded_size(bytes, max_encoded_bytes, "encoded_snapshot")?;
    let snapshot: Snapshot = serde_json::from_slice(bytes)
        .map_err(|error| ProtocolError::InvalidSnapshot(error.to_string()))?;
    crate::validate_snapshot(&snapshot, limits)?;
    Ok(snapshot)
}

fn encode_bounded<T: Serialize>(
    value: &T,
    max_encoded_bytes: usize,
    field: &'static str,
) -> Result<Vec<u8>, ProtocolError> {
    let mut writer = BoundedWriter::new(max_encoded_bytes);
    let result = serde_json::to_writer(&mut writer, value);
    if writer.bytes.len() > max_encoded_bytes {
        return Err(ProtocolError::ValueTooLarge {
            field,
            limit: max_encoded_bytes,
            actual: writer.bytes.len(),
        });
    }
    result.map_err(|error| ProtocolError::InvalidChange(error.to_string()))?;
    Ok(writer.bytes)
}

struct BoundedWriter {
    bytes: Vec<u8>,
    max_retained: usize,
}

impl BoundedWriter {
    fn new(max_encoded_bytes: usize) -> Self {
        let max_retained = max_encoded_bytes.saturating_add(1);
        Self {
            bytes: Vec::with_capacity(max_retained.min(8 * 1024)),
            max_retained,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let remaining = self.max_retained.saturating_sub(self.bytes.len());
        if remaining == 0 {
            return Err(io::Error::other("encoded output exceeds configured limit"));
        }
        let written = remaining.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn check_encoded_size(
    bytes: &[u8],
    max_encoded_bytes: usize,
    field: &'static str,
) -> Result<(), ProtocolError> {
    if bytes.len() > max_encoded_bytes {
        Err(ProtocolError::ValueTooLarge {
            field,
            limit: max_encoded_bytes,
            actual: bytes.len(),
        })
    } else {
        Ok(())
    }
}
