use mdstream::{Block, BlockId, BlockKind, BlockStatus, DocumentState, Update};

#[test]
fn document_state_applies_updates_in_order() {
    let mut state = DocumentState::new();

    let u1 = Update {
        committed: vec![Block {
            id: BlockId(1),
            status: BlockStatus::Committed,
            kind: BlockKind::Paragraph,
            raw: "A\n\n".to_string(),
            display: None,
        }],
        pending: Some(Block {
            id: BlockId(2),
            status: BlockStatus::Pending,
            kind: BlockKind::Paragraph,
            raw: "B".to_string(),
            display: Some("B_terminated".to_string()),
        }),
        reset: false,
        invalidated: Vec::new(),
    };
    let applied1 = state.apply(u1);
    assert!(!applied1.reset);
    assert!(applied1.invalidated.is_empty());
    assert_eq!(state.committed().len(), 1);
    assert_eq!(state.committed()[0].raw, "A\n\n");
    assert_eq!(state.pending().unwrap().raw, "B");

    let u2 = Update {
        committed: vec![Block {
            id: BlockId(3),
            status: BlockStatus::Committed,
            kind: BlockKind::Heading,
            raw: "# H\n".to_string(),
            display: None,
        }],
        pending: None,
        reset: false,
        invalidated: vec![BlockId(1)],
    };
    let applied2 = state.apply(u2);
    assert!(!applied2.reset);
    assert_eq!(applied2.invalidated, vec![BlockId(1)]);
    assert_eq!(state.committed().len(), 2);
    assert_eq!(state.committed()[0].id, BlockId(1));
    assert_eq!(state.committed()[1].id, BlockId(3));
    assert!(state.pending().is_none());
}

#[test]
fn document_state_reset_clears_previous_blocks() {
    let mut state = DocumentState::new();

    // Seed state.
    state.apply(Update {
        committed: vec![Block {
            id: BlockId(10),
            status: BlockStatus::Committed,
            kind: BlockKind::Paragraph,
            raw: "old\n".to_string(),
            display: None,
        }],
        pending: Some(Block {
            id: BlockId(11),
            status: BlockStatus::Pending,
            kind: BlockKind::Paragraph,
            raw: "pending".to_string(),
            display: None,
        }),
        reset: false,
        invalidated: Vec::new(),
    });

    let applied = state.apply(Update {
        committed: vec![Block {
            id: BlockId(1),
            status: BlockStatus::Committed,
            kind: BlockKind::Paragraph,
            raw: "X\n".to_string(),
            display: None,
        }],
        pending: None,
        reset: true,
        invalidated: Vec::new(),
    });
    assert!(applied.reset);
    assert_eq!(state.committed().len(), 1);
    assert_eq!(state.committed()[0].raw, "X\n");
    assert!(state.pending().is_none());
}
