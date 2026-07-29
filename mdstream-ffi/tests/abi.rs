use std::{ffi::CStr, mem::size_of, ptr};

use mdstream_bindings_core::{
    BINDING_OPTIONS_SCHEMA, BINDING_SCHEMA, BindingPayloadKind, BindingStatus, TRANSITION_SCHEMA,
};
use mdstream_ffi::{
    MDSTREAM_ABI_VERSION, MdstreamAllocationMetrics, MdstreamBuffer, MdstreamCallResult,
    MdstreamEngineResult, MdstreamPayloadResult, MdstreamProcessorSchedulerLimits,
    MdstreamReducerResult, mdstream_abi_version, mdstream_allocation_metrics,
    mdstream_allocation_metrics_struct_size, mdstream_binding_options_schema,
    mdstream_binding_schema, mdstream_buffer_free, mdstream_buffer_struct_size,
    mdstream_call_result_struct_size, mdstream_engine_append, mdstream_engine_execute,
    mdstream_engine_free, mdstream_engine_new, mdstream_engine_raw_append_byte_ceiling,
    mdstream_engine_result_struct_size, mdstream_output_free, mdstream_output_len,
    mdstream_output_remaining, mdstream_output_take, mdstream_package_version,
    mdstream_payload_result_struct_size, mdstream_processor_scheduler_limits_struct_size,
    mdstream_reducer_apply_change, mdstream_reducer_execute, mdstream_reducer_free,
    mdstream_reducer_new, mdstream_reducer_processor_scheduler_limits,
    mdstream_reducer_recover_snapshot, mdstream_reducer_result_struct_size,
    mdstream_transition_schema,
};
use mdstream_protocol::{
    ChangeId, ChangeSet, Epoch, ProtocolLimits, SourceCursor, SourceDelta, TransitionFacts,
    decode_snapshot_json, encode_change_json,
};

#[path = "support/ffi.rs"]
mod ffi_support;

use ffi_support::{free_success, take_buffer};

const FINISH: &[u8] = br#"{"schema":"mdstream.bindings/0.4","kind":"finish"}"#;
const RESET: &[u8] = br#"{"schema":"mdstream.bindings/0.4","kind":"reset"}"#;
const ENGINE_SNAPSHOT: &[u8] = br#"{"schema":"mdstream.bindings/0.4","kind":"snapshot"}"#;
const REDUCER_SNAPSHOT: &[u8] = ENGINE_SNAPSHOT;

