use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

use mdstream_bindings_core::{BindingError, BindingOutput, BindingPayloadKind, BindingStatus};

use crate::{
    buffers::{
        MdstreamBuffer, MdstreamOutput, MdstreamPayloadResult, buffer_from_vec, error_buffer,
    },
    handles::{MdstreamEngine, MdstreamReducer},
};

#[repr(C)]
#[derive(Debug)]
pub struct MdstreamCallResult {
    pub status: i32,
    pub output: *mut MdstreamOutput,
    pub error: MdstreamBuffer,
}

#[repr(C)]
#[derive(Debug)]
pub struct MdstreamEngineResult {
    pub status: i32,
    pub engine: *mut MdstreamEngine,
    pub error: MdstreamBuffer,
}

#[repr(C)]
#[derive(Debug)]
pub struct MdstreamReducerResult {
    pub status: i32,
    pub reducer: *mut MdstreamReducer,
    pub error: MdstreamBuffer,
}

pub(crate) fn ffi_call<F>(operation: F) -> MdstreamCallResult
where
    F: FnOnce() -> Result<BindingOutput, BindingError>,
{
    match catch_operation(operation) {
        Ok(output) => MdstreamCallResult {
            status: BindingStatus::Ok.code(),
            output: MdstreamOutput::from_binding(output),
            error: MdstreamBuffer::empty(),
        },
        Err(error) => call_error(error),
    }
}

pub(crate) fn ffi_engine<F>(operation: F) -> MdstreamEngineResult
where
    F: FnOnce() -> Result<*mut MdstreamEngine, BindingError>,
{
    match catch_operation(operation) {
        Ok(engine) => MdstreamEngineResult {
            status: BindingStatus::Ok.code(),
            engine,
            error: MdstreamBuffer::empty(),
        },
        Err(error) => MdstreamEngineResult {
            status: error.status().code(),
            engine: ptr::null_mut(),
            error: error_buffer(&error),
        },
    }
}

pub(crate) fn ffi_reducer<F>(operation: F) -> MdstreamReducerResult
where
    F: FnOnce() -> Result<*mut MdstreamReducer, BindingError>,
{
    match catch_operation(operation) {
        Ok(reducer) => MdstreamReducerResult {
            status: BindingStatus::Ok.code(),
            reducer,
            error: MdstreamBuffer::empty(),
        },
        Err(error) => MdstreamReducerResult {
            status: error.status().code(),
            reducer: ptr::null_mut(),
            error: error_buffer(&error),
        },
    }
}

pub(crate) fn ffi_payload<F>(operation: F) -> MdstreamPayloadResult
where
    F: FnOnce() -> Result<(BindingPayloadKind, Vec<u8>), BindingError>,
{
    match catch_operation(operation) {
        Ok((kind, bytes)) => MdstreamPayloadResult {
            status: BindingStatus::Ok.code(),
            kind: kind as u32,
            data: buffer_from_vec(bytes),
        },
        Err(error) => payload_error(error),
    }
}

pub(crate) unsafe fn with_raw_bytes<T>(
    data: *const u8,
    len: usize,
    field: &'static str,
    operation: impl FnOnce(&[u8]) -> Result<T, BindingError>,
) -> Result<T, BindingError> {
    if data.is_null() {
        if len == 0 {
            return operation(&[]);
        }
        return Err(BindingError::new(
            BindingStatus::InvalidArgument,
            "ffi.null_pointer",
            format!("{field} pointer is null but length is {len}"),
        ));
    }
    if len == 0 {
        return operation(&[]);
    }
    if len > isize::MAX as usize {
        return Err(BindingError::new(
            BindingStatus::InvalidArgument,
            "ffi.length_overflow",
            format!("{field} length exceeds the addressable slice domain"),
        ));
    }
    operation(unsafe { std::slice::from_raw_parts(data, len) })
}

pub(crate) unsafe fn drop_opaque<T>(value: *mut T) {
    let Some(value) = ptr::NonNull::new(value) else {
        return;
    };
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(value.as_ptr()));
    }));
}

pub(crate) fn null_handle(kind: &'static str) -> BindingError {
    BindingError::new(
        BindingStatus::InvalidArgument,
        "ffi.null_handle",
        format!("{kind} pointer is null"),
    )
}

pub(crate) fn poisoned_handle(kind: &'static str) -> BindingError {
    BindingError::new(
        BindingStatus::Internal,
        "ffi.poisoned_handle",
        format!("{kind} session lock is poisoned"),
    )
}

fn call_error(error: BindingError) -> MdstreamCallResult {
    MdstreamCallResult {
        status: error.status().code(),
        output: ptr::null_mut(),
        error: error_buffer(&error),
    }
}

fn payload_error(error: BindingError) -> MdstreamPayloadResult {
    MdstreamPayloadResult {
        status: error.status().code(),
        kind: 0,
        data: error_buffer(&error),
    }
}

fn panic_error() -> BindingError {
    BindingError::new(
        BindingStatus::Panic,
        "ffi.panic",
        "panic caught at mdstream FFI boundary",
    )
}

fn catch_operation<T>(
    operation: impl FnOnce() -> Result<T, BindingError>,
) -> Result<T, BindingError> {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(result) => result,
        Err(_) => Err(panic_error()),
    }
}
