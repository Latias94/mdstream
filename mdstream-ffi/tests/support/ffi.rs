use mdstream_bindings_core::BindingStatus;
use mdstream_ffi::{
    MdstreamBuffer, MdstreamCallResult, mdstream_buffer_free, mdstream_output_free,
};

pub fn free_success(result: MdstreamCallResult) {
    assert_eq!(result.status, BindingStatus::Ok.code());
    assert!(!result.output.is_null());
    assert!(result.error.data.is_null());
    unsafe { mdstream_output_free(result.output) };
}

pub fn take_buffer(buffer: MdstreamBuffer) -> Vec<u8> {
    if buffer.data.is_null() || buffer.len == 0 {
        return Vec::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(buffer.data, buffer.len).to_vec() };
    unsafe { mdstream_buffer_free(buffer) };
    bytes
}
