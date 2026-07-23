#include "mdstream.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int contains(MdstreamBuffer buffer, const char* needle) {
    size_t needle_len = strlen(needle);
    if (needle_len == 0) {
        return 1;
    }
    if (buffer.data == NULL || buffer.len < needle_len) {
        return 0;
    }
    for (size_t i = 0; i <= buffer.len - needle_len; ++i) {
        if (memcmp(buffer.data + i, needle, needle_len) == 0) {
            return 1;
        }
    }
    return 0;
}

static int free_success(MdstreamCallResult result) {
    if (result.status != MDSTREAM_OK || result.output == NULL) {
        mdstream_buffer_free(result.error);
        return 1;
    }
    if (result.error.data != NULL || result.error.len != 0) {
        mdstream_buffer_free(result.error);
        mdstream_output_free(result.output);
        return 2;
    }
    mdstream_output_free(result.output);
    return 0;
}

static int apply_changes(MdstreamReducer* reducer, MdstreamCallResult produced) {
    if (produced.status != MDSTREAM_OK || produced.output == NULL) {
        mdstream_buffer_free(produced.error);
        return 10;
    }
    size_t count = mdstream_output_len(produced.output);
    for (size_t index = 0; index < count; ++index) {
        MdstreamPayloadResult payload = mdstream_output_take(produced.output, index);
        if (payload.status != MDSTREAM_OK) {
            mdstream_buffer_free(payload.data);
            mdstream_output_free(produced.output);
            return 11;
        }
        if (payload.kind != MDSTREAM_PAYLOAD_CHANGE) {
            mdstream_buffer_free(payload.data);
            mdstream_output_free(produced.output);
            return 12;
        }
        MdstreamCallResult applied = mdstream_reducer_apply_change(
            reducer,
            payload.data.data,
            payload.data.len
        );
        mdstream_buffer_free(payload.data);
        if (free_success(applied) != 0) {
            mdstream_output_free(produced.output);
            return 13;
        }
    }
    if (mdstream_output_remaining(produced.output) != 0) {
        mdstream_output_free(produced.output);
        return 14;
    }
    mdstream_output_free(produced.output);
    return 0;
}

static int snapshot_contains(MdstreamReducer* reducer, const char* needle) {
    static const uint8_t command[] =
        "{\"schema\":\"mdstream.bindings/0.4\",\"kind\":\"snapshot\"}";
    MdstreamCallResult result = mdstream_reducer_execute(
        reducer,
        command,
        sizeof(command) - 1
    );
    if (result.status != MDSTREAM_OK || result.output == NULL) {
        mdstream_buffer_free(result.error);
        return 20;
    }
    if (mdstream_output_len(result.output) != 1) {
        mdstream_output_free(result.output);
        return 21;
    }
    MdstreamPayloadResult payload = mdstream_output_take(result.output, 0);
    if (
        payload.status != MDSTREAM_OK ||
        payload.kind != MDSTREAM_PAYLOAD_SNAPSHOT ||
        !contains(payload.data, needle)
    ) {
        mdstream_buffer_free(payload.data);
        mdstream_output_free(result.output);
        return 22;
    }
    mdstream_buffer_free(payload.data);
    mdstream_output_free(result.output);
    return 0;
}

static int allocations_are_zero(void) {
    MdstreamAllocationMetrics metrics = mdstream_allocation_metrics();
    return
        metrics.engine_handles == 0 &&
        metrics.reducer_handles == 0 &&
        metrics.outputs == 0 &&
        metrics.buffers == 0 &&
        metrics.buffer_bytes == 0;
}

