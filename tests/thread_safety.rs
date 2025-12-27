//! Static assertions to verify MdStream is Send + Sync.
//!
//! These tests ensure that MdStream can be used with reactive frameworks
//! like Leptos that require Send + Sync bounds for shared state.

use mdstream::MdStream;

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

#[test]
fn mdstream_is_send() {
    assert_send::<MdStream>();
}

#[test]
fn mdstream_is_sync() {
    assert_sync::<MdStream>();
}

#[test]
fn mdstream_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MdStream>();
}
