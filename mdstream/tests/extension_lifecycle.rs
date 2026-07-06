use std::sync::{Arc, Mutex};

use mdstream::{
    BoundaryPlugin, BoundaryUpdate, FnPendingTransformer, MdStream, Options, PendingTransformInput,
    PendingTransformer,
};

#[derive(Clone, Default)]
struct EventLog {
    events: Arc<Mutex<Vec<String>>>,
}

impl EventLog {
    fn push(&self, event: impl Into<String>) {
        self.events
            .lock()
            .expect("event log poisoned")
            .push(event.into());
    }

    fn snapshot(&self) -> Vec<String> {
        self.events.lock().expect("event log poisoned").clone()
    }
}

struct LoggingBoundaryPlugin {
    log: EventLog,
}

impl LoggingBoundaryPlugin {
    fn new(log: EventLog) -> Self {
        Self { log }
    }
}

impl BoundaryPlugin for LoggingBoundaryPlugin {
    fn matches_start(&self, line: &str) -> bool {
        line.trim_start().starts_with("{{")
    }

    fn start(&mut self, line: &str) {
        self.log.push(format!("boundary:start:{}", line.trim()));
    }

    fn update(&mut self, line: &str) -> BoundaryUpdate {
        self.log.push(format!("boundary:update:{}", line.trim()));
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

struct LoggingTransformer {
    log: EventLog,
}

impl LoggingTransformer {
    fn new(log: EventLog) -> Self {
        Self { log }
    }
}

impl PendingTransformer for LoggingTransformer {
    fn transform(&mut self, input: PendingTransformInput<'_>) -> Option<String> {
        self.log.push(format!(
            "transform:{}:{}",
            input.kind as u8,
            input.raw.len()
        ));
        None
    }

    fn reset(&mut self) {
        self.log.push("transform:reset");
    }
}

#[test]
fn boundary_plugin_start_and_update_order_is_stable() {
    let log = EventLog::default();
    let mut stream = MdStream::new(Options::default())
        .with_boundary_plugin(LoggingBoundaryPlugin::new(log.clone()));

    let update = stream.append("{{\nbody\n}}\n\nAfter\n");

    assert_eq!(
        log.snapshot(),
        vec![
            "boundary:start:{{".to_string(),
            "boundary:update:{{".to_string(),
            "boundary:update:body".to_string(),
            "boundary:update:}}".to_string(),
        ]
    );
    assert!(
        update
            .committed
            .iter()
            .any(|block| block.raw == "{{\nbody\n}}\n")
    );
}

#[test]
fn reset_calls_boundary_plugins_and_pending_transformers() {
    let log = EventLog::default();
    let mut stream = MdStream::new(Options::default())
        .with_boundary_plugin(LoggingBoundaryPlugin::new(log.clone()))
        .with_pending_transformer(LoggingTransformer::new(log.clone()));

    stream.append("{{\nbody");
    stream.reset();

    let events = log.snapshot();
    assert!(
        events.iter().any(|event| event == "boundary:reset"),
        "reset must reset boundary plugin state: {events:?}"
    );
    assert!(
        events.iter().any(|event| event == "transform:reset"),
        "reset must reset pending transformer state: {events:?}"
    );
}

#[test]
fn stateful_fn_pending_transformer_runs_for_each_pending_update() {
    let log = EventLog::default();
    let transform_log = log.clone();
    let transformer = FnPendingTransformer(move |input: PendingTransformInput<'_>| {
        transform_log.push(format!("fn-transform:{}", input.raw));
        Some(format!("{}<!--tick-->", input.display))
    });
    let mut stream = MdStream::new(Options::default()).with_pending_transformer(transformer);

    let first = stream.append("A");
    let second = stream.append("B");

    assert_eq!(
        first.pending.as_ref().and_then(|p| p.display.as_deref()),
        Some("A<!--tick-->")
    );
    assert_eq!(
        second.pending.as_ref().and_then(|p| p.display.as_deref()),
        Some("AB<!--tick-->")
    );
    assert_eq!(
        log.snapshot(),
        vec!["fn-transform:A".to_string(), "fn-transform:AB".to_string()]
    );
}
