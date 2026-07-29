use std::{path::PathBuf, ptr, sync::Mutex};

use mdstream_bindings_core::{
    BINDING_OPTIONS_SCHEMA, BindingOutput, BindingPayloadKind, BindingStatus, ReducerSession,
    TRANSITION_SCHEMA,
};
use mdstream_conformance::load_fixture;
use mdstream_ffi::{
    MdstreamCallResult, MdstreamReducer, mdstream_allocation_metrics, mdstream_output_free,
    mdstream_output_len, mdstream_output_take, mdstream_reducer_apply_change,
    mdstream_reducer_execute, mdstream_reducer_free, mdstream_reducer_new,
    mdstream_reducer_recover_snapshot,
};
use mdstream_protocol::{ProtocolLimits, decode_snapshot_json, encode_change_json};

#[path = "support/ffi.rs"]
mod ffi_support;

use ffi_support::{free_success, take_buffer};

const SNAPSHOT: &[u8] = br#"{"schema":"mdstream.bindings/0.4","kind":"snapshot"}"#;
static FFI_TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn gap_blocks_continuation_until_explicit_snapshot_recovery() {
    let _ffi_guard = FFI_TEST_LOCK.lock().unwrap();
    let changes = character_changes();

    let primary = new_reducer(None);
    for change in changes.iter().take(3) {
        free_success(apply(primary, change));
    }
    let recovery = take_single_payload(execute(primary, SNAPSHOT));
    assert_eq!(recovery.0, BindingPayloadKind::Snapshot as u32);
    free_success(apply(primary, &changes[3]));
    let expected_final = take_single_payload(execute(primary, SNAPSHOT));

    let options = transition_options();
    let replica = new_reducer(Some(&options));
    let initial = take_single_payload(apply(replica, &changes[0]));
    assert_eq!(initial.0, BindingPayloadKind::ReducerUpdate as u32);
    let initial = decode_update(&initial.1);
    assert_eq!(initial["transition"]["schema"], TRANSITION_SCHEMA);
    assert_eq!(
        initial["transition"]["facts"]["after"]["continuity_generation"],
        "0"
    );
    let same_floor = take_single_payload(execute(replica, SNAPSHOT));
    assert_eq!(same_floor.0, BindingPayloadKind::Snapshot as u32);
    let gap = take_single_payload(apply(replica, &changes[2]));
    assert_eq!(gap.0, BindingPayloadKind::ReducerUpdate as u32);
    let gap_json: serde_json::Value = serde_json::from_slice(&gap.1).unwrap();
    assert_eq!(gap_json["outcome"]["kind"], "recovery_required");
    assert!(gap_json.get("transition").is_none());

    let blocked = apply(replica, &changes[3]);
    assert_eq!(blocked.status, BindingStatus::NeedsSnapshot.code());
    assert!(blocked.output.is_null());
    let blocked_error: serde_json::Value =
        serde_json::from_slice(&take_buffer(blocked.error)).unwrap();
    assert_eq!(blocked_error["status_name"], "MDSTREAM_NEEDS_SNAPSHOT");

    let same_floor_recovery = take_single_payload(unsafe {
        mdstream_reducer_recover_snapshot(replica, same_floor.1.as_ptr(), same_floor.1.len())
    });
    assert_eq!(
        same_floor_recovery.0,
        BindingPayloadKind::ReducerUpdate as u32
    );
    let same_floor_recovery = decode_update(&same_floor_recovery.1);
    assert_eq!(same_floor_recovery["outcome"]["kind"], "recovered");
    assert!(same_floor_recovery.get("transition").is_none());

    let gap = take_single_payload(apply(replica, &changes[2]));
    assert_eq!(gap.0, BindingPayloadKind::ReducerUpdate as u32);
    assert_eq!(
        decode_update(&gap.1)["outcome"]["kind"],
        "recovery_required"
    );
    let advanced_recovery = take_single_payload(unsafe {
        mdstream_reducer_recover_snapshot(replica, recovery.1.as_ptr(), recovery.1.len())
    });
    assert_eq!(
        advanced_recovery.0,
        BindingPayloadKind::ReducerUpdate as u32
    );
    let advanced_recovery = decode_update(&advanced_recovery.1);
    assert_eq!(advanced_recovery["outcome"]["kind"], "recovered");
    assert_eq!(advanced_recovery["impact"]["full_replace"], true);
    assert_eq!(
        advanced_recovery["transition"]["facts"]["scope"],
        "full_replace"
    );
    assert_eq!(
        advanced_recovery["transition"]["facts"]["before"]["continuity_generation"],
        "0"
    );
    assert_eq!(
        advanced_recovery["transition"]["facts"]["after"]["continuity_generation"],
        "1"
    );
    free_success(apply(replica, &changes[3]));
    let final_snapshot = take_single_payload(execute(replica, SNAPSHOT));
    let expected =
        decode_snapshot_json(&expected_final.1, usize::MAX, ProtocolLimits::default()).unwrap();
    let actual =
        decode_snapshot_json(&final_snapshot.1, usize::MAX, ProtocolLimits::default()).unwrap();
    assert_eq!(actual, expected);

    unsafe {
        mdstream_reducer_free(replica);
        mdstream_reducer_free(primary);
    }
    assert_eq!(mdstream_allocation_metrics(), Default::default());
}

