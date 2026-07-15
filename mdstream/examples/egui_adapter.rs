use std::collections::BTreeSet;

use mdstream::{EngineOutput, StreamEngine};
use mdstream_protocol::{ApplyOutcome, NodeId, NodeVersion, Reducer};

#[derive(Debug)]
enum WidgetUpdate {
    Upsert { key: NodeId, version: NodeVersion },
    Remove { key: NodeId },
}

#[derive(Default)]
struct EguiDocument {
    reducer: Reducer,
    dirty: BTreeSet<NodeId>,
}

impl EguiDocument {
    fn apply(&mut self, output: EngineOutput) {
        for change in output.into_changes() {
            let outcome = self.reducer.apply(change).unwrap();
            let impact = match outcome {
                ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => {
                    impact
                }
                other => panic!("engine output must be continuous, got {other:?}"),
            };
            self.dirty.extend(impact.changed_nodes);
            self.dirty.extend(impact.removed_nodes);
        }
    }

    fn take_widget_updates(&mut self) -> Vec<WidgetUpdate> {
        let document = self.reducer.document();
        std::mem::take(&mut self.dirty)
            .into_iter()
            .map(
                |key| match document.and_then(|document| document.node(key)) {
                    Some(node) => WidgetUpdate::Upsert {
                        key,
                        version: node.version.clone(),
                    },
                    None => WidgetUpdate::Remove { key },
                },
            )
            .collect()
    }
}

fn main() {
    let mut engine = StreamEngine::new();
    let mut view = EguiDocument::default();

    view.apply(engine.append("# Stable key\n\nBody").unwrap());
    for update in view.take_widget_updates() {
        match update {
            WidgetUpdate::Upsert { key, version } => println!("upsert {key} {version}"),
            WidgetUpdate::Remove { key } => println!("remove {key}"),
        }
    }
}
