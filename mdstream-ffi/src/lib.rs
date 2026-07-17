#![deny(unsafe_op_in_unsafe_fn)]

//! Stable C ABI transport for mdstream binding sessions.
//!
//! Canonical state and wire behavior remain in `mdstream-bindings-core`. This
//! crate owns only opaque handles, byte-buffer ownership, panic containment,
//! and C-compatible ABI metadata.

#[cfg(panic = "abort")]
compile_error!("mdstream-ffi requires panic=unwind to contain panics at the C ABI boundary");

mod buffers;
mod errors;
mod handles;

use std::{ffi::c_char, mem::size_of, panic::AssertUnwindSafe};

pub use buffers::{
    MdstreamAllocationMetrics, MdstreamBuffer, MdstreamOutput, MdstreamPayloadResult,
};
pub use errors::{MdstreamCallResult, MdstreamEngineResult, MdstreamReducerResult};
pub use handles::{
    MdstreamEngine, MdstreamReducer, mdstream_engine_append, mdstream_engine_execute,
    mdstream_engine_free, mdstream_engine_new, mdstream_reducer_apply_change,
    mdstream_reducer_execute, mdstream_reducer_free, mdstream_reducer_new,
    mdstream_reducer_recover_snapshot,
};

pub const MDSTREAM_ABI_VERSION: u32 = 1;

const PACKAGE_VERSION: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();
const BINDING_SCHEMA: &[u8] = b"mdstream.bindings/0.4\0";
const BINDING_OPTIONS_SCHEMA: &[u8] = b"mdstream.bindings-options/0.4\0";

#[unsafe(no_mangle)]
pub extern "C" fn mdstream_abi_version() -> u32 {
    MDSTREAM_ABI_VERSION
}

/// Returns a static null-terminated package version owned by Rust.
#[unsafe(no_mangle)]
pub extern "C" fn mdstream_package_version() -> *const c_char {
    PACKAGE_VERSION.as_ptr().cast()
}

/// Returns the final binding schema as a static null-terminated string.
#[unsafe(no_mangle)]
pub extern "C" fn mdstream_binding_schema() -> *const c_char {
    BINDING_SCHEMA.as_ptr().cast()
}

/// Returns the final binding-options schema as a static null-terminated string.
#[unsafe(no_mangle)]
pub extern "C" fn mdstream_binding_options_schema() -> *const c_char {
    BINDING_OPTIONS_SCHEMA.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub extern "C" fn mdstream_buffer_struct_size() -> usize {
    size_of::<MdstreamBuffer>()
}

#[unsafe(no_mangle)]
pub extern "C" fn mdstream_call_result_struct_size() -> usize {
    size_of::<MdstreamCallResult>()
}

#[unsafe(no_mangle)]
pub extern "C" fn mdstream_engine_result_struct_size() -> usize {
    size_of::<MdstreamEngineResult>()
}

#[unsafe(no_mangle)]
pub extern "C" fn mdstream_reducer_result_struct_size() -> usize {
    size_of::<MdstreamReducerResult>()
}

#[unsafe(no_mangle)]
pub extern "C" fn mdstream_payload_result_struct_size() -> usize {
    size_of::<MdstreamPayloadResult>()
}

#[unsafe(no_mangle)]
pub extern "C" fn mdstream_allocation_metrics_struct_size() -> usize {
    size_of::<MdstreamAllocationMetrics>()
}

/// Returns process-wide live allocations owned by this FFI crate.
#[unsafe(no_mangle)]
pub extern "C" fn mdstream_allocation_metrics() -> MdstreamAllocationMetrics {
    buffers::allocation_metrics()
}

/// Returns the original payload count, or zero for a null output pointer.
///
/// # Safety
///
/// A non-null `output` must be a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_output_len(output: *const MdstreamOutput) -> usize {
    std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        output.as_ref().map_or(0, MdstreamOutput::len)
    }))
    .unwrap_or(0)
}

/// Returns the number of payloads not yet taken, or zero for a null pointer.
///
/// # Safety
///
/// A non-null `output` must be a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_output_remaining(output: *const MdstreamOutput) -> usize {
    std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        output.as_ref().map_or(0, MdstreamOutput::remaining)
    }))
    .unwrap_or(0)
}

/// Moves one payload out of an output handle.
///
/// Successful payload bytes and error bytes are owned buffers that must be
/// released with `mdstream_buffer_free`.
///
/// # Safety
///
/// `output` must be a live pointer returned by this library. Calls that mutate
/// one output must not overlap, and an index may be taken at most once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_output_take(
    output: *mut MdstreamOutput,
    index: usize,
) -> MdstreamPayloadResult {
    errors::ffi_payload(|| unsafe { buffers::with_output_mut(output, |output| output.take(index)) })
}

/// Releases an output and every payload that has not been taken.
///
/// Passing null is a no-op.
///
/// # Safety
///
/// A non-null pointer must be live, must have been returned by this library,
/// and must be released exactly once after all accesses have completed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_output_free(output: *mut MdstreamOutput) {
    unsafe { errors::drop_opaque(output) };
}

/// Releases an owned byte buffer returned by this library.
///
/// Passing `{ NULL, 0 }` is a no-op.
///
/// # Safety
///
/// A non-empty buffer must have been returned by this library and must be
/// released exactly once. Host allocators must not release Rust-owned buffers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_buffer_free(buffer: MdstreamBuffer) {
    let _ = std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        buffers::free_buffer(buffer);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdstream_bindings_core::{BINDING_OPTIONS_SCHEMA, BINDING_SCHEMA, BindingOutput};

    #[test]
    fn static_schema_probes_track_the_safe_binding_facade() {
        assert_eq!(
            &super::BINDING_SCHEMA[..super::BINDING_SCHEMA.len() - 1],
            BINDING_SCHEMA.as_bytes()
        );
        assert_eq!(
            &super::BINDING_OPTIONS_SCHEMA[..super::BINDING_OPTIONS_SCHEMA.len() - 1],
            BINDING_OPTIONS_SCHEMA.as_bytes()
        );
    }

    #[test]
    fn panic_boundary_returns_a_structured_error_without_unwinding() {
        let result = errors::ffi_call(|| -> Result<BindingOutput, _> { panic!("boom") });
        assert_eq!(
            result.status,
            mdstream_bindings_core::BindingStatus::Panic.code()
        );
        assert!(result.output.is_null());
        unsafe { mdstream_buffer_free(result.error) };
    }
}
