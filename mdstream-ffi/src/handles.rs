use std::{panic::AssertUnwindSafe, sync::Mutex};

use mdstream_bindings_core::{
    BindingError, BindingOutput, EngineSession, ProcessorSchedulerLimits, ReducerSession,
};

use crate::{
    buffers::{
        record_engine_created, record_engine_dropped, record_reducer_created,
        record_reducer_dropped,
    },
    errors::{
        MdstreamCallResult, MdstreamEngineResult, MdstreamReducerResult, ffi_call, ffi_engine,
        ffi_reducer, null_handle, poisoned_handle, with_raw_bytes,
    },
};

pub struct MdstreamEngine {
    inner: Mutex<EngineSession>,
}

impl MdstreamEngine {
    fn into_raw(inner: EngineSession) -> *mut Self {
        record_engine_created();
        Box::into_raw(Box::new(Self {
            inner: Mutex::new(inner),
        }))
    }
}

impl Drop for MdstreamEngine {
    fn drop(&mut self) {
        record_engine_dropped();
    }
}

pub struct MdstreamReducer {
    inner: Mutex<ReducerSession>,
    processor_scheduler_limits: MdstreamProcessorSchedulerLimits,
}

impl MdstreamReducer {
    fn into_raw(inner: ReducerSession) -> *mut Self {
        record_reducer_created();
        let processor_scheduler_limits = inner.processor_scheduler_limits().into();
        Box::into_raw(Box::new(Self {
            inner: Mutex::new(inner),
            processor_scheduler_limits,
        }))
    }
}

/// Effective native budgets used by a host-language processor scheduler.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MdstreamProcessorSchedulerLimits {
    pub max_in_flight_jobs: usize,
    pub max_queued_candidates: usize,
}

impl From<ProcessorSchedulerLimits> for MdstreamProcessorSchedulerLimits {
    fn from(limits: ProcessorSchedulerLimits) -> Self {
        Self {
            max_in_flight_jobs: limits.max_in_flight_jobs,
            max_queued_candidates: limits.max_queued_candidates,
        }
    }
}

impl Drop for MdstreamReducer {
    fn drop(&mut self) {
        record_reducer_dropped();
    }
}

/// Creates an engine session from optional binding-options JSON.
///
/// # Safety
///
/// `options_json` may be null only when `options_len` is zero. A non-null
/// pointer must be readable for `options_len` bytes for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_engine_new(
    options_json: *const u8,
    options_len: usize,
) -> MdstreamEngineResult {
    ffi_engine(|| unsafe {
        with_raw_bytes(options_json, options_len, "options_json", |options| {
            EngineSession::new(options).map(MdstreamEngine::into_raw)
        })
    })
}

/// Releases an engine handle. Passing null is a no-op.
///
/// # Safety
///
/// A non-null pointer must be a live handle returned by `mdstream_engine_new`.
/// The caller must wait for all calls to complete and free the handle exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_engine_free(engine: *mut MdstreamEngine) {
    unsafe { crate::errors::drop_opaque(engine) };
}

/// Appends raw UTF-8 bytes without wrapping them in a JSON command.
///
/// # Safety
///
/// `engine` must be a live handle. `chunk` may be null only when `chunk_len`
/// is zero and otherwise must be readable for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_engine_append(
    engine: *mut MdstreamEngine,
    chunk: *const u8,
    chunk_len: usize,
) -> MdstreamCallResult {
    ffi_call(|| unsafe {
        with_raw_bytes(chunk, chunk_len, "chunk", |chunk| {
            with_engine(engine, |session| session.append(chunk))
        })
    })
}

/// Returns the largest raw append input that might fit after newline
/// normalization.
///
/// A finalized engine and a null, poisoned, or panicking handle return
/// `usize::MAX`, meaning no useful local bound is available. Callers must fall
/// through to the ordinary structured append path, which always repeats the
/// authoritative admission check and reports the structured result.
///
/// # Safety
///
/// A non-null pointer must be a live handle returned by `mdstream_engine_new`
/// for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_engine_raw_append_byte_ceiling(
    engine: *const MdstreamEngine,
) -> usize {
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let Some(engine) = (unsafe { engine.as_ref() }) else {
            return usize::MAX;
        };
        engine
            .inner
            .lock()
            .map_or(usize::MAX, |session| session.raw_append_byte_ceiling())
    }))
    .unwrap_or(usize::MAX)
}

/// Executes a versioned engine command for finish, reset, or snapshot paths.
///
/// # Safety
///
/// `engine` must be a live handle. `command_json` may be null only when
/// `command_len` is zero and otherwise must be readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_engine_execute(
    engine: *mut MdstreamEngine,
    command_json: *const u8,
    command_len: usize,
) -> MdstreamCallResult {
    ffi_call(|| unsafe {
        with_raw_bytes(command_json, command_len, "command_json", |command| {
            with_engine(engine, |session| session.execute(command))
        })
    })
}

/// Creates a reducer and processor-host session from optional binding options.
///
/// # Safety
///
/// `options_json` follows the same pointer and length rules as `mdstream_engine_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_reducer_new(
    options_json: *const u8,
    options_len: usize,
) -> MdstreamReducerResult {
    ffi_reducer(|| unsafe {
        with_raw_bytes(options_json, options_len, "options_json", |options| {
            ReducerSession::new(options).map(MdstreamReducer::into_raw)
        })
    })
}

