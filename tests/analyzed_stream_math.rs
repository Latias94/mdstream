use mdstream::{AnalyzedStream, MathAnalyzer, MathMeta, Options};

#[test]
fn math_analyzer_marks_pending_unbalanced_then_committed_balanced() {
    let mut s = AnalyzedStream::new(Options::default(), MathAnalyzer);

    let u1 = s.append("$$\nE = mc^2\n");
    assert!(u1.update.committed.is_empty());
    let pending = u1.update.pending.expect("pending");
    assert_eq!(pending.kind, mdstream::BlockKind::MathBlock);
    assert_eq!(
        u1.pending_meta,
        Some(mdstream::BlockMeta {
            id: pending.id,
            meta: MathMeta { balanced: false }
        })
    );

    let u2 = s.append("$$\n\nAfter\n");
    assert!(u2.committed_meta.iter().any(|m| m.meta.balanced));
    assert_eq!(u2.update.pending.as_ref().unwrap().raw, "After\n");
}
