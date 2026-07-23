use mdstream::{
    AppendBytesError, AppendLimitKind, CompilerError, EngineError, EngineLimits, SplitSafety,
    StreamEngine,
};
use mdstream_protocol::{
    NodeId, ProtocolError, ProtocolLimits, ResourceId, SourceCursor, encode_change_json,
};

fn configured(limits: EngineLimits) -> StreamEngine {
    StreamEngine::builder()
        .engine_limits(limits)
        .build()
        .unwrap()
}

fn configured_protocol(limits: ProtocolLimits) -> StreamEngine {
    StreamEngine::builder()
        .protocol_limits(limits)
        .build()
        .unwrap()
}

#[test]
fn change_budget_accepts_exact_and_rejects_plus_one_atomically() {
    const CHANGE_BYTES: usize = 1813;
    let prefix = "seed\n\n";
    let suffix = "a paragraph with enough content to exceed the seed transition";

    let mut exact = configured(EngineLimits {
        max_change_bytes: CHANGE_BYTES,
        ..EngineLimits::default()
    });
    exact.append(prefix).unwrap();
    exact.append(suffix).unwrap();
    assert_eq!(exact.metrics().work.last_change_bytes, CHANGE_BYTES);

    let mut rejected = configured(EngineLimits {
        max_change_bytes: CHANGE_BYTES - 1,
        ..EngineLimits::default()
    });
    rejected.append(prefix).unwrap();
    let before_snapshot = rejected.snapshot().unwrap();
    let before_coordinate = rejected.coordinate().cloned().unwrap();
    let before_metrics = rejected.metrics();
    assert!(matches!(
        rejected.append(suffix),
        Err(EngineError::AppendLimitExceeded {
            kind: AppendLimitKind::ChangeBytes,
            limit,
            actual,
        }) if limit == CHANGE_BYTES - 1 && actual == CHANGE_BYTES
    ));
    assert_eq!(rejected.snapshot().unwrap(), before_snapshot);
    assert_eq!(rejected.coordinate(), Some(&before_coordinate));
    assert_eq!(rejected.metrics(), before_metrics);
}

#[test]
fn transaction_budget_counts_a_consumed_frontier_atomically() {
    const PENDING_BYTES: usize = 4096;
    const CLOSING_CHANGE_BYTES: usize = 1182;
    const STAGING_FRONTIER_BYTES: usize = PENDING_BYTES + 2;
    const TRANSACTION_BYTES: usize = CLOSING_CHANGE_BYTES + STAGING_FRONTIER_BYTES + 2;
    let pending = "x".repeat(PENDING_BYTES);

    let mut exact = configured(EngineLimits {
        max_transaction_bytes: TRANSACTION_BYTES,
        ..EngineLimits::default()
    });
    append_pending_paragraph(&mut exact, &pending);
    exact.append("\n\n").unwrap();
    assert_eq!(exact.metrics().work.last_change_bytes, CLOSING_CHANGE_BYTES);
    assert_eq!(
        exact.metrics().work.last_transaction_bytes,
        TRANSACTION_BYTES
    );

    let mut rejected = configured(EngineLimits {
        max_transaction_bytes: TRANSACTION_BYTES - 1,
        ..EngineLimits::default()
    });
    append_pending_paragraph(&mut rejected, &pending);
    let before_snapshot = rejected.snapshot().unwrap();
    let before_coordinate = rejected.coordinate().cloned().unwrap();
    let before_metrics = rejected.metrics();
    assert!(matches!(
        rejected.append("\n\n"),
        Err(EngineError::AppendLimitExceeded {
            kind: AppendLimitKind::TransactionBytes,
            limit,
            actual,
        }) if limit == TRANSACTION_BYTES - 1 && actual == TRANSACTION_BYTES
    ));
    assert_eq!(rejected.snapshot().unwrap(), before_snapshot);
    assert_eq!(rejected.coordinate(), Some(&before_coordinate));
    assert_eq!(rejected.metrics(), before_metrics);
}

