/*
 * mdstream.h - stable C ABI for the mdstream streaming content engine.
 *
 * All inputs and payloads are byte slices. Text and JSON payloads are UTF-8.
 * Every non-empty owned buffer returned by Rust must be released exactly once
 * with mdstream_buffer_free.
 */

#ifndef MDSTREAM_H
#define MDSTREAM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define MDSTREAM_ABI_VERSION 1

enum {
    MDSTREAM_OK = 0,
    MDSTREAM_INVALID_ARGUMENT = 1,
    MDSTREAM_UTF8_ERROR = 2,
    MDSTREAM_OPTIONS_ERROR = 3,
    MDSTREAM_COMMAND_ERROR = 4,
    MDSTREAM_UNSUPPORTED_SCHEMA = 5,
    MDSTREAM_TERMINAL = 6,
    MDSTREAM_ENGINE_ERROR = 7,
    MDSTREAM_PROTOCOL_ERROR = 8,
    MDSTREAM_NEEDS_SNAPSHOT = 9,
    MDSTREAM_PROCESSOR_ERROR = 10,
    MDSTREAM_RESOURCE_LIMIT_EXCEEDED = 11,
    MDSTREAM_INTERNAL_ERROR = 12,
    MDSTREAM_PANIC = 13
};

enum {
    MDSTREAM_PAYLOAD_CHANGE = 1,
    MDSTREAM_PAYLOAD_SNAPSHOT = 2,
    MDSTREAM_PAYLOAD_REDUCER_UPDATE = 3,
    MDSTREAM_PAYLOAD_NODE_VIEW = 4,
    MDSTREAM_PAYLOAD_RESOURCE_VIEW = 5,
    MDSTREAM_PAYLOAD_PROCESSOR_REQUEST = 6,
    MDSTREAM_PAYLOAD_PROCESSOR_COMPLETION = 7,
    MDSTREAM_PAYLOAD_ARTIFACT_CHANGE = 8,
    MDSTREAM_PAYLOAD_ARTIFACT_VIEW = 9,
    MDSTREAM_PAYLOAD_PENDING_SOURCE_VIEW = 10
};

/* Owned by Rust. Empty buffers are always { NULL, 0 }. */
typedef struct MdstreamBuffer {
    uint8_t* data;
    size_t len;
} MdstreamBuffer;

typedef struct MdstreamAllocationMetrics {
    uint64_t engine_handles;
    uint64_t reducer_handles;
    uint64_t outputs;
    uint64_t buffers;
    uint64_t buffer_bytes;
} MdstreamAllocationMetrics;

/* Immutable native budgets for a host-language processor scheduler. */
typedef struct MdstreamProcessorSchedulerLimits {
    size_t max_in_flight_jobs;
    size_t max_queued_candidates;
} MdstreamProcessorSchedulerLimits;

typedef struct MdstreamEngine MdstreamEngine;
typedef struct MdstreamReducer MdstreamReducer;
typedef struct MdstreamOutput MdstreamOutput;

/*
 * status == MDSTREAM_OK: output is non-null and error is empty.
 * status != MDSTREAM_OK: output is null and error contains owned JSON bytes.
 */
typedef struct MdstreamCallResult {
    int32_t status;
    MdstreamOutput* output;
    MdstreamBuffer error;
} MdstreamCallResult;

/*
 * status == MDSTREAM_OK: engine is non-null and error is empty.
 * status != MDSTREAM_OK: engine is null and error contains owned JSON bytes.
 */
typedef struct MdstreamEngineResult {
    int32_t status;
    MdstreamEngine* engine;
    MdstreamBuffer error;
} MdstreamEngineResult;

/* Constructor invariants match MdstreamEngineResult. */
typedef struct MdstreamReducerResult {
    int32_t status;
    MdstreamReducer* reducer;
    MdstreamBuffer error;
} MdstreamReducerResult;

/*
 * status == MDSTREAM_OK: kind is a MDSTREAM_PAYLOAD_* value and data is the
 * owned payload. On error kind is zero and data is owned JSON error bytes.
 */