int main(void) {
    static const uint8_t source[] = "# C consumer\n\nstreamed state\n";
    static const uint8_t finish[] =
        "{\"schema\":\"mdstream.bindings/0.4\",\"kind\":\"finish\"}";
    MdstreamCallResult (*recover_snapshot)(
        MdstreamReducer*,
        const uint8_t*,
        size_t
    ) = &mdstream_reducer_recover_snapshot;

    if (
        mdstream_abi_version() != MDSTREAM_ABI_VERSION ||
        MDSTREAM_PAYLOAD_REDUCER_UPDATE != 3 ||
        recover_snapshot == NULL
    ) {
        return 1;
    }
    if (
        mdstream_package_version() == NULL ||
        strcmp(mdstream_binding_schema(), "mdstream.bindings/0.4") != 0 ||
        strcmp(mdstream_binding_options_schema(), "mdstream.bindings-options/0.4") != 0 ||
        strcmp(mdstream_transition_schema(), "mdstream.transitions/1") != 0
    ) {
        return 2;
    }
    if (
        mdstream_buffer_struct_size() != sizeof(MdstreamBuffer) ||
        mdstream_call_result_struct_size() != sizeof(MdstreamCallResult) ||
        mdstream_engine_result_struct_size() != sizeof(MdstreamEngineResult) ||
        mdstream_reducer_result_struct_size() != sizeof(MdstreamReducerResult) ||
        mdstream_payload_result_struct_size() != sizeof(MdstreamPayloadResult) ||
        mdstream_allocation_metrics_struct_size() != sizeof(MdstreamAllocationMetrics) ||
        mdstream_processor_scheduler_limits_struct_size() !=
            sizeof(MdstreamProcessorSchedulerLimits)
    ) {
        return 3;
    }
    MdstreamProcessorSchedulerLimits null_limits =
        mdstream_reducer_processor_scheduler_limits(NULL);
    if (
        null_limits.max_in_flight_jobs != 0 ||
        null_limits.max_queued_candidates != 0
    ) {
        return 4;
    }
    if (!allocations_are_zero()) {
        return 5;
    }

    MdstreamEngineResult engine = mdstream_engine_new(NULL, 0);
    MdstreamReducerResult reducer = mdstream_reducer_new(NULL, 0);
    if (
        engine.status != MDSTREAM_OK || engine.engine == NULL ||
        reducer.status != MDSTREAM_OK || reducer.reducer == NULL
    ) {
        mdstream_buffer_free(engine.error);
        mdstream_buffer_free(reducer.error);
        return 6;
    }
    MdstreamProcessorSchedulerLimits default_limits =
        mdstream_reducer_processor_scheduler_limits(reducer.reducer);
    if (
        default_limits.max_in_flight_jobs != 32 ||
        default_limits.max_queued_candidates != 256
    ) {
        mdstream_reducer_free(reducer.reducer);
        mdstream_engine_free(engine.engine);
        return 7;
    }

    static const uint8_t custom_options[] =
        "{\"schema\":\"mdstream.bindings-options/0.4\","
        "\"processor\":{\"max_in_flight_jobs\":\"2\",\"max_slots\":\"25\"}}";
    MdstreamReducerResult custom = mdstream_reducer_new(
        custom_options,
        sizeof(custom_options) - 1
    );
    if (custom.status != MDSTREAM_OK || custom.reducer == NULL) {
        mdstream_buffer_free(custom.error);
        mdstream_reducer_free(reducer.reducer);
        mdstream_engine_free(engine.engine);
        return 8;
    }
    MdstreamProcessorSchedulerLimits custom_limits =
        mdstream_reducer_processor_scheduler_limits(custom.reducer);
    if (
        custom_limits.max_in_flight_jobs != 2 ||
        custom_limits.max_queued_candidates != 25
    ) {
        mdstream_reducer_free(custom.reducer);
        mdstream_reducer_free(reducer.reducer);
        mdstream_engine_free(engine.engine);
        return 9;
    }
    mdstream_reducer_free(custom.reducer);

    static const uint8_t invalid_utf8[] = {0xff};
    MdstreamCallResult error = mdstream_engine_append(
        engine.engine,
        invalid_utf8,
        sizeof(invalid_utf8)
    );
    if (
        error.status != MDSTREAM_UTF8_ERROR ||
        error.output != NULL ||
        !contains(error.error, "MDSTREAM_UTF8_ERROR")
    ) {
        mdstream_buffer_free(error.error);
        return 30;
    }
    mdstream_buffer_free(error.error);

    int rc = apply_changes(
        reducer.reducer,
        mdstream_engine_append(engine.engine, source, sizeof(source) - 1)
    );
    if (rc != 0) {
        return rc;
    }
    rc = apply_changes(
        reducer.reducer,
        mdstream_engine_execute(engine.engine, finish, sizeof(finish) - 1)
    );
    if (rc != 0) {
        return rc;
    }
    rc = snapshot_contains(reducer.reducer, "# C consumer");
    if (rc != 0) {
        return rc;
    }

    MdstreamCallResult terminal = mdstream_engine_append(
        engine.engine,
        invalid_utf8,
        sizeof(invalid_utf8)
    );
    if (
        terminal.status != MDSTREAM_TERMINAL ||
        terminal.output != NULL ||
        !contains(terminal.error, "MDSTREAM_TERMINAL") ||
        mdstream_engine_raw_append_byte_ceiling(engine.engine) != SIZE_MAX
    ) {
        mdstream_buffer_free(terminal.error);
        return 32;
    }
    mdstream_buffer_free(terminal.error);

    mdstream_reducer_free(reducer.reducer);
    mdstream_engine_free(engine.engine);
    mdstream_reducer_free(NULL);
    mdstream_engine_free(NULL);
    MdstreamBuffer empty = {0};
    mdstream_buffer_free(empty);

    if (!allocations_are_zero()) {
        return 31;
    }
    return 0;
}
