use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ChunkSchedule, NormalizedSnapshot, ProtocolTrace, RequiredCheckpoint, TraceInputEvent,
    replay_protocol_trace,
};
use mdstream_protocol::ProjectionOp;

/// Current draft schema for checked-in conformance fixtures.
pub const FIXTURE_SCHEMA: &str = "mdstream.conformance/0.4-draft.2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub schema: String,
    pub id: String,
    pub description: String,
    pub source: String,
    pub dialect: Dialect,
    pub profile: CompatibilityProfile,
    pub provenance: Provenance,
    #[serde(default)]
    pub options: BTreeMap<String, Value>,
    pub schedules: Vec<NamedChunkSchedule>,
    #[serde(default)]
    pub traces: Vec<ProtocolTrace>,
    pub expected: FixtureExpectation,
    #[serde(default)]
    pub required_checkpoints: Vec<RequiredCheckpoint>,
}

impl Fixture {
    /// Validates envelope-level invariants that JSON Schema cannot express.
    pub fn validate(&self) -> Result<(), FixtureError> {
        if self.schema != FIXTURE_SCHEMA {
            return Err(FixtureError::UnsupportedSchema(self.schema.clone()));
        }
        validate_identifier("fixture.id", &self.id)?;
        if self.description.trim().is_empty() {
            return Err(FixtureError::InvalidField {
                field: "description",
                message: "must not be empty".to_string(),
            });
        }
        validate_identifier("dialect.id", &self.dialect.id)?;
        validate_identifier("profile.id", &self.profile.id)?;
        if self.profile.claim_scope.is_empty() {
            return Err(FixtureError::InvalidField {
                field: "profile.claim_scope",
                message: "must contain at least one explicit claim".to_string(),
            });
        }
        if self
            .profile
            .claim_scope
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != self.profile.claim_scope.len()
        {
            return Err(FixtureError::InvalidField {
                field: "profile.claim_scope",
                message: "must not contain duplicate claims".to_string(),
            });
        }
        self.provenance.validate()?;
        if self.schedules.is_empty() {
            return Err(FixtureError::InvalidField {
                field: "schedules",
                message: "must contain at least one schedule".to_string(),
            });
        }

        let mut schedule_ids = BTreeSet::new();
        for named in &self.schedules {
            validate_identifier("schedule.id", &named.id)?;
            if !schedule_ids.insert(named.id.as_str()) {
                return Err(FixtureError::DuplicateSchedule(named.id.clone()));
            }
            named
                .schedule
                .ranges(&self.source)
                .map_err(|error| FixtureError::InvalidSchedule {
                    schedule: named.id.clone(),
                    message: error.to_string(),
                })?;
        }

        let mut trace_ids = BTreeSet::new();
        let mut traced_schedules = BTreeSet::new();
        for trace in &self.traces {
            validate_identifier("trace.id", &trace.id)?;
            if !trace_ids.insert(trace.id.as_str()) {
                return Err(FixtureError::DuplicateTrace(trace.id.clone()));
            }
            if !schedule_ids.contains(trace.schedule.as_str()) {
                return Err(FixtureError::UnknownSchedule {
                    trace: trace.id.clone(),
                    schedule: trace.schedule.clone(),
                });
            }
            if !traced_schedules.insert(trace.schedule.as_str()) {
                return Err(FixtureError::InvalidField {
                    field: "traces.schedule",
                    message: format!(
                        "schedule `{}` must have exactly one canonical trace",
                        trace.schedule
                    ),
                });
            }
            let scheduled_chunks = self
                .schedule(&trace.schedule)
                .expect("known schedule was checked above")
                .slices(&self.source)
                .expect("schedule validity was checked above");
            if trace.setup_changes > trace.changes.len() {
                return Err(FixtureError::InvalidField {
                    field: "traces.setup_changes",
                    message: format!("trace `{}` setup boundary is out of range", trace.id),
                });
            }
            if trace.setup_changes > 0 {
                validate_setup_boundary(trace)?;
            }
            let mut change_end = trace.setup_changes;
            let mut append_chunks = Vec::new();
            let mut saw_reset = false;
            let mut saw_finish = false;
            for (event_index, event) in trace.input_events.iter().enumerate() {
                let next_end = event.change_end();
                if next_end < change_end || next_end > trace.changes.len() {
                    return Err(FixtureError::InvalidField {
                        field: "traces.input_events.change_end",
                        message: format!(
                            "trace `{}` event {event_index} has a non-monotonic or out-of-range change boundary",
                            trace.id
                        ),
                    });
                }
                let event_changes = &trace.changes[change_end..next_end];
                match event {
                    TraceInputEvent::Append { chunk, .. } => {
                        if saw_finish {
                            return Err(FixtureError::InvalidField {
                                field: "traces.input_events",
                                message: format!(
                                    "trace `{}` contains append after finish",
                                    trace.id
                                ),
                            });
                        }
                        if chunk.is_empty() && next_end != change_end {
                            return Err(FixtureError::InvalidField {
                                field: "traces.input_events",
                                message: format!(
                                    "trace `{}` empty append emitted a change",
                                    trace.id
                                ),
                            });
                        }
                        if event_changes.iter().any(change_finishes_document) {
                            return Err(FixtureError::InvalidField {
                                field: "traces.input_events",
                                message: format!(
                                    "trace `{}` append event {event_index} owns document finalization",
                                    trace.id
                                ),
                            });
                        }
                        if !chunk.is_empty() {
                            append_chunks.push(chunk.as_str());
                        }
                    }
                    TraceInputEvent::Reset { .. } => {
                        if saw_finish
                            || saw_reset
                            || !append_chunks.is_empty()
                            || (trace.setup_changes > 0 && event_index != 0)
                        {
                            return Err(FixtureError::InvalidField {
                                field: "traces.input_events",
                                message: format!(
                                    "trace `{}` reset must be the unique first target event",
                                    trace.id
                                ),
                            });
                        }
                        validate_reset_span(trace, change_end, next_end)?;
                        saw_reset = true;
                    }
                    TraceInputEvent::Finish { .. } => {
                        if saw_finish || event_index + 1 != trace.input_events.len() {
                            return Err(FixtureError::InvalidField {
                                field: "traces.input_events",
                                message: format!(
                                    "trace `{}` must contain exactly one final finish event",
                                    trace.id
                                ),
                            });
                        }
                        let finish_operations = event_changes
                            .iter()
                            .flat_map(|change| change.operations())
                            .filter(|operation| matches!(operation, ProjectionOp::FinishDocument))
                            .count();
                        if finish_operations != 1
                            || !event_changes.last().is_some_and(change_finishes_document)
                        {
                            return Err(FixtureError::InvalidField {
                                field: "traces.input_events",
                                message: format!(
                                    "trace `{}` finish event must finalize exactly once in its last change",
                                    trace.id
                                ),
                            });
                        }
                        saw_finish = true;
                    }
                }
                change_end = next_end;
            }
            if !saw_finish || change_end != trace.changes.len() {
                return Err(FixtureError::InvalidField {
                    field: "traces.input_events",
                    message: format!(
                        "trace `{}` events do not account for every change through finish",
                        trace.id
                    ),
                });
            }
            if trace.setup_changes > 0 && !saw_reset {
                return Err(FixtureError::InvalidField {
                    field: "traces.input_events",
                    message: format!("trace `{}` setup changes require a reset event", trace.id),
                });
            }
            if !scheduled_chunks.iter().copied().eq(append_chunks) {
                return Err(FixtureError::InvalidField {
                    field: "traces.input_events",
                    message: format!(
                        "trace `{}` non-empty append events do not match schedule `{}`",
                        trace.id, trace.schedule
                    ),
                });
            }
            if trace.changes.is_empty() {
                return Err(FixtureError::InvalidField {
                    field: "traces.changes",
                    message: format!("trace `{}` must contain at least one change", trace.id),
                });
            }
        }

        if self.expected.is_empty() {
            return Err(FixtureError::MissingExpectation);
        }
        self.validate_contract(&schedule_ids, &traced_schedules)?;

        let mut checkpoint_ids = BTreeSet::new();
        for checkpoint in &self.required_checkpoints {
            validate_identifier("required_checkpoints.id", &checkpoint.id)?;
            if !checkpoint_ids.insert(checkpoint.id.as_str()) {
                return Err(FixtureError::InvalidCheckpoint {
                    checkpoint: checkpoint.id.clone(),
                    message: "checkpoint ID is duplicated".to_string(),
                });
            }
            let Some(trace) = self
                .traces
                .iter()
                .find(|trace| trace.id == checkpoint.trace)
            else {
                return Err(FixtureError::InvalidCheckpoint {
                    checkpoint: checkpoint.id.clone(),
                    message: format!("unknown trace `{}`", checkpoint.trace),
                });
            };
            if checkpoint.after_change >= trace.changes.len() {
                return Err(FixtureError::InvalidCheckpoint {
                    checkpoint: checkpoint.id.clone(),
                    message: format!(
                        "after_change {} is outside trace length {}",
                        checkpoint.after_change,
                        trace.changes.len()
                    ),
                });
            }
        }

        Ok(())
    }

