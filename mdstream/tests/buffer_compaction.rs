use mdstream::{MdStream, Options, StreamEngine};
use mdstream_conformance::NormalizedSnapshot;
use mdstream_protocol::{
    ApplyOutcome, ContentKind, NodeId, NodeVersion, Reducer, ResourceRef, Snapshot, SourceCursor,
};

fn shared_reference(snapshot: &Snapshot) -> (NodeId, NodeVersion, Option<ResourceRef>) {
    snapshot
        .nodes()
        .iter()
        .find_map(|node| match &node.content {
            ContentKind::Link {
                reference_label,
                target,
                ..
            } if reference_label.as_deref() == Some("shared") => {
                Some((node.id, node.version.clone(), target.clone()))
            }
            _ => None,
        })
        .expect("the shortcut reference must remain in the canonical projection")
}

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
    let mut engine = StreamEngine::new();
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
    assert_eq!(document.roots().len(), 100);
    let mut expected_start = 0_u64;
    for (index, root_id) in document.roots().iter().copied().enumerate() {
        let heading = document.node(root_id).unwrap();
        let raw = format!("# H{index}");
        let expected_end = expected_start + raw.len() as u64;
        assert_eq!(heading.source.start, SourceCursor::new(expected_start));
        assert_eq!(heading.source.end, SourceCursor::new(expected_end));
        assert!(source.is_char_boundary(heading.source.start.get() as usize));
        assert!(source.is_char_boundary(heading.source.end.get() as usize));
        expected_start = expected_end + 2;
    }
    assert_eq!(expected_start, source.len() as u64);
}

#[test]
fn compaction_preserves_identity_versions_and_late_semantic_dependencies() {
    let mut split = StreamEngine::new();
    split.append("[shared]\n\n").unwrap();

    let mut middle = String::new();
    for index in 0..200 {
        let block = format!("# H{index}\n\n");
        middle.push_str(&block);
        split.append(&block).unwrap();
    }

    let before = split.snapshot().unwrap();
    let (before_id, before_version, before_target) = shared_reference(&before);
    assert_eq!(before_target, None);
    let before_reference = before
        .nodes()
        .iter()
        .find(|node| node.id == before_id)
        .unwrap();
    assert_eq!(before_reference.source.start, SourceCursor::new(0));
    assert_eq!(
        before_reference.source.end,
        SourceCursor::new("[shared]".len() as u64)
    );
    assert!(split.metrics().retained_source_base > 0);
    assert_eq!(split.metrics().compiler.retained_semantic_dependencies, 1);

    let definition = "[shared]: /target\n";
    split.append(definition).unwrap();
    split.finish().unwrap();
    let after = split.snapshot().unwrap();
    let (after_id, after_version, after_target) = shared_reference(&after);
    assert_eq!(after_id, before_id);
    assert_ne!(after_version, before_version);
    assert!(after_target.is_some());

    let source = format!("[shared]\n\n{middle}{definition}");
    let mut whole = StreamEngine::new();
    whole.append(&source).unwrap();
    whole.finish().unwrap();
    assert_eq!(
        NormalizedSnapshot::from(after),
        NormalizedSnapshot::from(whole.snapshot().unwrap())
    );
}
