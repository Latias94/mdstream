use std::{
    fmt,
    io::{self, Write},
};

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    ChildList, ContentNode, Coordinate, DocumentLifecycle, PayloadDigest, ProtocolError,
    ProtocolLimits, SemanticResource, SnapshotDigest,
};

pub const PROTOCOL_SCHEMA: &str = "mdstream.content/0.4-candidate.1";

/// Deserializes a nullable field while keeping its presence mandatory.
pub(crate) fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

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
    projection_cursor: crate::SourceCursor,
    roots: ChildList,
    nodes: Vec<ContentNode>,
    resources: Vec<SemanticResource>,
}

pub(crate) struct CanonicalSnapshotParts {
    pub coordinate: Coordinate,
    pub last_payload_digest: PayloadDigest,
    pub lifecycle: DocumentLifecycle,
    pub source: String,
    pub projection_cursor: crate::SourceCursor,
    pub roots: ChildList,
    pub nodes: Vec<ContentNode>,
    pub resources: Vec<SemanticResource>,
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

    /// Returns the exclusive source frontier represented by the canonical projection.
    pub const fn projection_cursor(&self) -> crate::SourceCursor {
        self.projection_cursor
    }

    /// Returns the canonical source range not yet represented by the projection.
    ///
    /// Direct Serde deserialization does not validate a snapshot. This accessor
    /// therefore verifies the source cursors before exposing them as a range.
    pub fn pending_source_range(&self) -> Result<crate::SourceRange, ProtocolError> {
        self.validated_pending_source_range()
    }

    /// Returns the canonical source suffix not yet represented by the projection.
    pub fn pending_source(&self) -> Result<&str, ProtocolError> {
        let range = self.validated_pending_source_range()?;
        let start = usize::try_from(range.start.get()).map_err(|_| {
            ProtocolError::InvalidSnapshot(
                "projection cursor exceeds the source address space".to_string(),
            )
        })?;
        Ok(&self.source[start..])
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

    fn validated_pending_source_range(&self) -> Result<crate::SourceRange, ProtocolError> {
        let source_len = u64::try_from(self.source.len()).map_err(|_| {
            ProtocolError::InvalidSnapshot(
                "source length exceeds the protocol cursor address space".to_string(),
            )
        })?;
        if self.coordinate.source_cursor.get() != source_len {
            return Err(ProtocolError::InvalidSnapshot(
                "source cursor does not match source length".to_string(),
            ));
        }

        let range = crate::SourceRange::new(self.projection_cursor, self.coordinate.source_cursor);
        range.validate(&self.source).map_err(|_| {
            ProtocolError::InvalidSnapshot(
                "pending source range must be ordered, bounded, and use canonical UTF-8 boundaries"
                    .to_string(),
            )
        })?;
        Ok(range)
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
            projection_cursor: self.projection_cursor,
            roots: &self.roots,
            nodes: &self.nodes,
            resources: &self.resources,
        })
    }

    pub(crate) fn from_canonical_parts(parts: CanonicalSnapshotParts) -> Self {
        let CanonicalSnapshotParts {
            coordinate,
            last_payload_digest,
            lifecycle,
            source,
            projection_cursor,
            roots,
            nodes,
            resources,
        } = parts;
        let schema = SchemaVersion::current();
        let maturity = ProtocolMaturity::Candidate;
        let digest = derive_snapshot_digest(SnapshotDigestView {
            schema: &schema,
            maturity,
            coordinate: &coordinate,
            last_payload_digest: &last_payload_digest,
            lifecycle,
            source: &source,
            projection_cursor,
            roots: &roots,
            nodes: &nodes,
            resources: &resources,
        });
        Self {
            schema,
            maturity,
            digest,
            coordinate,
            last_payload_digest,
            lifecycle,
            source,
            projection_cursor,
            roots,
            nodes,
            resources,
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
    projection_cursor: crate::SourceCursor,
    roots: &'a ChildList,
    nodes: &'a [ContentNode],
    resources: &'a [SemanticResource],
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

/// Decodes and validates a binding-candidate change from canonical JSON.
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
