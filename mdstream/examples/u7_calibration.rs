use std::{env, fs, path::PathBuf, process::Command};

use mdstream::{DocumentState, MdStream, Options, Update};
use mdstream_conformance::{
    BUDGET_SCHEMA, CALIBRATION_COMMAND, CALIBRATION_FIXTURE_ID, CALIBRATION_FIXTURE_PATH,
    CALIBRATION_PROFILE, CALIBRATION_SCHEDULE, CalibrationFixture, CalibrationProvenance,
    ChunkSchedule, LegacyCalibration, LegacyCounts, MinimalTransportCalibration,
    MinimalTransportCounts, RustToolchain, StreamingBudget, U7_BASELINE_COMMIT, WireMeasurements,
    load_streaming_budget, source_only_trace,
};
use mdstream_protocol::{Epoch, ProtocolLimits, Reducer, encode_change_json, encode_snapshot_json};
use sha2::{Digest, Sha256};

const FIXTURE: &str = include_str!("../tests/fixtures/streamdown_bench/mixed_content_realistic.md");

enum Mode {
    Write(PathBuf),
    Check(PathBuf),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = mode()?;
    require_frozen_hot_paths()?;

    let chunks = ChunkSchedule::Characters.slices(FIXTURE)?;
    let fixture = CalibrationFixture {
        id: CALIBRATION_FIXTURE_ID.to_string(),
        path: CALIBRATION_FIXTURE_PATH.to_string(),
        sha256: sha256(FIXTURE.as_bytes()),
        bytes: to_u64(FIXTURE.len())?,
        schedule: CALIBRATION_SCHEDULE.to_string(),
        chunks: to_u64(chunks.len())?,
    };
    let budget = StreamingBudget {
        schema: BUDGET_SCHEMA.to_string(),
        kind: "streaming_calibration".to_string(),
        provenance: provenance(fixture)?,
        legacy_0_3: calibrate_legacy(&chunks)?,
        minimal_transport: calibrate_minimal_transport(&chunks)?,
    };
    budget.validate()?;
    match mode {
        Mode::Write(output) => {
            let mut encoded = serde_json::to_vec_pretty(&budget)?;
            encoded.push(b'\n');
            fs::write(&output, encoded)?;
            println!("wrote calibration candidate {}", output.display());
        }
        Mode::Check(expected_path) => {
            let expected = load_streaming_budget(&expected_path)?;
            budget.verify_deterministic_match(&expected)?;
            println!(
                "U7 deterministic calibration matches {}",
                expected_path.display()
            );
        }
    }
    Ok(())
}

fn mode() -> Result<Mode, Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    match (
        args.next().as_deref(),
        args.next().map(PathBuf::from),
        args.next(),
    ) {
        (Some(flag), Some(path), None) if flag == "--output" => Ok(Mode::Write(path)),
        (Some(flag), Some(path), None) if flag == "--check" => Ok(Mode::Check(path)),
        _ => Err("usage: u7_calibration (--output|--check) <path>".into()),
    }
}

fn require_frozen_hot_paths() -> Result<(), Box<dyn std::error::Error>> {
    let baseline = format!("{U7_BASELINE_COMMIT}^{{commit}}");
    let baseline_exists = Command::new("git")
        .args(["cat-file", "-e", &baseline])
        .status()?;
    if !baseline_exists.success() {
        return Err(format!("missing calibration source commit {U7_BASELINE_COMMIT}").into());
    }
    let status = Command::new("git")
        .args([
            "diff",
            "--quiet",
            U7_BASELINE_COMMIT,
            "--",
            "mdstream/src",
            "mdstream-protocol/src",
            "mdstream-conformance/src/trace.rs",
        ])
        .status()?;
    if !status.success() {
        return Err(
            "calibration hot paths differ from the frozen source commit; commit the final core hot paths, advance U7_BASELINE_COMMIT and matching schema/budget provenance, then recalibrate"
                .into(),
        );
    }
    Ok(())
}

fn provenance(
    fixture: CalibrationFixture,
) -> Result<CalibrationProvenance, Box<dyn std::error::Error>> {
    let rustc_verbose = command_output("rustc", &["--version", "--verbose"])?;
    let rustc = RustToolchain {
        release: rustc_field(&rustc_verbose, "release")?,
        host: rustc_field(&rustc_verbose, "host")?,
        commit_hash: rustc_field(&rustc_verbose, "commit-hash")?,
    };
    Ok(CalibrationProvenance {
        source_commit: U7_BASELINE_COMMIT.to_string(),
        hot_path_clean: true,
        os: env::consts::OS.to_string(),
        os_version: command_output("uname", &["-sr"])?,
        arch: env::consts::ARCH.to_string(),
        cpu: cpu_model()?,
        rustc,
        cargo_version: command_output("cargo", &["--version"])?,
        profile: CALIBRATION_PROFILE.to_string(),
        command: CALIBRATION_COMMAND.to_string(),
        fixture,
    })
}

fn calibrate_legacy(chunks: &[&str]) -> Result<LegacyCalibration, Box<dyn std::error::Error>> {
    let mut stream = MdStream::new(Options::default());
    let mut state = DocumentState::new();
    let mut counts = LegacyCounts {
        append_calls: to_u64(chunks.len())?,
        update_count: 0,
        committed_blocks_emitted: 0,
        pending_observations: 0,
        reset_count: 0,
        invalidated_block_ids: 0,
        observed_text_bytes: 0,
        final_block_count: 0,
        retained_buffer_bytes: 0,
    };

    for chunk in chunks {
        observe_legacy_update(&mut counts, &mut state, stream.append(chunk))?;
    }
    observe_legacy_update(&mut counts, &mut state, stream.finalize())?;
    counts.final_block_count = to_u64(state.blocks().count())?;
    counts.retained_buffer_bytes = to_u64(stream.buffer().len())?;

    Ok(LegacyCalibration {
        version: "0.3.0".to_string(),
        input_bytes: to_u64(FIXTURE.len())?,
        counts,
    })
}

