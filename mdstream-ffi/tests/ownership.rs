use std::ptr;

use mdstream_bindings_core::BindingStatus;
use mdstream_ffi::{
    MdstreamBuffer, mdstream_allocation_metrics, mdstream_buffer_free, mdstream_engine_append,
    mdstream_engine_free, mdstream_engine_new, mdstream_output_free, mdstream_output_len,
    mdstream_output_take, mdstream_reducer_free, mdstream_reducer_new,
};

#[test]
fn ten_thousand_success_error_and_partial_output_cycles_release_every_allocation() {
    assert_eq!(mdstream_allocation_metrics(), Default::default());

    for _ in 0..10_000 {
        let engine = unsafe { mdstream_engine_new(ptr::null(), 0) };
        let reducer = unsafe { mdstream_reducer_new(ptr::null(), 0) };
        assert_eq!(engine.status, BindingStatus::Ok.code());
        assert_eq!(reducer.status, BindingStatus::Ok.code());

        let output = unsafe { mdstream_engine_append(engine.engine, b"x".as_ptr(), 1) };
        assert_eq!(output.status, BindingStatus::Ok.code());
        let len = unsafe { mdstream_output_len(output.output) };
        if len > 0 {
            let first = unsafe { mdstream_output_take(output.output, 0) };
            assert_eq!(first.status, BindingStatus::Ok.code());
            unsafe { mdstream_buffer_free(first.data) };
        }
        unsafe { mdstream_output_free(output.output) };

        let error = unsafe { mdstream_engine_append(engine.engine, [0xff].as_ptr(), 1) };
        assert_eq!(error.status, BindingStatus::Utf8.code());
        assert!(error.output.is_null());
        unsafe { mdstream_buffer_free(error.error) };

        unsafe {
            mdstream_reducer_free(reducer.reducer);
            mdstream_engine_free(engine.engine);
        }
    }

    unsafe { mdstream_buffer_free(MdstreamBuffer::empty()) };
    assert_eq!(mdstream_allocation_metrics(), Default::default());
}
