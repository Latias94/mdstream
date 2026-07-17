use std::{
    ptr,
    sync::{Arc, Barrier},
    thread,
};

use mdstream_bindings_core::BindingStatus;
use mdstream_ffi::{
    mdstream_allocation_metrics, mdstream_engine_execute, mdstream_engine_free,
    mdstream_engine_new, mdstream_output_free,
};

const SNAPSHOT: &[u8] = br#"{"schema":"mdstream.bindings/0.4","kind":"snapshot"}"#;

#[test]
fn concurrent_live_handle_calls_serialize_and_free_after_join_is_exact() {
    let engine = unsafe { mdstream_engine_new(ptr::null(), 0) };
    assert_eq!(engine.status, BindingStatus::Ok.code());
    let address = engine.engine as usize;
    let workers = 8;
    let barrier = Arc::new(Barrier::new(workers));
    let mut threads = Vec::new();

    for _ in 0..workers {
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..200 {
                let result = unsafe {
                    mdstream_engine_execute(
                        address as *mut mdstream_ffi::MdstreamEngine,
                        SNAPSHOT.as_ptr(),
                        SNAPSHOT.len(),
                    )
                };
                assert_eq!(result.status, BindingStatus::Ok.code());
                assert!(!result.output.is_null());
                assert!(result.error.data.is_null());
                unsafe { mdstream_output_free(result.output) };
            }
        }));
    }

    for thread in threads {
        thread.join().unwrap();
    }
    unsafe { mdstream_engine_free(engine.engine) };
    assert_eq!(mdstream_allocation_metrics(), Default::default());
}
