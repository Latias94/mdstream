use std::{
    collections::BTreeMap,
    fmt, fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Deserializer, Serialize};

pub const BUDGET_SCHEMA: &str = "mdstream.budgets/1";
/// Frozen engine/protocol commit used by U7 calibration replay.
///
/// Advance this together with the Schema and checked-in budget provenance only
/// after the final core hot-path change is committed, then regenerate and
/// re-freeze the deterministic measurements below.
pub const U7_BASELINE_COMMIT: &str = "475c3b760a95d8845e7bbd9592f810e56c3a11a9";
pub const CALIBRATION_PROFILE: &str = "release";
pub const CALIBRATION_COMMAND: &str = "scripts/calibrate-budgets.sh";
pub const CALIBRATION_SCHEDULE: &str = "characters";

pub const CALIBRATION_FIXTURE_ID: &str = "streamdown-bench.mixed-content-realistic";
pub const CALIBRATION_FIXTURE_PATH: &str =
    "mdstream/tests/fixtures/streamdown_bench/mixed_content_realistic.md";
const CALIBRATION_FIXTURE_SHA256: &str =
    "e8a7bcfd218ecc7b621db414ff3585210d5047565705a11d2d580590066a90b0";
const CALIBRATION_FIXTURE_BYTES: u64 = 682;
const CALIBRATION_FIXTURE_CHUNKS: u64 = 676;

const FROZEN_LEGACY_COUNTS: LegacyCounts = LegacyCounts {
    append_calls: 676,
    update_count: 677,
    committed_blocks_emitted: 13,
    pending_observations: 669,
    reset_count: 0,
    invalidated_block_ids: 0,
    observed_text_bytes: 33_253,
    final_block_count: 13,
    retained_buffer_bytes: 682,
};

const FROZEN_MINIMAL_COUNTS: MinimalTransportCounts = MinimalTransportCounts {
    chunk_count: 676,
    change_count: 677,
    operation_count: 2,
    applied_changes: 677,
    operations_visited: 2,
    nodes_validated: 0,
    relationship_steps: 0,
    child_ids_copied: 0,
    snapshots_validated: 0,
};