#[test]
fn c_ffi_reducer_updates_are_byte_identical_to_the_direct_core() {
    let _ffi_guard = FFI_TEST_LOCK.lock().unwrap();
    let changes = character_changes();

    let mut producer = ReducerSession::new(b"").unwrap();
    for change in changes.iter().take(3) {
        producer.apply_change(change).unwrap();
    }
    let advanced_snapshot =
        take_core_payload(producer.snapshot().unwrap(), BindingPayloadKind::Snapshot);

    let options = transition_options();
    let mut direct = ReducerSession::new(&options).unwrap();
    let ffi = new_reducer(Some(&options));

    let continuous = assert_ffi_reducer_update_parity(
        direct.apply_change(&changes[0]).unwrap(),
        apply(ffi, &changes[0]),
    );
    let continuous = decode_update(&continuous);
    assert_eq!(continuous["transition"]["facts"]["scope"], "continuous");

    let gap = assert_ffi_reducer_update_parity(
        direct.apply_change(&changes[2]).unwrap(),
        apply(ffi, &changes[2]),
    );
    let gap = decode_update(&gap);
    assert_eq!(gap["outcome"]["kind"], "recovery_required");

    let full_replace = assert_ffi_reducer_update_parity(
        direct.recover_snapshot(&advanced_snapshot).unwrap(),
        unsafe {
            mdstream_reducer_recover_snapshot(
                ffi,
                advanced_snapshot.as_ptr(),
                advanced_snapshot.len(),
            )
        },
    );
    let full_replace = decode_update(&full_replace);
    assert_eq!(full_replace["transition"]["facts"]["scope"], "full_replace");

    unsafe { mdstream_reducer_free(ffi) };
    assert_eq!(mdstream_allocation_metrics(), Default::default());
}

fn character_changes() -> Vec<Vec<u8>> {
    let fixture = load_fixture(fixture_path()).unwrap();
    let trace = fixture
        .traces
        .iter()
        .find(|trace| trace.id == "characters")
        .unwrap();
    trace
        .changes
        .iter()
        .map(|change| encode_change_json(change, usize::MAX, ProtocolLimits::default()).unwrap())
        .collect()
}

fn take_core_payload(output: BindingOutput, expected_kind: BindingPayloadKind) -> Vec<u8> {
    let mut payloads = output.into_payloads();
    assert_eq!(payloads.len(), 1);
    let payload = payloads.pop().unwrap();
    assert_eq!(payload.kind(), expected_kind);
    payload.into_bytes()
}

fn assert_ffi_reducer_update_parity(
    expected: BindingOutput,
    actual: MdstreamCallResult,
) -> Vec<u8> {
    let expected = take_core_payload(expected, BindingPayloadKind::ReducerUpdate);
    let (kind, actual) = take_single_payload(actual);
    assert_eq!(BindingPayloadKind::ReducerUpdate as u32, 3);
    assert_eq!(kind, BindingPayloadKind::ReducerUpdate as u32);
    assert_eq!(actual, expected);
    actual
}

fn new_reducer(options: Option<&[u8]>) -> *mut MdstreamReducer {
    let (pointer, len) = options.map_or((ptr::null(), 0), |bytes| (bytes.as_ptr(), bytes.len()));
    let result = unsafe { mdstream_reducer_new(pointer, len) };
    assert_eq!(result.status, BindingStatus::Ok.code());
    assert!(!result.reducer.is_null());
    result.reducer
}

fn transition_options() -> Vec<u8> {
    format!(
        r#"{{
          "schema":"{BINDING_OPTIONS_SCHEMA}",
          "capture_transitions":true,
          "protocol":{{
            "max_source_bytes":"1024",
            "max_nodes":"16",
            "max_resources":"16",
            "max_operations":"32",
            "max_change_structural_items":"64",
            "max_children_per_list":"16"
          }},
          "wire":{{"max_reducer_update_bytes":"1048576"}}
        }}"#
    )
    .into_bytes()
}

fn decode_update(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(bytes).unwrap()
}

fn apply(reducer: *mut MdstreamReducer, change: &[u8]) -> MdstreamCallResult {
    unsafe { mdstream_reducer_apply_change(reducer, change.as_ptr(), change.len()) }
}

fn execute(reducer: *mut MdstreamReducer, command: &[u8]) -> MdstreamCallResult {
    unsafe { mdstream_reducer_execute(reducer, command.as_ptr(), command.len()) }
}

fn take_single_payload(result: MdstreamCallResult) -> (u32, Vec<u8>) {
    assert_eq!(result.status, BindingStatus::Ok.code());
    let output = result.output;
    assert_eq!(unsafe { mdstream_output_len(output) }, 1);
    let payload = unsafe { mdstream_output_take(output, 0) };
    assert_eq!(payload.status, BindingStatus::Ok.code());
    let result = (payload.kind, take_buffer(payload.data));
    unsafe { mdstream_output_free(output) };
    result
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("conformance/fixtures/protocol-linear-source.json")
}
