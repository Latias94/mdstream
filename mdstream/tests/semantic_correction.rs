use mdstream_protocol::{ContentKind, NodeId, NodeVersion, ResourceId};

#[path = "semantic_correction/footnote_overlay.rs"]
mod footnote_overlay;
#[path = "semantic_correction/limits.rs"]
mod limits;
#[path = "semantic_correction/references_citations_and_corrections.rs"]
mod references_citations_and_corrections;
#[path = "semantic_correction/topology.rs"]
mod topology;

fn reference(
    snapshot: &mdstream_protocol::Snapshot,
    start: u64,
) -> (NodeId, NodeVersion, Option<ResourceId>) {
    snapshot
        .nodes()
        .iter()
        .find_map(|node| {
            if node.source.start.get() != start {
                return None;
            }
            match &node.content {
                ContentKind::Link { target, .. } => Some((
                    node.id,
                    node.version.clone(),
                    target.as_ref().map(|target| target.id),
                )),
                _ => None,
            }
        })
        .expect("fixture reference must compile to a typed link")
}
