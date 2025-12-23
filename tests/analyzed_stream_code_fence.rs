use mdstream::{
    AnalyzedStream, BlockAnalyzer, CodeFenceAnalyzer, CodeFenceClass, CodeFenceMeta, Options,
};

#[test]
fn analyzed_stream_emits_pending_and_committed_meta_for_code_fences() {
    let mut s = AnalyzedStream::new(Options::default(), CodeFenceAnalyzer);

    let u1 = s.append("```mermaid\n");
    assert!(u1.update.committed.is_empty());
    let pending = u1.update.pending.expect("pending");
    assert_eq!(pending.kind, mdstream::BlockKind::CodeFence);
    assert_eq!(
        u1.pending_meta,
        Some(mdstream::BlockMeta {
            id: pending.id,
            meta: CodeFenceMeta {
                info: "mermaid".to_string(),
                language: Some("mermaid".to_string()),
                class: CodeFenceClass::Mermaid,
            }
        })
    );

    let u2 = s.append("graph TD;\nA-->B;\n");
    assert!(u2.update.committed.is_empty());
    assert!(u2.update.pending.is_some());

    let u3 = s.append("```\n");
    assert_eq!(u3.update.committed.len(), 1);
    assert_eq!(u3.update.committed[0].kind, mdstream::BlockKind::CodeFence);
    assert_eq!(u3.committed_meta.len(), 1);
    assert_eq!(u3.committed_meta[0].id, u3.update.committed[0].id);
    assert_eq!(u3.committed_meta[0].meta.class, CodeFenceClass::Mermaid);
    assert!(s.meta_for(u3.update.committed[0].id).is_some());
}

#[test]
fn tuple_analyzer_can_be_chained() {
    #[derive(Default)]
    struct OnlyParagraph;

    impl BlockAnalyzer for OnlyParagraph {
        type Meta = &'static str;

        fn analyze_block(&mut self, block: &mdstream::Block) -> Option<Self::Meta> {
            if block.kind == mdstream::BlockKind::Paragraph {
                Some("p")
            } else {
                None
            }
        }
    }

    let analyzer = (CodeFenceAnalyzer, OnlyParagraph);
    let mut s = AnalyzedStream::new(Options::default(), analyzer);

    let u1 = s.append("hi\n\n```json\n");
    assert!(
        u1.committed_meta
            .iter()
            .any(|m| m.meta.1 == Some("p") && m.id == u1.update.committed[0].id)
    );
    assert!(u1.pending_meta.is_some());
}
