use mdstream::{MdStream, Options};

#[test]
fn max_buffer_bytes_compacts_committed_prefix() {
    let max = 256usize;
    let opts = Options {
        max_buffer_bytes: Some(max),
        ..Default::default()
    };
    let mut s = MdStream::new(opts);

    // Many small blocks; the stream should be able to compact away committed prefixes and keep
    // its internal buffer bounded.
    let mut committed = 0usize;
    for i in 0..200 {
        let chunk = format!("# H{i}\n\n");
        let u = s.append(&chunk);
        committed += u.committed.len();
        assert!(
            s.buffer().len() <= max,
            "buffer should be compacted (len={})",
            s.buffer().len()
        );
    }

    let u = s.finalize();
    committed += u.committed.len();
    assert_eq!(committed, 200);
    assert!(s.buffer().len() <= max);
}