/// Releases a reducer handle. Passing null is a no-op.
///
/// # Safety
///
/// A non-null pointer must be live, all calls must have completed, and the
/// caller must release it exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_reducer_free(reducer: *mut MdstreamReducer) {
    unsafe { crate::errors::drop_opaque(reducer) };
}

/// Returns the immutable processor scheduler limits captured by a reducer.
///
/// A null pointer returns an all-zero value. For a non-null pointer, the
/// returned structure remains valid independently of the reducer handle.
///
/// # Safety
///
/// A non-null pointer must be a live handle returned by `mdstream_reducer_new`.
/// The caller must not race this query with `mdstream_reducer_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_reducer_processor_scheduler_limits(
    reducer: *const MdstreamReducer,
) -> MdstreamProcessorSchedulerLimits {
    std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
        reducer
            .as_ref()
            .map_or_else(MdstreamProcessorSchedulerLimits::default, |reducer| {
                reducer.processor_scheduler_limits
            })
    }))
    .unwrap_or_default()
}

/// Applies one canonical change JSON payload without a command wrapper.
///
/// # Safety
///
/// `reducer` must be live. `change_json` may be null only when `change_len`
/// is zero and otherwise must be readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_reducer_apply_change(
    reducer: *mut MdstreamReducer,
    change_json: *const u8,
    change_len: usize,
) -> MdstreamCallResult {
    ffi_call(|| unsafe {
        with_raw_bytes(change_json, change_len, "change_json", |change| {
            with_reducer(reducer, |session| session.apply_change(change))
        })
    })
}

/// Recovers reducer state from one canonical snapshot JSON payload.
///
/// # Safety
///
/// `reducer` must be live. `snapshot_json` may be null only when
/// `snapshot_len` is zero and otherwise must be readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_reducer_recover_snapshot(
    reducer: *mut MdstreamReducer,
    snapshot_json: *const u8,
    snapshot_len: usize,
) -> MdstreamCallResult {
    ffi_call(|| unsafe {
        with_raw_bytes(snapshot_json, snapshot_len, "snapshot_json", |snapshot| {
            with_reducer(reducer, |session| session.recover_snapshot(snapshot))
        })
    })
}

/// Executes a versioned reducer command for snapshots, views, and processor lifecycle operations.
///
/// # Safety
///
/// `reducer` must be live. `command_json` may be null only when
/// `command_len` is zero and otherwise must be readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mdstream_reducer_execute(
    reducer: *mut MdstreamReducer,
    command_json: *const u8,
    command_len: usize,
) -> MdstreamCallResult {
    ffi_call(|| unsafe {
        with_raw_bytes(command_json, command_len, "command_json", |command| {
            with_reducer(reducer, |session| session.execute(command))
        })
    })
}

unsafe fn with_engine(
    engine: *mut MdstreamEngine,
    operation: impl FnOnce(&mut EngineSession) -> Result<BindingOutput, BindingError>,
) -> Result<BindingOutput, BindingError> {
    let Some(engine) = (unsafe { engine.as_ref() }) else {
        return Err(null_handle("engine"));
    };
    let mut session = engine.inner.lock().map_err(|_| poisoned_handle("engine"))?;
    operation(&mut session)
}

unsafe fn with_reducer(
    reducer: *mut MdstreamReducer,
    operation: impl FnOnce(&mut ReducerSession) -> Result<BindingOutput, BindingError>,
) -> Result<BindingOutput, BindingError> {
    let Some(reducer) = (unsafe { reducer.as_ref() }) else {
        return Err(null_handle("reducer"));
    };
    let mut session = reducer
        .inner
        .lock()
        .map_err(|_| poisoned_handle("reducer"))?;
    operation(&mut session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        mdstream_allocation_metrics, mdstream_buffer_free, mdstream_engine_execute,
        mdstream_engine_free,
    };
    use mdstream_bindings_core::BindingStatus;

    #[test]
    fn panic_while_holding_a_session_lock_is_contained_and_poisoned() {
        let engine = MdstreamEngine::into_raw(EngineSession::new(b"").unwrap());
        let panic = ffi_call(|| unsafe {
            with_engine(engine, |_session| -> Result<BindingOutput, BindingError> {
                panic!("injected session panic")
            })
        });
        assert_eq!(panic.status, BindingStatus::Panic.code());
        assert!(panic.output.is_null());
        unsafe { mdstream_buffer_free(panic.error) };

        let command = br#"{"schema":"mdstream.bindings/0.4","kind":"snapshot"}"#;
        let poisoned = unsafe { mdstream_engine_execute(engine, command.as_ptr(), command.len()) };
        assert_eq!(poisoned.status, BindingStatus::Internal.code());
        assert!(poisoned.output.is_null());
        unsafe {
            mdstream_buffer_free(poisoned.error);
            mdstream_engine_free(engine);
        }
        assert_eq!(mdstream_allocation_metrics(), Default::default());
    }
}
