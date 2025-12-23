use mdstream::{Block, BlockId, BlockKind, BlockStatus};

#[test]
fn parses_code_fence_language_backticks() {
    let b = Block {
        id: BlockId(1),
        status: BlockStatus::Committed,
        kind: BlockKind::CodeFence,
        raw: "```mermaid\ngraph TD;\nA-->B;\n```\n".to_string(),
        display: None,
    };
    assert_eq!(b.code_fence_language(), Some("mermaid"));
    let h = b.code_fence_header().expect("header");
    assert_eq!(h.fence_char, '`');
    assert!(h.fence_len >= 3);
    assert_eq!(h.info, "mermaid");
}

#[test]
fn parses_code_fence_language_tildes_and_whitespace() {
    let b = Block {
        id: BlockId(1),
        status: BlockStatus::Committed,
        kind: BlockKind::CodeFence,
        raw: "~~~   jsonc   \n{a:1,}\n~~~\n".to_string(),
        display: None,
    };
    assert_eq!(b.code_fence_language(), Some("jsonc"));
}

#[test]
fn returns_none_for_non_code_fence_blocks() {
    let b = Block {
        id: BlockId(1),
        status: BlockStatus::Committed,
        kind: BlockKind::Paragraph,
        raw: "```mermaid\n".to_string(),
        display: None,
    };
    assert_eq!(b.code_fence_language(), None);
    assert!(b.code_fence_header().is_none());
}