fn append_pending_paragraph(engine: &mut StreamEngine, pending: &str) {
    for chunk in pending.as_bytes().chunks(2) {
        engine.append(std::str::from_utf8(chunk).unwrap()).unwrap();
    }
}

#[test]
fn encoded_change_budget_is_exact_and_transport_local() {
    let mut engine = StreamEngine::new();
    let before = engine.metrics();
    let output = engine.append("wire").unwrap();
    let change = &output.changes()[0];
    let encoded = encode_change_json(change, usize::MAX, ProtocolLimits::default()).unwrap();

    assert_eq!(
        encode_change_json(change, encoded.len(), ProtocolLimits::default()).unwrap(),
        encoded
    );
    assert!(encode_change_json(change, encoded.len() - 1, ProtocolLimits::default()).is_err());
    assert_ne!(engine.metrics(), before);
    assert_eq!(engine.snapshot().unwrap().source(), "wire");
}

#[test]
fn trailing_cr_debt_is_source_preflighted_atomically() {
    let mut exact = configured_protocol(ProtocolLimits {
        max_source_bytes: 2,
        ..ProtocolLimits::default()
    });
    exact.append("a\r").unwrap();
    assert_eq!(exact.snapshot().unwrap().source(), "a");
    assert_eq!(exact.metrics().retained_input_bytes, 2);
    assert_eq!(exact.metrics().storage.normalized_input_debt_bytes, 1);

    let mut rejected = configured_protocol(ProtocolLimits {
        max_source_bytes: 1,
        ..ProtocolLimits::default()
    });
    let before = rejected.metrics();
    assert!(matches!(
        rejected.append("a\r"),
        Err(EngineError::Protocol(
            mdstream_protocol::ProtocolError::SourceTooLarge {
                limit: 1,
                actual: 2,
            }
        ))
    ));
    assert!(rejected.snapshot().is_none());
    assert_eq!(rejected.metrics(), before);
    rejected.append("a").unwrap();
    assert_eq!(rejected.snapshot().unwrap().source(), "a");
}

#[test]
fn pending_cr_survives_empty_append_and_failed_resolution_until_finish() {
    let mut engine = configured_protocol(ProtocolLimits {
        max_source_bytes: 1,
        ..ProtocolLimits::default()
    });
    assert!(engine.append("\r").unwrap().changes().is_empty());
    let pending = engine.metrics();
    assert_eq!(pending.retained_input_bytes, 1);
    assert_eq!(pending.storage.normalized_input_debt_bytes, 1);
    assert!(engine.append("").unwrap().changes().is_empty());
    assert_eq!(engine.metrics(), pending);

    assert!(matches!(
        engine.append("x"),
        Err(EngineError::Protocol(
            mdstream_protocol::ProtocolError::SourceTooLarge {
                limit: 1,
                actual: 2,
            }
        ))
    ));
    assert_eq!(engine.metrics(), pending);

    engine.finish().unwrap();
    assert_eq!(engine.snapshot().unwrap().source(), "\n");
    assert_eq!(engine.metrics().retained_input_bytes, 0);
    assert_eq!(engine.metrics().storage.normalized_input_debt_bytes, 0);
}

#[test]
fn reset_clears_pending_cr_debt() {
    let mut engine = configured_protocol(ProtocolLimits {
        max_source_bytes: 1,
        ..ProtocolLimits::default()
    });
    engine.append("\r").unwrap();
    assert_eq!(engine.metrics().retained_input_bytes, 1);

    engine.reset().unwrap();
    assert_eq!(engine.metrics().retained_input_bytes, 0);
    assert_eq!(engine.metrics().storage.normalized_input_debt_bytes, 0);
    engine.finish().unwrap();
    assert_eq!(engine.snapshot().unwrap().source(), "");
}