typedef struct MdstreamPayloadResult {
    int32_t status;
    uint32_t kind;
    MdstreamBuffer data;
} MdstreamPayloadResult;

/* ABI and schema probes. Static strings are Rust-owned and must not be freed. */
uint32_t mdstream_abi_version(void);
const char* mdstream_package_version(void);
const char* mdstream_binding_schema(void);
const char* mdstream_binding_options_schema(void);
const char* mdstream_transition_schema(void);
size_t mdstream_buffer_struct_size(void);
size_t mdstream_call_result_struct_size(void);
size_t mdstream_engine_result_struct_size(void);
size_t mdstream_reducer_result_struct_size(void);
size_t mdstream_payload_result_struct_size(void);
size_t mdstream_allocation_metrics_struct_size(void);
size_t mdstream_processor_scheduler_limits_struct_size(void);

/* Process-wide diagnostic snapshot used by binding ownership tests. */
MdstreamAllocationMetrics mdstream_allocation_metrics(void);

/*
 * Create mutable stateful sessions. options_json uses mdstream.bindings-options/0.4.
 * NULL/0 selects defaults; NULL with a non-zero length is invalid.
 */
MdstreamEngineResult mdstream_engine_new(
    const uint8_t* options_json,
    size_t options_len
);
MdstreamReducerResult mdstream_reducer_new(
    const uint8_t* options_json,
    size_t options_len
);

/*
 * Free consumes a live handle exactly once. NULL is a no-op. Hosts must wait
 * for every call on a handle before freeing it. Concurrent use/free,
 * use-after-free, arbitrary pointers, and double-free are caller violations.
 */
void mdstream_engine_free(MdstreamEngine* engine);
void mdstream_reducer_free(MdstreamReducer* reducer);

/* NULL returns { 0, 0 }; a non-null reducer must remain live for the call. */
MdstreamProcessorSchedulerLimits mdstream_reducer_processor_scheduler_limits(
    const MdstreamReducer* reducer
);

/* Hot paths avoid an additional JSON command wrapper. */
MdstreamCallResult mdstream_engine_append(
    MdstreamEngine* engine,
    const uint8_t* chunk,
    size_t chunk_len
);
MdstreamCallResult mdstream_reducer_apply_change(
    MdstreamReducer* reducer,
    const uint8_t* change_json,
    size_t change_len
);
MdstreamCallResult mdstream_reducer_recover_snapshot(
    MdstreamReducer* reducer,
    const uint8_t* snapshot_json,
    size_t snapshot_len
);

/*
 * Versioned command paths cover finish/reset/snapshot and reducer views plus
 * processor lifecycle operations. Commands use mdstream.bindings/0.4.
 */
MdstreamCallResult mdstream_engine_execute(
    MdstreamEngine* engine,
    const uint8_t* command_json,
    size_t command_len
);
MdstreamCallResult mdstream_reducer_execute(
    MdstreamReducer* reducer,
    const uint8_t* command_json,
    size_t command_len
);

/*
 * Calls on one live engine or reducer are serialized by the library. No
 * foreign callback runs while either session lock is held.
 */

/*
 * Output payloads preserve binding-core order. Each index may be taken once.
 * Freeing an output releases every payload that was not taken. NULL free is a
 * no-op; a non-null output must be freed exactly once after all accesses end.
 */
size_t mdstream_output_len(const MdstreamOutput* output);
size_t mdstream_output_remaining(const MdstreamOutput* output);
MdstreamPayloadResult mdstream_output_take(MdstreamOutput* output, size_t index);
void mdstream_output_free(MdstreamOutput* output);

/*
 * Free an owned result or payload buffer. { NULL, 0 } is a no-op. Do not use
 * free(), delete, or any host allocator for Rust-owned bytes.
 */
void mdstream_buffer_free(MdstreamBuffer buffer);

#ifdef __cplusplus
}
#endif

#endif /* MDSTREAM_H */
