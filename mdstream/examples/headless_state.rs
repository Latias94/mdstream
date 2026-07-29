use std::collections::{BTreeMap, BTreeSet};

use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{
    ApplyOutcome, NodeStability, NodeVersion, TransitionNodeKey, TransitionReducer,
};

#[derive(Debug, PartialEq, Eq)]
enum NodeUpdate {
    Upsert {
        key: TransitionNodeKey,
        version: NodeVersion,
    },
    Remove {
        key: TransitionNodeKey,
    },
}

#[derive(Default)]
struct HeadlessState {
    reducer: TransitionReducer,
    rendered: BTreeMap<TransitionNodeKey, NodeVersion>,
    pending: Vec<NodeUpdate>,
}

impl HeadlessState {
    fn apply(&mut self, output: EngineOutput) -> Result<(), Box<dyn std::error::Error>> {
        for change in output.into_changes() {
            let outcome = self.reducer.apply(change)?;
            let impact = match outcome.outcome {
                ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => {
                    impact
                }
                other => {
                    return Err(format!("producer change was not continuous: {other:?}").into());
                }
            };
            let document = self
                .reducer
                .document()
                .expect("applied output has a document");
            let epoch = document.coordinate().epoch;
            let continuity_generation = self.reducer.continuity_generation();

            if impact.full_replace {
                self.pending.extend(
                    std::mem::take(&mut self.rendered)
                        .into_keys()
                        .map(|key| NodeUpdate::Remove { key }),
                );
                for node in document.nodes() {
                    let key = TransitionNodeKey {
                        continuity_generation,
                        epoch,
                        node_id: node.id,
                    };
                    self.rendered.insert(key, node.version.clone());
                    self.pending.push(NodeUpdate::Upsert {
                        key,
                        version: node.version.clone(),
                    });
                }
                continue;
            }

            let removed_nodes = impact
                .removed_nodes
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            for node_id in impact.changed_nodes {
                let key = TransitionNodeKey {
                    continuity_generation,
                    epoch,
                    node_id,
                };
                if removed_nodes.contains(&node_id) || document.node(node_id).is_none() {
                    self.rendered.remove(&key);
                    self.pending.push(NodeUpdate::Remove { key });
                } else {
                    let version = document.node(node_id).unwrap().version.clone();
                    self.rendered.insert(key, version.clone());
                    self.pending.push(NodeUpdate::Upsert { key, version });
                }
            }
        }
        Ok(())
    }

    fn take_node_updates(&mut self) -> Vec<NodeUpdate> {
        std::mem::take(&mut self.pending)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = StreamEngine::new();
    let mut state = HeadlessState::default();

    state.apply(engine.append("# Stable identity\n\nThe host")?)?;
    let document = state.reducer.document().unwrap();
    let paragraph_id = document.roots().as_slice()[1];
    let paragraph_key = TransitionNodeKey {
        continuity_generation: state.reducer.continuity_generation(),
        epoch: document.coordinate().epoch,
        node_id: paragraph_id,
    };
    assert_eq!(
        document.node(paragraph_id).unwrap().stability,
        NodeStability::Provisional
    );
    state.take_node_updates();

    state.apply(engine.append(" updates only changed nodes.\n\n")?)?;
    let append_updates = state.take_node_updates();
    assert_eq!(
        state.reducer.document().unwrap().roots().as_slice()[1],
        paragraph_id,
        "stabilizing append must preserve the paragraph identity"
    );
    assert_eq!(
        state
            .reducer
            .document()
            .unwrap()
            .node(paragraph_id)
            .unwrap()
            .stability,
        NodeStability::Stable
    );
    let paragraph_version = &state
        .reducer
        .document()
        .unwrap()
        .node(paragraph_id)
        .unwrap()
        .version;
    assert!(append_updates.iter().any(|update| {
        matches!(
            update,
            NodeUpdate::Upsert { key, version }
                if *key == paragraph_key && version == paragraph_version
        )
    }));

    state.apply(engine.finish()?)?;
    let finish_updates = state.take_node_updates();
    assert_eq!(
        state.reducer.document().unwrap().roots().as_slice()[1],
        paragraph_id,
        "finish must preserve the paragraph identity"
    );

    let rendered_before_reset = state.rendered.keys().copied().collect::<BTreeSet<_>>();
    state.apply(engine.reset()?)?;
    let reset_updates = state.take_node_updates();
    assert!(rendered_before_reset.iter().all(|key| {
        reset_updates
            .iter()
            .any(|update| matches!(update, NodeUpdate::Remove { key: removed } if removed == key))
    }));

    println!(
        "append_and_stabilize identity={} invalidations={} host_action=upsert-changed-only",
        paragraph_key.node_id,
        append_updates.len()
    );
    println!(
        "finish identity={} invalidations={} host_action=retain-key",
        paragraph_key.node_id,
        finish_updates.len()
    );
    println!(
        "reset removed_keys={} host_action=discard-prior-epoch-state",
        rendered_before_reset.len()
    );
    Ok(())
}