#[test]
fn c_abi_metadata_errors_outputs_and_stateful_roundtrip_match_the_frozen_contract() {
    assert_eq!(
        [
            BindingStatus::Ok.code(),
            BindingStatus::InvalidArgument.code(),
            BindingStatus::Utf8.code(),
            BindingStatus::Options.code(),
            BindingStatus::Command.code(),
            BindingStatus::UnsupportedSchema.code(),
            BindingStatus::Terminal.code(),
            BindingStatus::Engine.code(),
            BindingStatus::Protocol.code(),
            BindingStatus::NeedsSnapshot.code(),
            BindingStatus::Processor.code(),
            BindingStatus::ResourceLimit.code(),
            BindingStatus::Internal.code(),
            BindingStatus::Panic.code(),
        ],
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
    );
    assert_eq!(
        [
            BindingPayloadKind::Change as u32,
            BindingPayloadKind::Snapshot as u32,
            BindingPayloadKind::ReducerUpdate as u32,
            BindingPayloadKind::NodeView as u32,
            BindingPayloadKind::ResourceView as u32,
            BindingPayloadKind::ProcessorRequest as u32,
            BindingPayloadKind::ProcessorCompletion as u32,
            BindingPayloadKind::ArtifactChange as u32,
            BindingPayloadKind::ArtifactView as u32,
            BindingPayloadKind::PendingSourceView as u32,
        ],
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    );
    assert_eq!(mdstream_abi_version(), MDSTREAM_ABI_VERSION);
    assert_eq!(
        static_string(mdstream_package_version()),
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(static_string(mdstream_binding_schema()), BINDING_SCHEMA);
    assert_eq!(
        static_string(mdstream_binding_options_schema()),
        BINDING_OPTIONS_SCHEMA
    );
    assert_eq!(
        static_string(mdstream_transition_schema()),
        TRANSITION_SCHEMA
    );
    assert_eq!(mdstream_buffer_struct_size(), size_of::<MdstreamBuffer>());
    assert_eq!(
        mdstream_call_result_struct_size(),
        size_of::<MdstreamCallResult>()
    );
    assert_eq!(
        mdstream_engine_result_struct_size(),
        size_of::<MdstreamEngineResult>()
    );
    assert_eq!(
        mdstream_reducer_result_struct_size(),
        size_of::<MdstreamReducerResult>()
    );
    assert_eq!(
        mdstream_payload_result_struct_size(),
        size_of::<MdstreamPayloadResult>()
    );
    assert_eq!(
        mdstream_allocation_metrics_struct_size(),
        size_of::<MdstreamAllocationMetrics>()
    );
    assert_eq!(
        mdstream_processor_scheduler_limits_struct_size(),
        size_of::<MdstreamProcessorSchedulerLimits>()
    );
    assert_eq!(
        unsafe { mdstream_reducer_processor_scheduler_limits(ptr::null()) },
        MdstreamProcessorSchedulerLimits::default()
    );

    let invalid = unsafe { mdstream_engine_new(ptr::null(), 1) };
    assert_eq!(invalid.status, BindingStatus::InvalidArgument.code());
    assert!(invalid.engine.is_null());
    assert_error(
        invalid.error,
        "MDSTREAM_INVALID_ARGUMENT",
        "ffi.null_pointer",
    );

    let invalid_options = unsafe { mdstream_engine_new(b"{".as_ptr(), 1) };
    assert_eq!(invalid_options.status, BindingStatus::Options.code());
    assert!(invalid_options.engine.is_null());
    assert_error(
        invalid_options.error,
        "MDSTREAM_OPTIONS_ERROR",
        "bindings.invalid_options",
    );

    let transition_options = transition_options();
    let wrong_options_schema = String::from_utf8(transition_options.clone())
        .unwrap()
        .replace(BINDING_OPTIONS_SCHEMA, "mdstream.bindings-options/999");
    let schema_mismatch =
        unsafe { mdstream_reducer_new(wrong_options_schema.as_ptr(), wrong_options_schema.len()) };
    assert_eq!(
        schema_mismatch.status,
        BindingStatus::UnsupportedSchema.code()
    );
    assert!(schema_mismatch.reducer.is_null());
    assert_error(
        schema_mismatch.error,
        "MDSTREAM_UNSUPPORTED_SCHEMA",
        "bindings.unsupported_options_schema",
    );

    let null_handle = unsafe { mdstream_engine_append(ptr::null_mut(), ptr::null(), 0) };
    assert_eq!(null_handle.status, BindingStatus::InvalidArgument.code());
    assert_error(
        null_handle.error,
        "MDSTREAM_INVALID_ARGUMENT",
        "ffi.null_handle",
    );
    let null_output = unsafe { mdstream_output_take(ptr::null_mut(), 0) };
    assert_eq!(null_output.status, BindingStatus::InvalidArgument.code());
    assert_error(
        null_output.data,
        "MDSTREAM_INVALID_ARGUMENT",
        "ffi.null_handle",
    );
    let overflowing_length =
        unsafe { mdstream_engine_new(b"".as_ptr(), (isize::MAX as usize).saturating_add(1)) };
    assert_eq!(
        overflowing_length.status,
        BindingStatus::InvalidArgument.code()
    );
    assert_error(
        overflowing_length.error,
        "MDSTREAM_INVALID_ARGUMENT",
        "ffi.length_overflow",
    );

    let engine = unsafe { mdstream_engine_new(ptr::null(), 0) };
    assert_eq!(engine.status, BindingStatus::Ok.code());
    assert!(!engine.engine.is_null());
    assert!(engine.error.data.is_null());

    let reducer = unsafe { mdstream_reducer_new(ptr::null(), 0) };
    assert_eq!(reducer.status, BindingStatus::Ok.code());
    assert!(!reducer.reducer.is_null());
    assert!(reducer.error.data.is_null());
    assert_eq!(
        unsafe { mdstream_reducer_processor_scheduler_limits(reducer.reducer) },
        MdstreamProcessorSchedulerLimits {
            max_in_flight_jobs: 32,
            max_queued_candidates: 256,
        }
    );

    let custom_options = br#"{
        "schema":"mdstream.bindings-options/0.4",
        "processor":{"max_in_flight_jobs":"2","max_slots":"25"}
    }"#;
    let custom = unsafe { mdstream_reducer_new(custom_options.as_ptr(), custom_options.len()) };
    assert_eq!(custom.status, BindingStatus::Ok.code());
    assert!(!custom.reducer.is_null());
    assert_eq!(
        unsafe { mdstream_reducer_processor_scheduler_limits(custom.reducer) },
        MdstreamProcessorSchedulerLimits {
            max_in_flight_jobs: 2,
            max_queued_candidates: 25,
        }
    );
    unsafe { mdstream_reducer_free(custom.reducer) };

    let captured =
        unsafe { mdstream_reducer_new(transition_options.as_ptr(), transition_options.len()) };
    assert_eq!(captured.status, BindingStatus::Ok.code());
    assert!(!captured.reducer.is_null());
    let transition_change = encode_change_json(
        &ChangeSet::start_epoch(
            Epoch::new(1),
            ChangeId::new("ffi:transition:start").unwrap(),
            None,
            SourceDelta::append(SourceCursor::new(0), "A"),
            Vec::new(),
        )
        .unwrap(),
        usize::MAX,
        ProtocolLimits::default(),
    )
    .unwrap();
    let transition_update = take_output(unsafe {
        mdstream_reducer_apply_change(
            captured.reducer,
            transition_change.as_ptr(),
            transition_change.len(),
        )
    });
    assert_eq!(transition_update.len(), 1);
    assert_eq!(
        transition_update[0].0,
        BindingPayloadKind::ReducerUpdate as u32
    );
    let transition_update: serde_json::Value =
        serde_json::from_slice(&transition_update[0].1).unwrap();
    assert_eq!(transition_update["schema"], BINDING_SCHEMA);
    assert_eq!(transition_update["kind"], "reducer_update");
    assert_eq!(transition_update["transition"]["schema"], TRANSITION_SCHEMA);
    let facts: TransitionFacts =
        serde_json::from_value(transition_update["transition"]["facts"].clone()).unwrap();
    assert!(matches!(
        facts,
        TransitionFacts::Continuous { before: None, .. }
    ));
    unsafe { mdstream_reducer_free(captured.reducer) };

    let invalid_utf8 = unsafe { mdstream_engine_append(engine.engine, [0xff].as_ptr(), 1) };
    assert_eq!(invalid_utf8.status, BindingStatus::Utf8.code());
    assert!(invalid_utf8.output.is_null());
    assert_error(
        invalid_utf8.error,
        "MDSTREAM_UTF8_ERROR",
        "bindings.invalid_utf8",
    );

    let source = b"# C ABI\n\nstreamed state\n";
    let append = unsafe { mdstream_engine_append(engine.engine, source.as_ptr(), source.len()) };
    apply_changes(reducer.reducer, append);
    let finish = unsafe { mdstream_engine_execute(engine.engine, FINISH.as_ptr(), FINISH.len()) };
    apply_changes(reducer.reducer, finish);

    let engine_snapshot = unsafe {
        mdstream_engine_execute(
            engine.engine,
            ENGINE_SNAPSHOT.as_ptr(),
            ENGINE_SNAPSHOT.len(),
        )
    };
    assert_snapshot_source(engine_snapshot, source);

    let reducer_snapshot = unsafe {
        mdstream_reducer_execute(
            reducer.reducer,
            REDUCER_SNAPSHOT.as_ptr(),
            REDUCER_SNAPSHOT.len(),
        )
    };
    let recovery_snapshot = assert_snapshot_source(reducer_snapshot, source);

    let replica = unsafe { mdstream_reducer_new(ptr::null(), 0) };
    assert_eq!(replica.status, BindingStatus::Ok.code());
    let recovered = unsafe {
        mdstream_reducer_recover_snapshot(
            replica.reducer,
            recovery_snapshot.as_ptr(),
            recovery_snapshot.len(),
        )
    };
    free_success(recovered);
    let replica_snapshot = unsafe {
        mdstream_reducer_execute(
            replica.reducer,
            REDUCER_SNAPSHOT.as_ptr(),
            REDUCER_SNAPSHOT.len(),
        )
    };
    assert_snapshot_source(replica_snapshot, source);
    unsafe { mdstream_reducer_free(replica.reducer) };

    let terminal = unsafe { mdstream_engine_append(engine.engine, b"late".as_ptr(), 4) };
    assert_eq!(terminal.status, BindingStatus::Terminal.code());
    assert!(terminal.output.is_null());
    assert_error(terminal.error, "MDSTREAM_TERMINAL", "engine.finished");

    let reset = unsafe { mdstream_engine_execute(engine.engine, RESET.as_ptr(), RESET.len()) };
    apply_changes(reducer.reducer, reset);
    let after_reset = unsafe {
        mdstream_reducer_execute(
            reducer.reducer,
            REDUCER_SNAPSHOT.as_ptr(),
            REDUCER_SNAPSHOT.len(),
        )
    };
    assert_snapshot_source(after_reset, b"");

    let bad_command = b"{}";
    let command_error = unsafe {
        mdstream_reducer_execute(reducer.reducer, bad_command.as_ptr(), bad_command.len())
    };
    assert_eq!(command_error.status, BindingStatus::Command.code());
    assert_error(
        command_error.error,
        "MDSTREAM_COMMAND_ERROR",
        "bindings.invalid_command",
    );

    let old_schema = br#"{"schema":"mdstream.bindings/0.3","kind":"snapshot"}"#;
    let schema_error =
        unsafe { mdstream_engine_execute(engine.engine, old_schema.as_ptr(), old_schema.len()) };
    assert_eq!(schema_error.status, BindingStatus::UnsupportedSchema.code());
    assert_error(
        schema_error.error,
        "MDSTREAM_UNSUPPORTED_SCHEMA",
        "bindings.unsupported_command_schema",
    );

    let bounded_options = br#"{
        "schema":"mdstream.bindings-options/0.4",
        "protocol":{"max_source_bytes":"2"},
        "wire":{"max_command_bytes":"4"}
    }"#;
    let bounded = unsafe { mdstream_engine_new(bounded_options.as_ptr(), bounded_options.len()) };
    assert_eq!(bounded.status, BindingStatus::Ok.code());
    assert_eq!(
        unsafe { mdstream_engine_raw_append_byte_ceiling(bounded.engine) },
        4
    );
    let oversized = unsafe { mdstream_engine_append(bounded.engine, b"12345".as_ptr(), 5) };
    assert_eq!(oversized.status, BindingStatus::ResourceLimit.code());
    assert_error(
        oversized.error,
        "MDSTREAM_RESOURCE_LIMIT_EXCEEDED",
        "bindings.resource_limit",
    );
    let retry = unsafe { mdstream_engine_append(bounded.engine, b"ok".as_ptr(), 2) };
    assert_eq!(retry.status, BindingStatus::Ok.code());
    assert_eq!(
        unsafe { mdstream_engine_raw_append_byte_ceiling(bounded.engine) },
        0
    );
    assert!(unsafe { mdstream_output_len(retry.output) } > 0);
    let first = unsafe { mdstream_output_take(retry.output, 0) };
    assert_eq!(first.status, BindingStatus::Ok.code());
    take_buffer(first.data);
    let repeated = unsafe { mdstream_output_take(retry.output, 0) };
    assert_eq!(repeated.status, BindingStatus::InvalidArgument.code());
    assert_error(
        repeated.data,
        "MDSTREAM_INVALID_ARGUMENT",
        "ffi.output_index",
    );
    unsafe { mdstream_output_free(retry.output) };
    unsafe { mdstream_engine_free(bounded.engine) };

    let empty = unsafe { mdstream_engine_append(engine.engine, b"".as_ptr(), 0) };
    assert_eq!(empty.status, BindingStatus::Ok.code());
    assert!(!empty.output.is_null());
    let len = unsafe { mdstream_output_len(empty.output) };
    assert_eq!(unsafe { mdstream_output_remaining(empty.output) }, len);
    let out_of_range = unsafe { mdstream_output_take(empty.output, len) };
    assert_eq!(out_of_range.status, BindingStatus::InvalidArgument.code());
    assert_eq!(out_of_range.kind, 0);
    assert_error(
        out_of_range.data,
        "MDSTREAM_INVALID_ARGUMENT",
        "ffi.output_index",
    );
    unsafe { mdstream_output_free(empty.output) };

    unsafe {
        mdstream_reducer_free(reducer.reducer);
        mdstream_engine_free(engine.engine);
        mdstream_reducer_free(ptr::null_mut());
        mdstream_engine_free(ptr::null_mut());
        mdstream_buffer_free(MdstreamBuffer::empty());
    }
    assert_eq!(mdstream_allocation_metrics(), Default::default());
}

