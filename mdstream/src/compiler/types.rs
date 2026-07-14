use std::fmt;

use mdstream_protocol::{NodeId, ResourceId, SourceCursor};

use super::{
    checkpoints,
    identity::IdentityError,
    markdown::{MarkdownDiagnostic, MarkdownError},
    operations::OperationLimitError,
    reconcile::ReconcileError,
};
use crate::syntax::containers::parse_tag_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompilerMetrics {
    pub structural_source_bytes: u64,
    pub deferred_source_bytes: u64,
    pub parse_passes: u64,
    pub parsed_source_bytes: u64,
    pub custom_scan_source_bytes: u64,
    pub reconcile_node_visits: u64,
    pub reconcile_structure_owners: u64,
    pub reconcile_structure_id_comparisons: u64,
    pub reconcile_structure_version_steps: u64,
    pub reconcile_structure_ids_emitted: u64,
    pub reconcile_resource_visits: u64,
    pub incremental_projection_visits: u64,
    pub semantic_definition_visits: u64,
    pub semantic_state_key_visits: u64,
    pub semantic_state_edge_visits: u64,
    pub semantic_candidate_node_visits: u64,
    pub semantic_candidate_dependency_visits: u64,
    pub semantic_dependent_visits: u64,
    pub semantic_corrections_emitted: u64,
    pub retained_semantic_definitions: usize,
    pub retained_semantic_dependencies: usize,
    pub retained_semantic_metadata_bytes: usize,
    pub frontier_bytes: usize,
    pub next_checkpoint: usize,
}

impl Default for CompilerMetrics {
    fn default() -> Self {
        Self {
            structural_source_bytes: 0,
            deferred_source_bytes: 0,
            parse_passes: 0,
            parsed_source_bytes: 0,
            custom_scan_source_bytes: 0,
            reconcile_node_visits: 0,
            reconcile_structure_owners: 0,
            reconcile_structure_id_comparisons: 0,
            reconcile_structure_version_steps: 0,
            reconcile_structure_ids_emitted: 0,
            reconcile_resource_visits: 0,
            incremental_projection_visits: 0,
            semantic_definition_visits: 0,
            semantic_state_key_visits: 0,
            semantic_state_edge_visits: 0,
            semantic_candidate_node_visits: 0,
            semantic_candidate_dependency_visits: 0,
            semantic_dependent_visits: 0,
            semantic_corrections_emitted: 0,
            retained_semantic_definitions: 0,
            retained_semantic_dependencies: 0,
            retained_semantic_metadata_bytes: 0,
            frontier_bytes: 0,
            next_checkpoint: checkpoints::INITIAL_CHECKPOINT,
        }
    }
}

/// Static configuration for a paired standalone block represented as a
/// canonical custom node.
///
/// Open and close delimiters must occupy an unindented physical line. An
/// opening delimiter is recognized only at the start of the document or after
/// a blank line; a closing delimiter must match the current custom stack top.
/// Chunk boundaries never complete a delimiter line: an EOF delimiter remains
/// provisional until a line ending or `finish` confirms it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomBlockSpec {
    namespace: String,
    name: String,
    opaque: bool,
    case_insensitive: bool,
}

impl CustomBlockSpec {
    pub fn try_new(
        namespace: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, CompilerError> {
        let namespace = namespace.into();
        let name = name.into();
        let valid_namespace = !namespace.is_empty()
            && namespace
                .chars()
                .all(|character| !character.is_control() && !character.is_whitespace());
        let valid_name = parse_tag_name(&name).is_some_and(|(_, remaining)| remaining.is_empty());
        if !valid_namespace || !valid_name {
            return Err(CompilerError::InvalidConfiguration(
                "custom blocks require a non-empty namespace without whitespace or control characters and a valid HTML tag name".to_string(),
            ));
        }
        Ok(Self {
            namespace,
            name,
            opaque: true,
            case_insensitive: true,
        })
    }

    pub fn opaque(mut self, opaque: bool) -> Self {
        self.opaque = opaque;
        self
    }

    pub fn case_insensitive(mut self, case_insensitive: bool) -> Self {
        self.case_insensitive = case_insensitive;
        self
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn is_opaque(&self) -> bool {
        self.opaque
    }

    pub const fn is_case_insensitive(&self) -> bool {
        self.case_insensitive
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompilerError {
    CursorOverflow,
    InvalidSourceBoundary(SourceCursor),
    InvalidConfiguration(String),
    LimitExceeded {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    Markdown(MarkdownDiagnostic),
    NodeIdentityCollision(NodeId),
    ResourceIdentityCollision(ResourceId),
    InvalidIdentity(String),
    InvalidReconciliation(String),
    MetricsOverflow(&'static str),
}

impl fmt::Display for CompilerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CursorOverflow => formatter.write_str("compiler source cursor overflow"),
            Self::InvalidSourceBoundary(cursor) => {
                write!(
                    formatter,
                    "compiler frontier {cursor} is not a source boundary"
                )
            }
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid compiler configuration: {message}")
            }
            Self::LimitExceeded {
                field,
                limit,
                actual,
            } => write!(
                formatter,
                "compiler {field} {actual} exceeds the configured limit of {limit}"
            ),
            Self::Markdown(diagnostic) => {
                write!(formatter, "Markdown compilation failed: {diagnostic}")
            }
            Self::NodeIdentityCollision(id) => {
                write!(formatter, "node identity collision for {id}")
            }
            Self::ResourceIdentityCollision(id) => {
                write!(formatter, "resource identity collision for {id}")
            }
            Self::InvalidIdentity(message) => {
                write!(formatter, "invalid content identity: {message}")
            }
            Self::InvalidReconciliation(message) => {
                write!(formatter, "content reconciliation failed: {message}")
            }
            Self::MetricsOverflow(field) => write!(formatter, "compiler metric {field} overflowed"),
        }
    }
}

impl std::error::Error for CompilerError {}

impl From<MarkdownError> for CompilerError {
    fn from(error: MarkdownError) -> Self {
        match error {
            MarkdownError::LimitExceeded {
                field,
                limit,
                actual,
            } => Self::LimitExceeded {
                field,
                limit,
                actual,
            },
            error => Self::Markdown(error),
        }
    }
}

impl From<IdentityError> for CompilerError {
    fn from(error: IdentityError) -> Self {
        match error {
            IdentityError::NodeCollision(id) | IdentityError::DuplicateLiveNode(id) => {
                Self::NodeIdentityCollision(id)
            }
            IdentityError::ResourceCollision(id) | IdentityError::ResourceConflict(id) => {
                Self::ResourceIdentityCollision(id)
            }
            error => Self::InvalidIdentity(error.to_string()),
        }
    }
}

impl From<ReconcileError> for CompilerError {
    fn from(error: ReconcileError) -> Self {
        match error {
            ReconcileError::OperationLimit(error) => error.into(),
            error => Self::InvalidReconciliation(error.to_string()),
        }
    }
}

impl From<OperationLimitError> for CompilerError {
    fn from(error: OperationLimitError) -> Self {
        Self::LimitExceeded {
            field: error.field,
            limit: error.limit,
            actual: error.actual,
        }
    }
}
