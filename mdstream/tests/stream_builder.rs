use std::sync::{Arc, Mutex};

use mdstream::{
    BoundaryPlugin, BoundaryUpdate, ContainerBoundaryPlugin, FnPendingTransformer, MdStream,
    MdStreamBuilder, Options, PendingTransformInput, PendingTransformer, Update,
};

fn collect_updates(mut stream: MdStream, chunks: &[&str]) -> Vec<Update> {
    let mut updates = Vec::new();
    for chunk in chunks {
        updates.push(stream.append(chunk));
    }
    updates.push(stream.finalize());
    updates
}

#[test]
fn builder_with_default_options_matches_mdstream_new() {
    let chunks = ["# Title\n\nPara", "graph\n\n- item\n"];

    let direct = MdStream::new(Options::default());
    let built = MdStream::builder(Options::default()).build();

    assert_eq!(
        collect_updates(direct, &chunks),
        collect_updates(built, &chunks)
    );
}

#[test]
fn builder_streamdown_defaults_match_mdstream_streamdown_defaults() {
    let chunks = ["See [docs](", " and ![alt]("];

    let direct = MdStream::streamdown_defaults();
    let built = MdStreamBuilder::streamdown_defaults().build();

    assert_eq!(
        collect_updates(direct, &chunks),
        collect_updates(built, &chunks)
    );
}

#[test]
fn builder_registers_boundary_plugins_and_pending_transformers() {
    let mut stream = MdStream::builder(Options::default())
        .boundary_plugin(ContainerBoundaryPlugin::default())
        .pending_transformer(FnPendingTransformer(|input: PendingTransformInput<'_>| {
            Some(format!("{}<<builder>>", input.display))
        }))
        .build();

    let update = stream.append("::: note\nbody\n:::\n\nTail");

    assert!(
        update
            .committed
            .iter()
            .any(|block| block.raw == "::: note\nbody\n:::\n")
    );
    assert_eq!(
        update.pending.as_ref().and_then(|p| p.display.as_deref()),
        Some("Tail<<builder>>")
    );
}

#[derive(Clone, Default)]
struct EventLog {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl EventLog {
    fn push(&self, event: &'static str) {
        self.events.lock().expect("event log poisoned").push(event);
    }

    fn snapshot(&self) -> Vec<&'static str> {
        self.events.lock().expect("event log poisoned").clone()
    }
}

struct ResetBoundary {
    log: EventLog,
}

impl BoundaryPlugin for ResetBoundary {
    fn matches_start(&self, line: &str) -> bool {
        line.trim_start().starts_with("{{")
    }

    fn start(&mut self, _line: &str) {}

    fn update(&mut self, line: &str) -> BoundaryUpdate {
        if line.trim() == "}}" {
            BoundaryUpdate::Close
        } else {
            BoundaryUpdate::Continue
        }
    }

    fn reset(&mut self) {
        self.log.push("boundary:reset");
    }
}

struct ResetTransformer {
    log: EventLog,
}

impl PendingTransformer for ResetTransformer {
    fn transform(&mut self, _input: PendingTransformInput<'_>) -> Option<String> {
        None
    }

    fn reset(&mut self) {
        self.log.push("transform:reset");
    }
}

#[test]
fn builder_registered_extensions_participate_in_reset_lifecycle() {
    let log = EventLog::default();
    let mut stream = MdStreamBuilder::default()
        .boundary_plugin(ResetBoundary { log: log.clone() })
        .pending_transformer(ResetTransformer { log: log.clone() })
        .build();

    stream.append("{{\nbody");
    stream.reset();

    let events = log.snapshot();
    assert!(events.contains(&"boundary:reset"), "{events:?}");
    assert!(events.contains(&"transform:reset"), "{events:?}");
}