fn apply_changes(reducer: *mut mdstream_ffi::MdstreamReducer, result: MdstreamCallResult) {
    assert_eq!(result.status, BindingStatus::Ok.code());
    assert!(!result.output.is_null());
    assert!(result.error.data.is_null());
    let payloads = take_output(result);
    for (kind, bytes) in payloads {
        assert_eq!(kind, BindingPayloadKind::Change as u32);
        let applied =
            unsafe { mdstream_reducer_apply_change(reducer, bytes.as_ptr(), bytes.len()) };
        assert_eq!(applied.status, BindingStatus::Ok.code());
        free_success(applied);
    }
}

fn assert_snapshot_source(result: MdstreamCallResult, expected: &[u8]) -> Vec<u8> {
    let mut payloads = take_output(result);
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].0, BindingPayloadKind::Snapshot as u32);
    let bytes = payloads.pop().unwrap().1;
    let snapshot = decode_snapshot_json(&bytes, usize::MAX, ProtocolLimits::default())
        .expect("FFI snapshot must decode under the canonical contract");
    assert_eq!(snapshot.source(), std::str::from_utf8(expected).unwrap());
    bytes
}

fn take_output(result: MdstreamCallResult) -> Vec<(u32, Vec<u8>)> {
    assert_eq!(result.status, BindingStatus::Ok.code());
    assert!(!result.output.is_null());
    assert!(result.error.data.is_null());
    let len = unsafe { mdstream_output_len(result.output) };
    let mut payloads = Vec::with_capacity(len);
    for index in 0..len {
        let payload = unsafe { mdstream_output_take(result.output, index) };
        assert_eq!(payload.status, BindingStatus::Ok.code());
        payloads.push((payload.kind, take_buffer(payload.data)));
    }
    assert_eq!(unsafe { mdstream_output_remaining(result.output) }, 0);
    unsafe { mdstream_output_free(result.output) };
    payloads
}

fn assert_error(buffer: MdstreamBuffer, status_name: &str, detail_code: &str) {
    let value: serde_json::Value = serde_json::from_slice(&take_buffer(buffer)).unwrap();
    assert_eq!(value["status_name"], status_name);
    assert_eq!(value["detail_code"], detail_code);
}

fn static_string(pointer: *const std::ffi::c_char) -> &'static str {
    assert!(!pointer.is_null());
    unsafe { CStr::from_ptr(pointer) }.to_str().unwrap()
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
