# mdstream-ffi

`mdstream-ffi` exposes the final mdstream 0.4 binding contract through a small
C ABI. Canonical parsing, reduction, recovery, and processor state remain in
the safe `mdstream-bindings-core` facade; this crate owns only native handles,
byte ownership, panic containment, and C-compatible metadata.

## Build

```sh
cargo build -p mdstream-ffi --release
```

The crate produces `cdylib`, `staticlib`, and `rlib` artifacts. C and
C-compatible hosts include `include/mdstream.h`. Native FFI builds must use
Rust's unwind panic strategy; an aborting panic strategy cannot satisfy the ABI
panic-containment contract and is rejected at compile time.

## Contract

- Compare `mdstream_abi_version()` with `MDSTREAM_ABI_VERSION` and verify the
  result-structure sizes before creating sessions.
- `mdstream_engine_append`, `mdstream_reducer_apply_change`, and
  `mdstream_reducer_recover_snapshot` are unwrapped byte hot paths.
- `mdstream_engine_execute` carries finish, reset, snapshot, and cold append
  commands using `mdstream.bindings/0.4`.
- `mdstream_reducer_execute` carries snapshot, view, and processor lifecycle
  commands using the same versioned schema.
- Successful calls return a non-null `MdstreamOutput`. Payload order and kinds
  match `mdstream-bindings-core`; each index may be taken once.
- Errors use the stable `MDSTREAM_*` status table and an owned JSON envelope.

## Ownership

Every non-empty `MdstreamBuffer` returned by Rust must be released exactly once
with `mdstream_buffer_free`. Do not use `free`, `delete`, or a host allocator.

`mdstream_output_take` transfers one payload into an owned buffer. Releasing an
output with `mdstream_output_free` drops every payload that was not taken.
Engine, reducer, and output handles are exact-once resources; null free calls
are no-ops.

## Threading And Safety

Calls on one live engine or reducer are serialized by a per-handle mutex. Calls
on different handles do not share a global lock. Hosts must wait for all calls
on a handle before releasing it.

The following are caller violations and may cause undefined behavior:

- racing a handle call with its free function;
- using a handle or output after it was freed;
- freeing a non-null handle, output, or buffer more than once;
- passing an arbitrary non-null pointer;
- passing a non-null input pointer that is not readable for its declared
  length.

`NULL/0` input pairs represent an empty byte slice. `NULL` with a non-zero
length returns `MDSTREAM_INVALID_ARGUMENT`. Rust panics from behavior entry
points are caught and returned as `MDSTREAM_PANIC`; allocation failure is not a
recoverable ABI error.

## Minimal Flow

```c
#include "mdstream.h"

int main(void) {
    MdstreamEngineResult engine = mdstream_engine_new(NULL, 0);
    MdstreamReducerResult reducer = mdstream_reducer_new(NULL, 0);
    if (engine.status != MDSTREAM_OK || reducer.status != MDSTREAM_OK) {
        mdstream_buffer_free(engine.error);
        mdstream_buffer_free(reducer.error);
        mdstream_engine_free(engine.engine);
        mdstream_reducer_free(reducer.reducer);
        return 1;
    }

    static const uint8_t chunk[] = "# Hello\n";
    MdstreamCallResult produced = mdstream_engine_append(
        engine.engine,
        chunk,
        sizeof(chunk) - 1
    );
    if (produced.status != MDSTREAM_OK) {
        mdstream_buffer_free(produced.error);
        mdstream_reducer_free(reducer.reducer);
        mdstream_engine_free(engine.engine);
        return 2;
    }

    for (size_t index = 0; index < mdstream_output_len(produced.output); ++index) {
        MdstreamPayloadResult payload = mdstream_output_take(produced.output, index);
        if (payload.status == MDSTREAM_OK && payload.kind == MDSTREAM_PAYLOAD_CHANGE) {
            MdstreamCallResult applied = mdstream_reducer_apply_change(
                reducer.reducer,
                payload.data.data,
                payload.data.len
            );
            if (applied.status == MDSTREAM_OK) {
                mdstream_output_free(applied.output);
            } else {
                mdstream_buffer_free(applied.error);
            }
        }
        mdstream_buffer_free(payload.data);
    }

    mdstream_output_free(produced.output);
    mdstream_reducer_free(reducer.reducer);
    mdstream_engine_free(engine.engine);
    return 0;
}
```

Production hosts must check every constructor and call status before reading
the success field. See `include/mdstream.h` and `tests/c_consumer_smoke.c` for
the complete result invariants and a dynamic/static consumer flow.
