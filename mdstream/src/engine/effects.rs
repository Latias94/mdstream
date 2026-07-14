use std::collections::BTreeMap;

use mdstream_protocol::{
    ChangeSet, ChildList, ChildListOwner, ContentKind, ContentNode, NodeId, NodeStability,
    ProjectionOp, SourceCursor, SourceRange,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOutput {
    changes: Vec<ChangeSet>,
}

impl EngineOutput {
    pub(crate) fn one(change: ChangeSet) -> Self {
        Self {
            changes: vec![change],
        }
    }

    pub fn changes(&self) -> &[ChangeSet] {
        &self.changes
    }

    pub fn into_changes(self) -> Vec<ChangeSet> {
        self.changes
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct FrameShell {
    node: Option<ContentNode>,
}

impl FrameShell {
    pub(super) fn append(&self, source_end: SourceCursor) -> (Vec<ProjectionOp>, Self) {
        self.project(source_end, NodeStability::Provisional)
    }

    pub(super) fn finish(&self, source_end: SourceCursor) -> (Vec<ProjectionOp>, Self) {
        let (mut operations, next) = self.project(source_end, NodeStability::Stable);
        operations.push(ProjectionOp::FinishDocument);
        (operations, next)
    }

    fn project(
        &self,
        source_end: SourceCursor,
        stability: NodeStability,
    ) -> (Vec<ProjectionOp>, Self) {
        if source_end == SourceCursor::new(0) {
            return (Vec::new(), Self::default());
        }

        let next = frame_node(source_end, stability);
        let operations = if let Some(current) = &self.node {
            if current.stability == NodeStability::Provisional
                && stability == NodeStability::Stable
                && current.source == next.source
                && current.body == next.body
                && current.content == next.content
            {
                vec![ProjectionOp::StabilizeNode {
                    node_id: current.id,
                    expected_version: current.version.clone(),
                    new_version: next.version.clone(),
                }]
            } else {
                vec![ProjectionOp::ReplaceNode {
                    node_id: current.id,
                    expected_version: current.version.clone(),
                    projection: next.projection(),
                }]
            }
        } else {
            let empty = ChildList::empty();
            let roots = ChildList::new(vec![next.id]);
            vec![
                ProjectionOp::InsertNode { node: next.clone() },
                ProjectionOp::SpliceChildren {
                    owner: ChildListOwner::Document,
                    expected_version: empty.version,
                    start: 0,
                    delete_count: 0,
                    insert: vec![next.id],
                    new_version: roots.version,
                },
            ]
        };
        (operations, Self { node: Some(next) })
    }
}

fn frame_node(source_end: SourceCursor, stability: NodeStability) -> ContentNode {
    let source = SourceRange::new(SourceCursor::new(0), source_end);
    ContentNode::leaf(
        NodeId::new(0),
        stability,
        source,
        ContentKind::Custom {
            namespace: "mdstream.frame/1".to_string(),
            name: "frontier".to_string(),
            opaque: true,
            attributes: BTreeMap::new(),
        },
    )
}