fn observe_legacy_update(
    counts: &mut LegacyCounts,
    state: &mut DocumentState,
    update: Update,
) -> Result<(), Box<dyn std::error::Error>> {
    counts.update_count = counts
        .update_count
        .checked_add(1)
        .ok_or("update count overflow")?;
    counts.committed_blocks_emitted = counts
        .committed_blocks_emitted
        .checked_add(to_u64(update.committed.len())?)
        .ok_or("committed block count overflow")?;
    counts.pending_observations = counts
        .pending_observations
        .checked_add(u64::from(update.pending.is_some()))
        .ok_or("pending observation count overflow")?;
    counts.reset_count = counts
        .reset_count
        .checked_add(u64::from(update.reset))
        .ok_or("reset count overflow")?;
    counts.invalidated_block_ids = counts
        .invalidated_block_ids
        .checked_add(to_u64(update.invalidated.len())?)
        .ok_or("invalidated block count overflow")?;
    let observed = update
        .committed
        .iter()
        .map(|block| block.raw.len())
        .chain(
            update
                .pending
                .iter()
                .map(|block| block.display_or_raw().len()),
        )
        .try_fold(0_u64, |total, bytes| total.checked_add(to_u64(bytes).ok()?))
        .ok_or("observed text byte count overflow")?;
    counts.observed_text_bytes = counts
        .observed_text_bytes
        .checked_add(observed)
        .ok_or("observed text byte count overflow")?;
    state.apply(update);
    Ok(())
}

fn calibrate_minimal_transport(
    chunks: &[&str],
) -> Result<MinimalTransportCalibration, Box<dyn std::error::Error>> {
    let limits = ProtocolLimits::default();
    let trace = source_only_trace(
        "u7-calibration",
        "characters",
        Epoch::new(1),
        chunks.iter().copied(),
    )?;
    let operation_count = trace
        .changes
        .iter()
        .map(|change| change.operations().len())
        .try_fold(0_usize, |total, operations| total.checked_add(operations))
        .ok_or("operation count overflow")?;
    let encoded_change_bytes = trace
        .changes
        .iter()
        .try_fold(0_usize, |total, change| {
            let bytes = encode_change_json(change, usize::MAX, limits).ok()?;
            total.checked_add(bytes.len())
        })
        .ok_or("change wire byte count overflow")?;

    let mut reducer = Reducer::new();
    for change in &trace.changes {
        reducer.apply(change.clone())?;
    }
    let document = reducer
        .document()
        .ok_or("minimal transport did not install a document")?;
    if document.source() != FIXTURE {
        return Err("minimal transport did not preserve the fixture source".into());
    }
    let snapshot = document.snapshot();
    let encoded_snapshot_bytes = encode_snapshot_json(&snapshot, usize::MAX, limits)?.len();
    let metrics = reducer.metrics();

    Ok(MinimalTransportCalibration {
        protocol_version: "0.4.0".to_string(),
        input_bytes: to_u64(FIXTURE.len())?,
        counts: MinimalTransportCounts {
            chunk_count: to_u64(chunks.len())?,
            change_count: to_u64(trace.changes.len())?,
            operation_count: to_u64(operation_count)?,
            applied_changes: metrics.applied_changes,
            operations_visited: metrics.operations_visited,
            nodes_validated: metrics.nodes_validated,
            relationship_steps: metrics.relationship_steps,
            child_ids_copied: metrics.child_ids_copied,
            snapshots_validated: metrics.snapshots_validated,
        },
        wire: WireMeasurements {
            encoded_change_bytes: to_u64(encoded_change_bytes)?,
            encoded_snapshot_bytes: to_u64(encoded_snapshot_bytes)?,
        },
    })
}

fn cpu_model() -> Result<String, Box<dyn std::error::Error>> {
    if env::consts::OS == "macos" {
        return command_output("sysctl", &["-n", "machdep.cpu.brand_string"]);
    }
    if env::consts::OS == "linux" {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo")?;
        if let Some(model) = cpuinfo.lines().find_map(|line| {
            line.split_once(':')
                .filter(|(field, _)| *field == "model name" || *field == "Hardware")
                .map(|(_, value)| value.trim().to_string())
        }) {
            return Ok(model);
        }
    }
    env::var("PROCESSOR_IDENTIFIER")
        .map_err(|_| "unable to determine CPU model for calibration provenance".into())
}

fn rustc_field(output: &str, field: &str) -> Result<String, Box<dyn std::error::Error>> {
    output
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| *name == field)
                .map(|(_, value)| value.trim().to_string())
        })
        .ok_or_else(|| format!("rustc --version --verbose omitted {field}").into())
}

fn command_output(program: &str, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new(program).args(args).output()?;
    if !output.status.success() {
        return Err(format!("{program} {} failed", args.join(" ")).into());
    }
    let value = String::from_utf8(output.stdout)?.trim().to_string();
    if value.is_empty() {
        return Err(format!("{program} {} produced no output", args.join(" ")).into());
    }
    Ok(value)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn to_u64(value: usize) -> Result<u64, Box<dyn std::error::Error>> {
    u64::try_from(value).map_err(|_| "calibration count exceeds u64".into())
}
