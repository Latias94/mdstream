use std::{path::PathBuf, ptr};

use mdstream_bindings_core::{BindingPayloadKind, BindingStatus};
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

#[test]
fn gap_blocks_continuation_until_explicit_snapshot_recovery() {
    let fixture = load_fixture(fixture_path()).unwrap();
    let trace = fixture
        .traces
        .iter()
        .find(|trace| trace.id == "characters")
        .unwrap();
    let changes = trace
        .changes
        .iter()
        .map(|change| encode_change_json(change, usize::MAX, ProtocolLimits::default()).unwrap())
        .collect::<Vec<_>>();

    let primary = new_reducer();
    for change in changes.iter().take(3) {
        free_success(apply(primary, change));
    }
    let recovery = take_single_payload(execute(primary, SNAPSHOT));
    assert_eq!(recovery.0, BindingPayloadKind::Snapshot as u32);
    free_success(apply(primary, &changes[3]));
    let expected_final = take_single_payload(execute(primary, SNAPSHOT));

    let replica = new_reducer();
    free_success(apply(replica, &changes[0]));
    let gap = take_single_payload(apply(replica, &changes[2]));
    assert_eq!(gap.0, BindingPayloadKind::ReducerUpdate as u32);
    let gap_json: serde_json::Value = serde_json::from_slice(&gap.1).unwrap();
    assert_eq!(gap_json["outcome"]["kind"], "recovery_required");

    let blocked = apply(replica, &changes[3]);
    assert_eq!(blocked.status, BindingStatus::NeedsSnapshot.code());
    assert!(blocked.output.is_null());
    let blocked_error: serde_json::Value =
        serde_json::from_slice(&take_buffer(blocked.error)).unwrap();
    assert_eq!(blocked_error["status_name"], "MDSTREAM_NEEDS_SNAPSHOT");

    free_success(unsafe {
        mdstream_reducer_recover_snapshot(replica, recovery.1.as_ptr(), recovery.1.len())
    });
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

fn new_reducer() -> *mut MdstreamReducer {
    let result = unsafe { mdstream_reducer_new(ptr::null(), 0) };
    assert_eq!(result.status, BindingStatus::Ok.code());
    assert!(!result.reducer.is_null());
    result.reducer
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