    fn validate_contract(
        &self,
        schedule_ids: &BTreeSet<&str>,
        traced_schedules: &BTreeSet<&str>,
    ) -> Result<(), FixtureError> {
        let claims = &self.profile.claim_scope;
        let has = |claim| claims.contains(&claim);
        let invalid = |message: &str| FixtureError::InvalidField {
            field: "fixture.contract",
            message: message.to_string(),
        };

        match self.provenance.oracle_kind() {
            OracleKind::CanonicalProtocol
                if self.traces.is_empty() || self.expected.normalized_snapshot.is_none() =>
            {
                return Err(invalid(
                    "canonical_protocol requires traces and normalized_snapshot",
                ));
            }
            OracleKind::ExactBlockSequence if self.expected.legacy_framing.is_none() => {
                return Err(invalid(
                    "exact_block_sequence requires a legacy_framing expectation",
                ));
            }
            OracleKind::ExactPendingProjection if self.expected.pending_projection.is_none() => {
                return Err(invalid(
                    "exact_pending_projection requires pending_projection",
                ));
            }
            OracleKind::PinnedFinalAst if self.expected.upstream_ast.is_none() => {
                return Err(invalid("pinned_final_ast requires upstream_ast"));
            }
            OracleKind::UpstreamPredicate
                if self
                    .expected
                    .upstream_predicates
                    .as_ref()
                    .is_none_or(Vec::is_empty) =>
            {
                return Err(invalid(
                    "upstream_predicate requires at least one upstream predicate",
                ));
            }
            _ => {}
        }

        if has(ClaimScope::ProtocolReplay) {
            if self.provenance.oracle_kind() != OracleKind::CanonicalProtocol
                || self.traces.is_empty()
                || self.expected.normalized_snapshot.is_none()
            {
                return Err(invalid(
                    "protocol_replay requires canonical_protocol traces and normalized_snapshot",
                ));
            }
            if traced_schedules != schedule_ids {
                return Err(invalid(
                    "protocol_replay requires exactly one trace for every declared schedule",
                ));
            }
        } else if !self.traces.is_empty() {
            return Err(invalid(
                "fixtures with protocol traces must declare protocol_replay",
            ));
        }

        if has(ClaimScope::LegacyBlockFraming)
            && self.expected.legacy_framing.is_none()
            && self.expected.upstream_predicates.is_none()
        {
            return Err(invalid(
                "legacy_block_framing requires a block sequence or upstream predicates",
            ));
        }
        if has(ClaimScope::PendingRepair) && self.expected.pending_projection.is_none() {
            return Err(invalid(
                "pending_repair requires an exact pending_projection",
            ));
        }
        if has(ClaimScope::FinalAstCharacterization)
            && self.expected.upstream_ast.is_none()
            && self.expected.upstream_predicates.is_none()
        {
            return Err(invalid(
                "final_ast_characterization requires an AST golden or upstream predicates",
            ));
        }
        if has(ClaimScope::LifecycleCharacterization)
            && self.required_checkpoints.is_empty()
            && self.expected.upstream_predicates.is_none()
        {
            return Err(invalid(
                "lifecycle_characterization requires checkpoints or upstream predicates",
            ));
        }
        Ok(())
    }

