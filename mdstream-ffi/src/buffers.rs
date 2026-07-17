use std::{
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

use mdstream_bindings_core::{
    BindingError, BindingOutput, BindingPayload, BindingPayloadKind, BindingStatus,
    error_payload_json_bytes,
};

static ENGINE_HANDLES: AtomicUsize = AtomicUsize::new(0);
static REDUCER_HANDLES: AtomicUsize = AtomicUsize::new(0);
static OUTPUTS: AtomicUsize = AtomicUsize::new(0);
static BUFFERS: AtomicUsize = AtomicUsize::new(0);
static BUFFER_BYTES: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub struct MdstreamBuffer {
    pub data: *mut u8,
    pub len: usize,
}

impl MdstreamBuffer {
    pub const fn empty() -> Self {
        Self {
            data: ptr::null_mut(),
            len: 0,
        }
    }
}

impl Default for MdstreamBuffer {
    fn default() -> Self {
        Self::empty()
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MdstreamAllocationMetrics {
    pub engine_handles: u64,
    pub reducer_handles: u64,
    pub outputs: u64,
    pub buffers: u64,
    pub buffer_bytes: u64,
}

#[repr(C)]
#[derive(Debug)]
pub struct MdstreamPayloadResult {
    pub status: i32,
    pub kind: u32,
    pub data: MdstreamBuffer,
}

#[derive(Debug)]
pub struct MdstreamOutput {
    payloads: Vec<Option<BindingPayload>>,
    remaining: usize,
}

impl MdstreamOutput {
    pub(crate) fn from_binding(output: BindingOutput) -> *mut Self {
        let payloads = output
            .into_payloads()
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        let remaining = payloads.len();
        OUTPUTS.fetch_add(1, Ordering::Relaxed);
        Box::into_raw(Box::new(Self {
            payloads,
            remaining,
        }))
    }

    pub(crate) fn len(&self) -> usize {
        self.payloads.len()
    }

    pub(crate) fn remaining(&self) -> usize {
        self.remaining
    }

    pub(crate) fn take(
        &mut self,
        index: usize,
    ) -> Result<(BindingPayloadKind, Vec<u8>), BindingError> {
        let payload = self
            .payloads
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| {
                BindingError::new(
                    BindingStatus::InvalidArgument,
                    "ffi.output_index",
                    format!("output payload {index} is missing or already consumed"),
                )
            })?;
        self.remaining = self
            .remaining
            .checked_sub(1)
            .expect("a taken payload must have been counted as remaining");
        Ok((payload.kind(), payload.into_bytes()))
    }
}

impl Drop for MdstreamOutput {
    fn drop(&mut self) {
        OUTPUTS.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) unsafe fn with_output_mut<T>(
    output: *mut MdstreamOutput,
    operation: impl FnOnce(&mut MdstreamOutput) -> Result<T, BindingError>,
) -> Result<T, BindingError> {
    let Some(output) = (unsafe { output.as_mut() }) else {
        return Err(crate::errors::null_handle("output"));
    };
    operation(output)
}

pub(crate) fn buffer_from_vec(bytes: Vec<u8>) -> MdstreamBuffer {
    if bytes.is_empty() {
        return MdstreamBuffer::empty();
    }
    let bytes = bytes.into_boxed_slice();
    let len = bytes.len();
    let buffer = MdstreamBuffer {
        data: Box::into_raw(bytes) as *mut u8,
        len,
    };
    BUFFERS.fetch_add(1, Ordering::Relaxed);
    BUFFER_BYTES.fetch_add(buffer.len, Ordering::Relaxed);
    buffer
}

pub(crate) fn error_buffer(error: &BindingError) -> MdstreamBuffer {
    buffer_from_vec(error_payload_json_bytes(error))
}

pub(crate) unsafe fn free_buffer(buffer: MdstreamBuffer) {
    if buffer.data.is_null() || buffer.len == 0 {
        return;
    }
    BUFFERS.fetch_sub(1, Ordering::Relaxed);
    BUFFER_BYTES.fetch_sub(buffer.len, Ordering::Relaxed);
    let raw = ptr::slice_from_raw_parts_mut(buffer.data, buffer.len);
    unsafe {
        drop(Box::from_raw(raw));
    }
}

pub(crate) fn record_engine_created() {
    ENGINE_HANDLES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_engine_dropped() {
    ENGINE_HANDLES.fetch_sub(1, Ordering::Relaxed);
}

pub(crate) fn record_reducer_created() {
    REDUCER_HANDLES.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn record_reducer_dropped() {
    REDUCER_HANDLES.fetch_sub(1, Ordering::Relaxed);
}

pub(crate) fn allocation_metrics() -> MdstreamAllocationMetrics {
    MdstreamAllocationMetrics {
        engine_handles: saturating_u64(ENGINE_HANDLES.load(Ordering::Relaxed)),
        reducer_handles: saturating_u64(REDUCER_HANDLES.load(Ordering::Relaxed)),
        outputs: saturating_u64(OUTPUTS.load(Ordering::Relaxed)),
        buffers: saturating_u64(BUFFERS.load(Ordering::Relaxed)),
        buffer_bytes: saturating_u64(BUFFER_BYTES.load(Ordering::Relaxed)),
    }
}

fn saturating_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