#[test]
fn only_typed_append_local_limits_are_split_safe() {
    for kind in [
        AppendLimitKind::ChangeOperations,
        AppendLimitKind::ChangeStructuralItems,
        AppendLimitKind::ChangeMetadataBytes,
        AppendLimitKind::ChangeBytes,
        AppendLimitKind::TransactionBytes,
    ] {
        let error = match kind {
            AppendLimitKind::ChangeBytes | AppendLimitKind::TransactionBytes => {
                EngineError::AppendLimitExceeded {
                    kind,
                    limit: 1,
                    actual: 2,
                }
            }
            _ => EngineError::Compiler(CompilerError::AppendLimitExceeded {
                kind,
                limit: 1,
                actual: 2,
            }),
        };
        assert_eq!(error.split_safety(), SplitSafety::RetryAtOriginalBoundaries);
    }

    let compiler_errors = [
        CompilerError::CursorOverflow,
        CompilerError::InvalidSourceBoundary(SourceCursor::new(0)),
        CompilerError::InvalidConfiguration("invalid".to_string()),
        CompilerError::LimitExceeded {
            field: "markdown.events",
            limit: 1,
            actual: 2,
        },
        CompilerError::Markdown(mdstream::MarkdownDiagnostic::Unsupported("test")),
        CompilerError::NodeIdentityCollision(NodeId::from(1_u64)),
        CompilerError::ResourceIdentityCollision(ResourceId::from(1_u64)),
        CompilerError::InvalidIdentity("invalid".to_string()),
        CompilerError::InvalidReconciliation("invalid".to_string()),
        CompilerError::MetricsOverflow("metrics"),
    ];
    for error in compiler_errors {
        assert_eq!(error.split_safety(), SplitSafety::NotSafe);
        assert_eq!(
            EngineError::Compiler(error).split_safety(),
            SplitSafety::NotSafe
        );
    }

    let mut source_limited = configured_protocol(ProtocolLimits {
        max_source_bytes: 1,
        ..ProtocolLimits::default()
    });
    let source_error = source_limited.append("too large").unwrap_err();
    assert_eq!(source_error.split_safety(), SplitSafety::NotSafe);

    let protocol_error = EngineError::Protocol(ProtocolError::SourceTooLarge {
        limit: 1,
        actual: 2,
    });
    assert_eq!(protocol_error.split_safety(), SplitSafety::NotSafe);
    assert_eq!(
        EngineError::Finished.split_safety(),
        SplitSafety::NotSafe,
        "lifecycle failures must never invite input replay"
    );
}

#[test]
fn raw_byte_admission_rejects_before_utf8_and_keeps_crlf_exact() {
    let mut engine = configured_protocol(ProtocolLimits {
        max_source_bytes: 4,
        ..ProtocolLimits::default()
    });
    assert_eq!(engine.raw_append_byte_ceiling(), 8);
    engine.append_bytes(b"\r\n\r\n\r\n\r\n").unwrap();
    assert_eq!(engine.snapshot().unwrap().source(), "\n\n\n\n");

    let mut rejected = configured_protocol(ProtocolLimits {
        max_source_bytes: 2,
        ..ProtocolLimits::default()
    });
    assert_eq!(rejected.raw_append_byte_ceiling(), 4);
    assert!(matches!(
        rejected.append_bytes(&[0xff; 5]),
        Err(AppendBytesError::RawInputTooLarge {
            limit: 4,
            actual: 5,
        })
    ));
    assert!(rejected.snapshot().is_none());
    assert!(matches!(
        rejected.append_bytes(&[0xff]),
        Err(AppendBytesError::InvalidUtf8(_))
    ));
    rejected.append_bytes(b"ok").unwrap();
    assert_eq!(rejected.snapshot().unwrap().source(), "ok");
}

#[test]
fn raw_ceiling_accounts_for_a_cross_chunk_trailing_cr() {
    let mut engine = configured_protocol(ProtocolLimits {
        max_source_bytes: 4,
        ..ProtocolLimits::default()
    });
    engine.append("a\r").unwrap();
    assert_eq!(engine.raw_append_byte_ceiling(), 5);
    engine.append_bytes(b"\n\r\n\r\n").unwrap();
    assert_eq!(engine.snapshot().unwrap().source(), "a\n\n\n");
}
