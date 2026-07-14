use mdstream::{MdStream, Options, StreamEngine};
use mdstream_protocol::{ApplyOutcome, Reducer, SourceCursor};

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

#[test]
fn stream_engine_keeps_absolute_source_and_ranges_after_scanner_compaction() {
    let max = 64usize;
    let options = Options {
        max_buffer_bytes: Some(max),
        ..Options::default()
    };
    let mut engine = StreamEngine::new(options);
    let mut reducer = Reducer::new();
    let mut source = String::new();

    for index in 0..100 {
        let chunk = format!("# H{index}\n\n");
        source.push_str(&chunk);
        for change in engine.append(&chunk).unwrap().into_changes() {
            assert!(matches!(
                reducer.apply(change).unwrap(),
                ApplyOutcome::Applied { .. }
            ));
        }
        assert!(engine.metrics().retained_input_bytes <= max);
    }
    for change in engine.finish().unwrap().into_changes() {
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::Applied { .. }
        ));
    }

    assert!(engine.metrics().retained_source_base > 0);
    let document = reducer.document().unwrap();
    assert_eq!(document.source(), source);
    assert_eq!(
        document.coordinate().source_cursor,
        SourceCursor::new(source.len() as u64)
    );
    let frame = document.nodes().next().unwrap();
    assert_eq!(frame.source.start, SourceCursor::new(0));
    assert_eq!(frame.source.end, SourceCursor::new(source.len() as u64));
}
