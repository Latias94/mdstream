use mdstream::{CompilerLimits, StreamEngine};
use mdstream_protocol::{ProjectionOp, ProtocolLimits};

#[test]
fn late_definition_work_is_exactly_proportional_to_its_reverse_edges() {
    use std::fmt::Write as _;

    let mut source = String::new();
    for index in 0..100 {
        writeln!(source, "[dependent {index}][late]\n").unwrap();
    }
    for index in 0..1_000 {
        writeln!(source, "[semantic {index}][other-{index}]\n").unwrap();
    }
    for index in 0..8_900 {
        writeln!(source, "unrelated {index}\n").unwrap();
    }
    for index in 0..1_000 {
        writeln!(source, "[other-{index}]: /other-{index}").unwrap();
    }
    source.push('\n');
    let mut engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_operations: 100_000,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    engine.append(&source).unwrap();
    let before = engine.metrics().compiler;

    let output = engine.append("[late]: /target\n").unwrap();
    let after = engine.metrics().compiler;
    assert_eq!(
        after.semantic_dependent_visits - before.semantic_dependent_visits,
        100
    );
    assert_eq!(
        after.semantic_corrections_emitted - before.semantic_corrections_emitted,
        100
    );
    assert_eq!(
        after.semantic_state_key_visits - before.semantic_state_key_visits,
        2,
        "stable definitions and reverse edges must not be scanned"
    );
    assert_eq!(
        after.reconcile_node_visits - before.reconcile_node_visits,
        0,
        "stable unrelated nodes must not be revisited by frontier reconciliation"
    );
    assert_eq!(after.retained_semantic_definitions, 1_001);
    assert_eq!(after.retained_semantic_dependencies, 1_100);
    let replacements = output
        .changes()
        .iter()
        .flat_map(|change| change.operations())
        .filter(|operation| matches!(operation, ProjectionOp::ReplaceNode { .. }))
        .count();
    assert_eq!(replacements, 100);

    let before_duplicate = engine.metrics().compiler;
    let duplicate = engine.append("[late]: /ignored\n").unwrap();
    let after_duplicate = engine.metrics().compiler;
    assert_eq!(
        after_duplicate.semantic_dependent_visits,
        before_duplicate.semantic_dependent_visits
    );
    assert_eq!(
        after_duplicate.semantic_corrections_emitted,
        before_duplicate.semantic_corrections_emitted
    );
    assert!(duplicate.changes().iter().all(|change| {
        change
            .operations()
            .iter()
            .all(|operation| !matches!(operation, ProjectionOp::ReplaceNode { .. }))
    }));
}

