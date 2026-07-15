use mdstream::{EngineLimits, StreamEngine};
use mdstream_protocol::{ApplyOutcome, Reducer};

const SIZES: [usize; 4] = [8 * 1024, 16 * 1024, 32 * 1024, 64 * 1024];

#[test]
fn source_growth_and_retained_text_obey_deterministic_capacity_bounds() {
    let source = "x".repeat(SIZES[SIZES.len() - 1]);
    let limits = EngineLimits::default();
    let mut engine = StreamEngine::builder()
        .engine_limits(limits)
        .build()
        .unwrap();

    for (index, byte) in source.as_bytes().iter().enumerate() {
        engine
            .append(std::str::from_utf8(std::slice::from_ref(byte)).unwrap())
            .unwrap();
        let size = index + 1;
        if !SIZES.contains(&size) {
            continue;
        }
        let metrics = engine.metrics();
        assert_eq!(metrics.storage.canonical_source_bytes, size);
        assert!(
            metrics.storage.canonical_source_capacity <= size.saturating_mul(2) + 64,
            "source capacity at {size}: {metrics:?}"
        );
        assert!(
            metrics.storage.source_reallocation_copied_bytes <= size.saturating_mul(2) + 64,
            "source reallocation copies at {size}: {metrics:?}"
        );
        assert_eq!(metrics.storage.duplicated_source_body_bytes, 0);
        assert_eq!(
            metrics.storage.retained_text_bytes,
            size.saturating_add(metrics.storage.canonical_ir_text_bytes)
        );
        assert!(
            metrics.storage.retained_text_capacity <= size.saturating_mul(3) + 64,
            "retained text capacity at {size}: {metrics:?}"
        );
        assert!(metrics.work.peak_transaction_bytes <= limits.max_transaction_bytes);
    }
}

#[test]
fn snapshot_build_and_load_are_measured_separately_from_append_work() {
    let source = "snapshot source".repeat(256);
    let mut engine = StreamEngine::new();
    engine.append(&source).unwrap();
    let append_metrics = engine.metrics();
    let snapshot = engine.snapshot().unwrap();
    assert_eq!(snapshot.source().len(), source.len());
    assert_eq!(engine.metrics(), append_metrics);

    let mut recovered = Reducer::new();
    assert!(matches!(
        recovered.recover_snapshot(snapshot).unwrap(),
        ApplyOutcome::Recovered { .. }
    ));
    let recovered_document = recovered.document().unwrap();
    assert_eq!(
        recovered_document.retained_ir_text_bytes(),
        append_metrics.storage.canonical_ir_text_bytes
    );
    assert!(
        recovered_document.retained_ir_text_capacity()
            >= recovered_document.retained_ir_text_bytes()
    );
    assert_eq!(
        recovered.metrics().snapshot_source_bytes_loaded,
        source.len() as u64
    );
    assert_eq!(recovered.metrics().source_bytes_appended, 0);
    assert_eq!(recovered.metrics().source_reallocation_copied_bytes, 0);
}

#[test]
fn canonical_ir_owned_text_is_included_in_retained_storage() {
    let source = concat!(
        "[label](https://example.test/path \"a retained title\") &amp; ",
        "[reference][shared]\n\n",
        "[shared]: https://shared.test \"shared title\"\n",
    );
    let mut engine = StreamEngine::new();
    engine.append(source).unwrap();
    engine.finish().unwrap();

    let storage = engine.metrics().storage;
    assert!(storage.canonical_ir_text_bytes > 0);
    assert!(storage.canonical_ir_text_capacity >= storage.canonical_ir_text_bytes);
    assert!(storage.retained_text_bytes > storage.canonical_source_bytes);
    assert!(storage.retained_text_capacity > storage.canonical_source_capacity);
    assert_eq!(storage.duplicated_source_body_bytes, 0);

    engine.reset().unwrap();
    let reset = engine.metrics().storage;
    assert!(reset.canonical_ir_text_bytes > 0);
    assert!(reset.canonical_ir_text_capacity >= reset.canonical_ir_text_bytes);
    assert_eq!(reset.retained_text_bytes, reset.canonical_ir_text_bytes);
    assert_eq!(
        reset.retained_text_capacity,
        reset.canonical_ir_text_capacity
    );
    assert_eq!(reset.duplicated_source_body_bytes, 0);
}

#[test]
fn an_empty_canonical_document_counts_its_root_version() {
    let mut engine = StreamEngine::new();
    assert_eq!(engine.metrics().storage.canonical_ir_text_bytes, 0);

    engine.reset().unwrap();
    let storage = engine.metrics().storage;
    assert_eq!(storage.canonical_source_bytes, 0);
    assert!(storage.canonical_ir_text_bytes > 0);
    assert_eq!(storage.retained_text_bytes, storage.canonical_ir_text_bytes);
    assert_eq!(storage.duplicated_source_body_bytes, 0);
}
