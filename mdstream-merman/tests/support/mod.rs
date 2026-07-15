#![allow(dead_code)]

use mdstream_protocol::{
    ChangeId, ChangeSet, ChildList, ChildListOwner, CodeBlockSyntax, CodeFenceMarker, ContentKind,
    ContentNode, Epoch, NodeId, NodeStability, ProjectionOp, Reducer, SemanticText, SourceCursor,
    SourceDelta, SourceRange,
};

pub const EPOCH: Epoch = Epoch::new(7);
pub const NODE_ID: NodeId = NodeId::new(41);

pub fn mermaid_document(source: &str) -> Reducer {
    document_with_content(
        EPOCH,
        NODE_ID,
        source,
        NodeStability::Stable,
        ContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced {
                marker: CodeFenceMarker::Backtick,
                length: 3,
            },
            info: Some("mermaid".to_string()),
            text: SemanticText::Source {},
        },
    )
}

pub fn provisional_mermaid_document(source: &str) -> Reducer {
    document_with_content(
        EPOCH,
        NODE_ID,
        source,
        NodeStability::Provisional,
        ContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced {
                marker: CodeFenceMarker::Backtick,
                length: 3,
            },
            info: Some("mermaid".to_string()),
            text: SemanticText::Source {},
        },
    )
}

pub fn paragraph_document(source: &str) -> Reducer {
    document_with_content(
        EPOCH,
        NODE_ID,
        source,
        NodeStability::Stable,
        ContentKind::Paragraph {},
    )
}

pub fn document_with_content(
    epoch: Epoch,
    node_id: NodeId,
    source: &str,
    stability: NodeStability,
    content: ContentKind,
) -> Reducer {
    let end = SourceCursor::new(source.len() as u64);
    let range = SourceRange::new(SourceCursor::new(0), end);
    let node = ContentNode::leaf(node_id, stability, range, content);
    let roots = ChildList::new(vec![node_id]);
    let change = ChangeSet::start_epoch(
        epoch,
        ChangeId::new(format!("epoch:{}:node:{}", epoch.get(), node_id.get())).unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        vec![
            ProjectionOp::InsertNode { node },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: ChildList::empty().version().clone(),
                start: 0,
                delete_count: 0,
                insert: roots.as_slice().to_vec(),
                new_version: roots.version().clone(),
            },
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: end,
            },
        ],
    )
    .unwrap();
    let mut reducer = Reducer::new();
    reducer.apply(change).unwrap();
    reducer
}