#[test]
fn definition_edge_limit_rejects_the_entire_transition_and_allows_retry() {
    let mut engine = StreamEngine::builder()
        .compiler_limits(CompilerLimits {
            max_definition_edges: 1,
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    engine.append("[a][label]\n\n").unwrap();
    let before = engine.snapshot().unwrap();
    let before_metrics = engine.metrics();

    assert!(matches!(
        engine.append("[b][label]\n\n"),
        Err(mdstream::EngineError::Compiler(
            mdstream::CompilerError::LimitExceeded {
                field: "definition.dependencies",
                limit: 1,
                actual: 2,
            }
        ))
    ));
    assert_eq!(engine.snapshot().unwrap(), before);
    assert_eq!(engine.metrics(), before_metrics);
    engine.append("plain retry").unwrap();
}

#[test]
fn definition_value_limit_accepts_the_boundary_and_atomically_rejects_boundary_plus_one() {
    let mut engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_metadata_value_bytes: 5,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    engine
        .append("[a]: /1234\n\n")
        .expect("a five-byte destination must fit the exact value limit");
    let before = engine.snapshot().unwrap();
    let before_coordinate = engine.coordinate().cloned().unwrap();
    let before_metrics = engine.metrics();

    assert!(matches!(
        engine.append("[b]: /12345\n"),
        Err(mdstream::EngineError::Compiler(
            mdstream::CompilerError::LimitExceeded {
                field: "definition.destination",
                limit: 5,
                actual: 6,
            }
        ))
    ));
    assert_eq!(engine.snapshot().unwrap(), before);
    assert_eq!(engine.coordinate(), Some(&before_coordinate));
    assert_eq!(engine.metrics(), before_metrics);

    let retry = engine
        .append("[b]: /1234\n")
        .expect("a corrected definition must succeed after atomic rejection");
    assert_eq!(retry.changes().len(), 1);
    assert_eq!(
        retry.changes()[0].sequence(),
        before_coordinate.sequence.checked_add(1).unwrap()
    );
    assert_eq!(
        engine.metrics().compiler.retained_semantic_definitions,
        2,
        "the rejected definition must not pollute retained semantic state"
    );
}

#[test]
fn normalized_definition_label_limit_accepts_the_boundary_and_rejects_boundary_plus_one() {
    let mut engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_metadata_value_bytes: 2,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    engine
        .append("[aa]: /a\n\n")
        .expect("a two-byte normalized label must fit the exact value limit");
    let before = engine.snapshot().unwrap();
    let before_coordinate = engine.coordinate().cloned().unwrap();
    let before_metrics = engine.metrics();

    assert!(matches!(
        engine.append("[İ]: /a\n"),
        Err(mdstream::EngineError::Compiler(
            mdstream::CompilerError::LimitExceeded {
                field: "definition.normalized_label",
                limit: 2,
                actual: 3,
            }
        ))
    ));
    assert_eq!(engine.snapshot().unwrap(), before);
    assert_eq!(engine.coordinate(), Some(&before_coordinate));
    assert_eq!(engine.metrics(), before_metrics);

    let retry = engine
        .append("[i]: /a\n")
        .expect("a corrected normalized label must succeed after rejection");
    assert_eq!(retry.changes().len(), 1);
    assert_eq!(
        retry.changes()[0].sequence(),
        before_coordinate.sequence.checked_add(1).unwrap()
    );
    assert_eq!(engine.metrics().compiler.retained_semantic_definitions, 2);
}

#[test]
fn optional_definition_title_limit_accepts_the_boundary_and_rejects_boundary_plus_one() {
    let mut engine = StreamEngine::builder()
        .protocol_limits(ProtocolLimits {
            max_metadata_value_bytes: 5,
            ..ProtocolLimits::default()
        })
        .build()
        .unwrap();
    engine
        .append("[a]: /a \"12345\"\n\n")
        .expect("a five-byte title must fit the exact value limit");
    let before = engine.snapshot().unwrap();
    let before_coordinate = engine.coordinate().cloned().unwrap();
    let before_metrics = engine.metrics();

    assert!(matches!(
        engine.append("[b]: /b \"123456\"\n"),
        Err(mdstream::EngineError::Compiler(
            mdstream::CompilerError::LimitExceeded {
                field: "definition.title",
                limit: 5,
                actual: 6,
            }
        ))
    ));
    assert_eq!(engine.snapshot().unwrap(), before);
    assert_eq!(engine.coordinate(), Some(&before_coordinate));
    assert_eq!(engine.metrics(), before_metrics);

    let retry = engine
        .append("[b]: /b \"12345\"\n")
        .expect("a corrected title must succeed after rejection");
    assert_eq!(retry.changes().len(), 1);
    assert_eq!(
        retry.changes()[0].sequence(),
        before_coordinate.sequence.checked_add(1).unwrap()
    );
    assert_eq!(engine.metrics().compiler.retained_semantic_definitions, 2);
}

#[test]
fn cumulative_definition_metadata_limit_is_atomic_at_boundary_plus_one() {
    let mut engine = StreamEngine::builder()
        .compiler_limits(CompilerLimits {
            max_definition_metadata_bytes: 12,
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    engine.append("[a]: /12\n\n").unwrap();
    assert_eq!(
        engine.metrics().compiler.retained_semantic_metadata_bytes,
        6
    );
    let before = engine.snapshot().unwrap();
    let before_coordinate = engine.coordinate().cloned().unwrap();
    let before_metrics = engine.metrics();

    assert!(matches!(
        engine.append("[b]: /123\n"),
        Err(mdstream::EngineError::Compiler(
            mdstream::CompilerError::LimitExceeded {
                field: "definition.metadata",
                limit: 12,
                actual: 13,
            }
        ))
    ));
    assert_eq!(engine.snapshot().unwrap(), before);
    assert_eq!(engine.coordinate(), Some(&before_coordinate));
    assert_eq!(engine.metrics(), before_metrics);

    let retry = engine
        .append("[b]: /12\n")
        .expect("an exact-budget retry must succeed after atomic rejection");
    assert_eq!(retry.changes().len(), 1);
    assert_eq!(
        retry.changes()[0].sequence(),
        before_coordinate.sequence.checked_add(1).unwrap()
    );
    assert_eq!(
        engine.metrics().compiler.retained_semantic_metadata_bytes,
        12
    );
    assert_eq!(engine.metrics().compiler.retained_semantic_definitions, 2);
}

#[test]
fn unused_definitions_are_retained_under_independent_atomic_limits() {
    let mut engine = StreamEngine::builder()
        .compiler_limits(CompilerLimits {
            max_definitions: 1,
            max_definition_metadata_bytes: 3 * "a".len() + "/a".len(),
            ..CompilerLimits::default()
        })
        .build()
        .unwrap();
    engine.append("[a]: /a\n\n").unwrap();
    let before = engine.snapshot().unwrap();

    assert!(matches!(
        engine.append("[b]: /b\n"),
        Err(mdstream::EngineError::Compiler(
            mdstream::CompilerError::LimitExceeded {
                field: "definitions",
                limit: 1,
                actual: 2,
            }
        ))
    ));
    assert_eq!(engine.snapshot().unwrap(), before);
    engine
        .append("[a]: /ignored-duplicate\n")
        .expect("a duplicate does not retain another definition");
}