    pub fn schedule(&self, id: &str) -> Option<&ChunkSchedule> {
        self.schedules
            .iter()
            .find(|schedule| schedule.id == id)
            .map(|schedule| &schedule.schedule)
    }
}

fn change_finishes_document(change: &mdstream_protocol::ChangeSet) -> bool {
    change
        .operations()
        .iter()
        .any(|operation| matches!(operation, ProjectionOp::FinishDocument))
}

fn validate_setup_boundary(trace: &ProtocolTrace) -> Result<(), FixtureError> {
    if trace.setup_changes >= trace.changes.len() {
        return Err(FixtureError::InvalidField {
            field: "traces.setup_changes",
            message: format!(
                "trace `{}` setup must leave at least one target change",
                trace.id
            ),
        });
    }

    let setup_trace = ProtocolTrace {
        id: format!("{}:setup", trace.id),
        schedule: trace.schedule.clone(),
        setup_changes: 0,
        input_events: Vec::new(),
        changes: trace.changes[..trace.setup_changes].to_vec(),
    };
    let setup =
        replay_protocol_trace(&setup_trace).map_err(|error| FixtureError::InvalidField {
            field: "traces.setup_changes",
            message: format!("trace `{}` has invalid setup changes: {error}", trace.id),
        })?;
    let expected_predecessor = setup.final_snapshot.coordinate();
    let actual_predecessor = trace.changes[trace.setup_changes]
        .epoch_start()
        .and_then(|start| start.predecessor.as_ref());
    if actual_predecessor != Some(expected_predecessor) {
        return Err(FixtureError::InvalidField {
            field: "traces.setup_changes",
            message: format!(
                "trace `{}` target must start from the exact setup predecessor",
                trace.id
            ),
        });
    }
    Ok(())
}

