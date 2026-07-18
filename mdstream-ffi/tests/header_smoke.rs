use std::{fs, path::PathBuf};

#[path = "support/host.rs"]
mod host_support;

use host_support::{TempDir, current_target};

#[test]
fn public_header_compiles_with_exact_constants_layouts_and_function_signatures() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out_dir = TempDir::new("mdstream-ffi-header-smoke");
    let source = out_dir.path().join("header_smoke.c");
    fs::write(
        &source,
        r#"
#include "mdstream.h"

#if MDSTREAM_ABI_VERSION != 1
#error "unexpected ABI version"
#endif

_Static_assert(MDSTREAM_OK == 0, "status drift");
_Static_assert(MDSTREAM_PANIC == 13, "status drift");
_Static_assert(MDSTREAM_PAYLOAD_CHANGE == 1, "payload drift");
_Static_assert(MDSTREAM_PAYLOAD_ARTIFACT_VIEW == 9, "payload drift");
_Static_assert(MDSTREAM_PAYLOAD_PENDING_SOURCE_VIEW == 10, "payload drift");

int mdstream_header_smoke(void) {
    MdstreamBuffer buffer = {0};
    MdstreamAllocationMetrics allocations = {0};
    MdstreamCallResult call = {MDSTREAM_OK, 0, buffer};
    MdstreamEngineResult engine = {MDSTREAM_OK, 0, buffer};
    MdstreamReducerResult reducer = {MDSTREAM_OK, 0, buffer};
    MdstreamPayloadResult payload = {MDSTREAM_OK, 0, buffer};
    uint32_t (*abi_version)(void) = &mdstream_abi_version;
    const char* (*package_version)(void) = &mdstream_package_version;
    const char* (*binding_schema)(void) = &mdstream_binding_schema;
    const char* (*binding_options_schema)(void) = &mdstream_binding_options_schema;
    size_t (*buffer_size)(void) = &mdstream_buffer_struct_size;
    size_t (*call_size)(void) = &mdstream_call_result_struct_size;
    size_t (*engine_result_size)(void) = &mdstream_engine_result_struct_size;
    size_t (*reducer_result_size)(void) = &mdstream_reducer_result_struct_size;
    size_t (*payload_result_size)(void) = &mdstream_payload_result_struct_size;
    size_t (*allocation_size)(void) = &mdstream_allocation_metrics_struct_size;
    MdstreamAllocationMetrics (*allocation_metrics)(void) = &mdstream_allocation_metrics;
    MdstreamEngineResult (*engine_new)(const uint8_t*, size_t) = &mdstream_engine_new;
    void (*engine_free)(MdstreamEngine*) = &mdstream_engine_free;
    MdstreamCallResult (*engine_append)(MdstreamEngine*, const uint8_t*, size_t) = &mdstream_engine_append;
    MdstreamCallResult (*engine_execute)(MdstreamEngine*, const uint8_t*, size_t) = &mdstream_engine_execute;
    MdstreamReducerResult (*reducer_new)(const uint8_t*, size_t) = &mdstream_reducer_new;
    void (*reducer_free)(MdstreamReducer*) = &mdstream_reducer_free;
    MdstreamCallResult (*reducer_apply)(MdstreamReducer*, const uint8_t*, size_t) = &mdstream_reducer_apply_change;
    MdstreamCallResult (*reducer_recover)(MdstreamReducer*, const uint8_t*, size_t) = &mdstream_reducer_recover_snapshot;
    MdstreamCallResult (*reducer_execute)(MdstreamReducer*, const uint8_t*, size_t) = &mdstream_reducer_execute;
    size_t (*output_len)(const MdstreamOutput*) = &mdstream_output_len;
    size_t (*output_remaining)(const MdstreamOutput*) = &mdstream_output_remaining;
    MdstreamPayloadResult (*output_take)(MdstreamOutput*, size_t) = &mdstream_output_take;
    void (*output_free)(MdstreamOutput*) = &mdstream_output_free;
    void (*buffer_free)(MdstreamBuffer) = &mdstream_buffer_free;
    (void)allocations;
    (void)call;
    (void)engine;
    (void)reducer;
    (void)payload;
    (void)abi_version;
    (void)package_version;
    (void)binding_schema;
    (void)binding_options_schema;
    (void)buffer_size;
    (void)call_size;
    (void)engine_result_size;
    (void)reducer_result_size;
    (void)payload_result_size;
    (void)allocation_size;
    (void)allocation_metrics;
    (void)engine_new;
    (void)engine_free;
    (void)engine_append;
    (void)engine_execute;
    (void)reducer_new;
    (void)reducer_free;
    (void)reducer_apply;
    (void)reducer_recover;
    (void)reducer_execute;
    (void)output_len;
    (void)output_remaining;
    (void)output_take;
    (void)output_free;
    (void)buffer_free;
    return (int)(buffer.len + allocations.outputs);
}
"#,
    )
    .unwrap();

    cc::Build::new()
        .target(current_target())
        .host(current_target())
        .flag_if_supported("-std=c11")
        .opt_level(0)
        .include(manifest_dir.join("include"))
        .file(source)
        .out_dir(out_dir.path())
        .try_compile("mdstream_header_smoke")
        .expect("public C header must compile");
}