const FROZEN_WIRE_MEASUREMENTS: WireMeasurements = WireMeasurements {
    encoded_change_bytes: 142_015,
    encoded_snapshot_bytes: 1_274,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamingBudget {
    pub schema: String,
    pub kind: String,
    pub provenance: CalibrationProvenance,
    pub legacy_0_3: LegacyCalibration,
    pub minimal_transport: MinimalTransportCalibration,
}

impl StreamingBudget {
    pub fn validate(&self) -> Result<(), BudgetValidationError> {
        require_contract_header(&self.schema, &self.kind, "streaming_calibration")?;
        self.provenance.validate()?;
        if self.legacy_0_3.version != "0.3.0" {
            return Err(invalid("legacy_0_3.version must be 0.3.0"));
        }
        if self.minimal_transport.protocol_version != "0.4.0" {
            return Err(invalid("minimal_transport.protocol_version must be 0.4.0"));
        }
        let fixture_bytes = self.provenance.fixture.bytes;
        if self.legacy_0_3.input_bytes != fixture_bytes
            || self.minimal_transport.input_bytes != fixture_bytes
        {
            return Err(invalid(
                "calibration input bytes must match provenance.fixture.bytes",
            ));
        }
        if self.legacy_0_3.counts.append_calls != self.provenance.fixture.chunks {
            return Err(invalid(
                "legacy append count must match provenance.fixture.chunks",
            ));
        }
        if self.legacy_0_3.counts.update_count
            != self.legacy_0_3.counts.append_calls.saturating_add(1)
        {
            return Err(invalid(
                "legacy update count must include one finalization update",
            ));
        }
        if self.minimal_transport.counts.chunk_count != self.provenance.fixture.chunks {
            return Err(invalid(
                "minimal transport chunk count must match provenance.fixture.chunks",
            ));
        }
        if self.minimal_transport.counts.change_count
            != self.minimal_transport.counts.applied_changes
        {
            return Err(invalid(
                "minimal transport change_count must match applied_changes",
            ));
        }
        if self.minimal_transport.counts.operation_count
            != self.minimal_transport.counts.operations_visited
        {
            return Err(invalid(
                "minimal transport operation_count must match operations_visited",
            ));
        }
        if self.minimal_transport.counts.change_count == 0
            || self.minimal_transport.wire.encoded_change_bytes == 0
            || self.minimal_transport.wire.encoded_snapshot_bytes == 0
        {
            return Err(invalid(
                "minimal transport counts and wire measurements must be non-zero",
            ));
        }
        if self.legacy_0_3.input_bytes != CALIBRATION_FIXTURE_BYTES
            || self.legacy_0_3.counts != FROZEN_LEGACY_COUNTS
        {
            return Err(invalid("legacy calibration measurements drifted"));
        }
        if self.minimal_transport.input_bytes != CALIBRATION_FIXTURE_BYTES
            || self.minimal_transport.counts != FROZEN_MINIMAL_COUNTS
            || self.minimal_transport.wire != FROZEN_WIRE_MEASUREMENTS
        {
            return Err(invalid("minimal transport measurements drifted"));
        }
        Ok(())
    }

    /// Compares only fixture identity and deterministic calibration output.
    /// Host, operating-system, and tool invocation details remain provenance,
    /// but do not make a replay fail on another machine.
    pub fn verify_deterministic_match(&self, expected: &Self) -> Result<(), BudgetValidationError> {
        if self.provenance.fixture != expected.provenance.fixture {
            return Err(invalid("calibration fixture provenance drifted"));
        }
        if self.legacy_0_3 != expected.legacy_0_3 {
            return Err(invalid("legacy calibration measurements drifted"));
        }
        if self.minimal_transport != expected.minimal_transport {
            return Err(invalid("minimal transport measurements drifted"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationProvenance {
    pub source_commit: String,
    pub hot_path_clean: bool,
    pub os: String,
    pub os_version: String,
    pub arch: String,
    pub cpu: String,
    pub rustc: RustToolchain,
    pub cargo_version: String,
    pub profile: String,
    pub command: String,
    pub fixture: CalibrationFixture,
}

impl CalibrationProvenance {
    fn validate(&self) -> Result<(), BudgetValidationError> {
        if self.source_commit != U7_BASELINE_COMMIT {
            return Err(invalid(format!(
                "source_commit must identify the frozen U7 baseline {U7_BASELINE_COMMIT}"
            )));
        }
        if !self.hot_path_clean {
            return Err(invalid(
                "calibration hot paths must be unchanged from source_commit",
            ));
        }
        for (field, value) in [
            ("provenance.os", self.os.as_str()),
            ("provenance.os_version", self.os_version.as_str()),
            ("provenance.arch", self.arch.as_str()),
            ("provenance.cpu", self.cpu.as_str()),
            ("provenance.cargo_version", self.cargo_version.as_str()),
            ("provenance.profile", self.profile.as_str()),
            ("provenance.command", self.command.as_str()),
            ("provenance.rustc.host", self.rustc.host.as_str()),
            (
                "provenance.rustc.commit_hash",
                self.rustc.commit_hash.as_str(),
            ),
            ("provenance.fixture.id", self.fixture.id.as_str()),
            ("provenance.fixture.path", self.fixture.path.as_str()),
            (
                "provenance.fixture.schedule",
                self.fixture.schedule.as_str(),
            ),
        ] {
            if value.trim().is_empty() {
                return Err(invalid(format!("{field} must not be empty")));
            }
        }
        if !self.rustc.release.starts_with("1.85.") {
            return Err(invalid("calibration must use the Rust 1.85 lane"));
        }
        if !is_lower_hex(&self.rustc.commit_hash, 40) {
            return Err(invalid(
                "provenance.rustc.commit_hash must be 40 lowercase hexadecimal characters",
            ));
        }
        if !is_lower_hex(&self.fixture.sha256, 64) {
            return Err(invalid(
                "provenance.fixture.sha256 must be 64 lowercase hexadecimal characters",
            ));
        }
        if self.fixture.bytes == 0 || self.fixture.chunks == 0 {
            return Err(invalid(
                "calibration fixture bytes and chunk count must be non-zero",
            ));
        }
        if self.profile != CALIBRATION_PROFILE {
            return Err(invalid(format!(
                "provenance.profile must be {CALIBRATION_PROFILE}"
            )));
        }
        if self.command != CALIBRATION_COMMAND {
            return Err(invalid(format!(
                "provenance.command must be {CALIBRATION_COMMAND}"
            )));
        }
        if self.fixture.schedule != CALIBRATION_SCHEDULE {
            return Err(invalid(format!(
                "provenance.fixture.schedule must be {CALIBRATION_SCHEDULE}"
            )));
        }
        if self.fixture.id != CALIBRATION_FIXTURE_ID
            || self.fixture.path != CALIBRATION_FIXTURE_PATH
            || self.fixture.sha256 != CALIBRATION_FIXTURE_SHA256
            || self.fixture.bytes != CALIBRATION_FIXTURE_BYTES
            || self.fixture.chunks != CALIBRATION_FIXTURE_CHUNKS
        {
            return Err(invalid("calibration fixture provenance drifted"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RustToolchain {
    pub release: String,
    pub host: String,
    pub commit_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationFixture {
    pub id: String,
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub schedule: String,
    pub chunks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyCalibration {
    pub version: String,
    pub input_bytes: u64,
    pub counts: LegacyCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyCounts {
    pub append_calls: u64,
    pub update_count: u64,
    pub committed_blocks_emitted: u64,
    pub pending_observations: u64,
    pub reset_count: u64,
    pub invalidated_block_ids: u64,
    pub observed_text_bytes: u64,
    pub final_block_count: u64,
    pub retained_buffer_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinimalTransportCalibration {
    pub protocol_version: String,
    pub input_bytes: u64,
    pub counts: MinimalTransportCounts,
    pub wire: WireMeasurements,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinimalTransportCounts {
    pub chunk_count: u64,
    pub change_count: u64,
    pub operation_count: u64,
    pub applied_changes: u64,
    pub operations_visited: u64,
    pub nodes_validated: u64,
    pub relationship_steps: u64,
    pub child_ids_copied: u64,
    pub snapshots_validated: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireMeasurements {
    pub encoded_change_bytes: u64,
    pub encoded_snapshot_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingBudgets {
    pub schema: String,
    pub kind: String,
    pub policy: BindingBudgetPolicy,
    pub artifacts: Vec<BindingArtifactBudget>,
}

impl BindingBudgets {
    pub fn validate(&self) -> Result<(), BudgetValidationError> {
        require_contract_header(&self.schema, &self.kind, "binding_artifact_ceilings")?;
        if self.policy.ceiling_basis != CeilingBasis::AbsoluteBytes {
            return Err(invalid("binding ceilings must use absolute bytes"));
        }
        if !self.policy.relative_limits_are_advisory {
            return Err(invalid("relative limits must remain advisory"));
        }
        if self.policy.default_artifacts_allow_merman {
            return Err(invalid("default artifacts must not allow Merman"));
        }
        if !self
            .policy
            .forbidden_default_dependencies
            .iter()
            .any(|dependency| dependency.eq_ignore_ascii_case("merman"))
        {
            return Err(invalid(
                "forbidden_default_dependencies must include Merman",
            ));
        }

        let mut actual = BTreeMap::new();
        for artifact in &self.artifacts {
            if actual.insert(artifact.artifact, artifact).is_some() {
                return Err(invalid(format!(
                    "duplicate binding artifact budget: {}",
                    artifact.artifact.as_str()
                )));
            }
            match (artifact.status, artifact.measurement.as_ref()) {
                (ArtifactStatus::Pending, None) => {}
                (ArtifactStatus::Measured, Some(measurement)) => {
                    if measurement.measured_bytes > artifact.ceiling_bytes {
                        return Err(invalid(format!(
                            "{} measurement exceeds its absolute ceiling",
                            artifact.artifact.as_str()
                        )));
                    }
                    if !is_lower_hex(&measurement.artifact_sha256, 64)
                        || measurement.command.trim().is_empty()
                    {
                        return Err(invalid(format!(
                            "{} measurement provenance is incomplete",
                            artifact.artifact.as_str()
                        )));
                    }
                }
                (ArtifactStatus::Pending, Some(_)) => {
                    return Err(invalid(format!(
                        "{} is pending and must not contain a measurement",
                        artifact.artifact.as_str()
                    )));
                }
                (ArtifactStatus::Measured, None) => {
                    return Err(invalid(format!(
                        "{} is measured but has no measurement",
                        artifact.artifact.as_str()
                    )));
                }
            }
            let expected_regression_percent = match artifact.artifact {
                BindingArtifact::WasmRaw
                | BindingArtifact::WasmStripped
                | BindingArtifact::WasmGzip
                | BindingArtifact::WasmBrotli => 15,
                BindingArtifact::NpmPacked
                | BindingArtifact::DartPacked
                | BindingArtifact::FlutterNativeLibrary
                | BindingArtifact::PlatformPackageIncrement => 20,
            };
            if artifact.regression_percent != expected_regression_percent {
                return Err(invalid(format!(
                    "{} must retain its {}% advisory regression band",
                    artifact.artifact.as_str(),
                    expected_regression_percent
                )));
            }
        }

        for (artifact, ceiling_bytes, owner) in REQUIRED_BINDING_CEILINGS {
            let Some(budget) = actual.get(&artifact) else {
                return Err(invalid(format!(
                    "missing binding artifact budget: {}",
                    artifact.as_str()
                )));
            };
            if budget.ceiling_bytes != ceiling_bytes || budget.owner != owner {
                return Err(invalid(format!(
                    "{} must retain its frozen owner and absolute ceiling",
                    artifact.as_str()
                )));
            }
        }
        if actual.len() != REQUIRED_BINDING_CEILINGS.len() {
            return Err(invalid("binding budget contains an unexpected artifact"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingBudgetPolicy {
    pub ceiling_basis: CeilingBasis,
    pub relative_limits_are_advisory: bool,
    pub default_artifacts_allow_merman: bool,
    pub forbidden_default_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CeilingBasis {
    AbsoluteBytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingArtifactBudget {
    pub artifact: BindingArtifact,
    pub owner: BudgetOwner,
    pub ceiling_bytes: u64,
    pub regression_percent: u64,
    pub status: ArtifactStatus,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub measurement: Option<ArtifactMeasurement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMeasurement {
    pub measured_bytes: u64,
    pub artifact_sha256: String,
    pub command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Pending,
    Measured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingArtifact {
    WasmRaw,
    WasmStripped,
    WasmGzip,
    WasmBrotli,
    NpmPacked,
    DartPacked,
    FlutterNativeLibrary,
    PlatformPackageIncrement,
}

impl BindingArtifact {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WasmRaw => "wasm_raw",
            Self::WasmStripped => "wasm_stripped",
            Self::WasmGzip => "wasm_gzip",
            Self::WasmBrotli => "wasm_brotli",
            Self::NpmPacked => "npm_packed",
            Self::DartPacked => "dart_packed",
            Self::FlutterNativeLibrary => "flutter_native_library",
            Self::PlatformPackageIncrement => "platform_package_increment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetOwner {
    U14,
    U16,
    U17,
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

pub const REQUIRED_BINDING_CEILINGS: [(BindingArtifact, u64, BudgetOwner); 8] = [
    (BindingArtifact::WasmRaw, 1_572_864, BudgetOwner::U14),
    (BindingArtifact::WasmStripped, 1_310_720, BudgetOwner::U14),
    (BindingArtifact::WasmGzip, 460_800, BudgetOwner::U14),
    (BindingArtifact::WasmBrotli, 409_600, BudgetOwner::U14),
    (BindingArtifact::NpmPacked, 665_600, BudgetOwner::U14),
    (BindingArtifact::DartPacked, 163_840, BudgetOwner::U16),
    (
        BindingArtifact::FlutterNativeLibrary,
        6_291_456,
        BudgetOwner::U17,
    ),
    (
        BindingArtifact::PlatformPackageIncrement,
        8_388_608,
        BudgetOwner::U17,
    ),
];

pub fn load_streaming_budget(path: impl AsRef<Path>) -> Result<StreamingBudget, BudgetLoadError> {
    load_and_validate(path, StreamingBudget::validate)
}

pub fn load_binding_budgets(path: impl AsRef<Path>) -> Result<BindingBudgets, BudgetLoadError> {
    load_and_validate(path, BindingBudgets::validate)
}

fn load_and_validate<T>(
    path: impl AsRef<Path>,
    validate: impl FnOnce(&T) -> Result<(), BudgetValidationError>,
) -> Result<T, BudgetLoadError>
where
    T: for<'de> Deserialize<'de>,
{
    let path = path.as_ref();
    let bytes = fs::read(path).map_err(|source| BudgetLoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let value = serde_json::from_slice(&bytes).map_err(|source| BudgetLoadError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    validate(&value).map_err(BudgetLoadError::Invalid)?;
    Ok(value)
}

fn require_contract_header(
    schema: &str,
    actual_kind: &str,
    expected_kind: &str,
) -> Result<(), BudgetValidationError> {
    if schema != BUDGET_SCHEMA {
        return Err(invalid(format!("unsupported budget schema: {schema}")));
    }
    if actual_kind != expected_kind {
        return Err(invalid(format!(
            "expected budget kind {expected_kind}, found {actual_kind}"
        )));
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid(message: impl Into<String>) -> BudgetValidationError {
    BudgetValidationError(message.into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetValidationError(String);

impl fmt::Display for BudgetValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BudgetValidationError {}

#[derive(Debug)]
pub enum BudgetLoadError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    Invalid(BudgetValidationError),
}

impl fmt::Display for BudgetLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "failed to read {}: {source}", path.display())
            }
            Self::Json { path, source } => {
                write!(formatter, "failed to decode {}: {source}", path.display())
            }
            Self::Invalid(source) => write!(formatter, "invalid budget contract: {source}"),
        }
    }
}

impl std::error::Error for BudgetLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            Self::Invalid(source) => Some(source),
        }
    }
}