fn validate_reset_span(
    trace: &ProtocolTrace,
    change_start: usize,
    change_end: usize,
) -> Result<(), FixtureError> {
    let changes = &trace.changes[change_start..change_end];
    if changes.len() != 1 {
        return Err(FixtureError::InvalidField {
            field: "traces.input_events",
            message: format!("trace `{}` reset must emit exactly one change", trace.id),
        });
    }
    let reset = &changes[0];
    let Some(epoch_start) = reset.epoch_start() else {
        return Err(FixtureError::InvalidField {
            field: "traces.input_events",
            message: format!("trace `{}` reset must emit an epoch start", trace.id),
        });
    };
    if !reset.source().suffix.is_empty() || !reset.operations().is_empty() {
        return Err(FixtureError::InvalidField {
            field: "traces.input_events",
            message: format!("trace `{}` reset epoch start must be empty", trace.id),
        });
    }

    let expected_predecessor = if change_start == 0 {
        None
    } else {
        let prefix = ProtocolTrace {
            id: format!("{}:before-reset", trace.id),
            schedule: trace.schedule.clone(),
            setup_changes: 0,
            input_events: Vec::new(),
            changes: trace.changes[..change_start].to_vec(),
        };
        let report =
            replay_protocol_trace(&prefix).map_err(|error| FixtureError::InvalidField {
                field: "traces.input_events",
                message: format!("trace `{}` has an invalid reset prefix: {error}", trace.id),
            })?;
        Some(report.final_snapshot.coordinate().clone())
    };
    if epoch_start.predecessor != expected_predecessor {
        return Err(FixtureError::InvalidField {
            field: "traces.input_events",
            message: format!("trace `{}` reset predecessor is not exact", trace.id),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialect {
    pub id: String,
    #[serde(default)]
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityProfile {
    pub id: String,
    pub claim_scope: Vec<ClaimScope>,
    #[serde(default)]
    pub pipeline: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimScope {
    ProtocolReplay,
    LegacyBlockFraming,
    PendingRepair,
    FinalAstCharacterization,
    LifecycleCharacterization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum Provenance {
    Synthetic {
        generator: String,
        oracle_kind: OracleKind,
    },
    Upstream {
        repository_url: String,
        commit_sha: String,
        package: String,
        package_version: String,
        license: String,
        upstream_path: String,
        upstream_test_name: String,
        oracle_kind: OracleKind,
        extraction: Extraction,
    },
}

impl Provenance {
    pub const fn oracle_kind(&self) -> OracleKind {
        match self {
            Self::Synthetic { oracle_kind, .. } | Self::Upstream { oracle_kind, .. } => {
                *oracle_kind
            }
        }
    }

    fn validate(&self) -> Result<(), FixtureError> {
        match self {
            Self::Synthetic {
                generator,
                oracle_kind: _,
            } => {
                if generator.trim().is_empty() {
                    return Err(FixtureError::InvalidProvenance(
                        "synthetic generator must not be empty".to_string(),
                    ));
                }
            }
            Self::Upstream {
                repository_url,
                commit_sha,
                package,
                package_version,
                license,
                upstream_path,
                upstream_test_name,
                oracle_kind: _,
                extraction: _,
            } => {
                if !repository_url.starts_with("https://") {
                    return Err(FixtureError::InvalidProvenance(
                        "repository_url must be an https URL".to_string(),
                    ));
                }
                if commit_sha.len() != 40
                    || !commit_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(FixtureError::InvalidProvenance(
                        "commit_sha must be a full 40-character hexadecimal revision".to_string(),
                    ));
                }
                for (field, value) in [
                    ("package", package),
                    ("package_version", package_version),
                    ("license", license),
                    ("upstream_path", upstream_path),
                    ("upstream_test_name", upstream_test_name),
                ] {
                    if value.trim().is_empty() {
                        return Err(FixtureError::InvalidProvenance(format!(
                            "{field} must not be empty"
                        )));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OracleKind {
    CanonicalProtocol,
    ExactBlockSequence,
    ExactPendingProjection,
    PinnedFinalAst,
    UpstreamPredicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum Extraction {
    Literal,
    PinnedGenerated,
    Transformed { steps: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedChunkSchedule {
    pub id: String,
    pub schedule: ChunkSchedule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct FixtureExpectation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_snapshot: Option<NormalizedSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_framing: Option<Vec<LegacyBlock>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_projection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_ast: Option<UpstreamAstExpectation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_predicates: Option<Vec<String>>,
}

impl FixtureExpectation {
    pub fn is_empty(&self) -> bool {
        self.normalized_snapshot.is_none()
            && self.legacy_framing.is_none()
            && self.pending_projection.is_none()
            && self.upstream_ast.is_none()
            && self.upstream_predicates.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyBlock {
    pub kind: String,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamAstExpectation {
    pub golden_path: String,
    pub normalization: Vec<String>,
    #[serde(default)]
    pub supplemental_indexes: Vec<String>,
}

pub fn load_fixture(path: impl AsRef<Path>) -> Result<Fixture, FixtureLoadError> {
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| FixtureLoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let fixture =
        serde_json::from_slice::<Fixture>(&bytes).map_err(|source| FixtureLoadError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    fixture
        .validate()
        .map_err(|source| FixtureLoadError::Invalid {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(fixture)
}

pub fn load_fixture_dir(path: impl AsRef<Path>) -> Result<Vec<Fixture>, FixtureLoadError> {
    let path = path.as_ref();
    let entries = fs::read_dir(path).map_err(|source| FixtureLoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut paths = entries
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|source| FixtureLoadError::Io {
                    path: path.to_path_buf(),
                    source,
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    paths.sort();
    paths.into_iter().map(load_fixture).collect()
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), FixtureError> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b'+')
        })
    {
        return Err(FixtureError::InvalidIdentifier {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureError {
    UnsupportedSchema(String),
    InvalidIdentifier {
        field: &'static str,
        value: String,
    },
    InvalidField {
        field: &'static str,
        message: String,
    },
    InvalidProvenance(String),
    DuplicateSchedule(String),
    DuplicateTrace(String),
    UnknownSchedule {
        trace: String,
        schedule: String,
    },
    InvalidSchedule {
        schedule: String,
        message: String,
    },
    MissingExpectation,
    InvalidCheckpoint {
        checkpoint: String,
        message: String,
    },
}

impl fmt::Display for FixtureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "unsupported fixture schema `{schema}`")
            }
            Self::InvalidIdentifier { field, value } => {
                write!(formatter, "{field} contains invalid identifier `{value}`")
            }
            Self::InvalidField { field, message } => write!(formatter, "{field} {message}"),
            Self::InvalidProvenance(message) => write!(formatter, "invalid provenance: {message}"),
            Self::DuplicateSchedule(schedule) => {
                write!(formatter, "duplicate chunk schedule `{schedule}`")
            }
            Self::DuplicateTrace(trace) => write!(formatter, "duplicate protocol trace `{trace}`"),
            Self::UnknownSchedule { trace, schedule } => {
                write!(
                    formatter,
                    "trace `{trace}` references unknown schedule `{schedule}`"
                )
            }
            Self::InvalidSchedule { schedule, message } => {
                write!(formatter, "invalid schedule `{schedule}`: {message}")
            }
            Self::MissingExpectation => formatter.write_str("fixture has no expectation oracle"),
            Self::InvalidCheckpoint {
                checkpoint,
                message,
            } => write!(formatter, "invalid checkpoint `{checkpoint}`: {message}"),
        }
    }
}

impl std::error::Error for FixtureError {}

#[derive(Debug)]
pub enum FixtureLoadError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    Invalid {
        path: PathBuf,
        source: FixtureError,
    },
}

impl fmt::Display for FixtureLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::Json { path, source } => {
                write!(formatter, "failed to decode {}: {source}", path.display())
            }
            Self::Invalid { path, source } => {
                write!(formatter, "invalid fixture {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for FixtureLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::Invalid { source, .. } => Some(source),
        }
    }
}
