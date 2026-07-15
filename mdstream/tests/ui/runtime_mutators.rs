use mdstream::StreamEngine;

fn push_boundary_plugin() {
    StreamEngine::new().push_boundary_plugin(());
}

fn with_boundary_plugin() {
    StreamEngine::new().with_boundary_plugin(());
}

fn push_pending_transformer() {
    StreamEngine::new().push_pending_transformer(());
}

fn with_pending_transformer() {
    StreamEngine::new().with_pending_transformer(());
}

fn mutable_and_borrowed_escape_hatches() {
    let mut engine = StreamEngine::new();
    let _ = engine.append_ref("text");
    let _ = engine.finalize_ref();
    let _ = engine.snapshot_blocks();
    let _ = engine.committed_mut();
}

fn main() {}
