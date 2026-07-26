#[allow(dead_code)]
#[path = "../examples/agent_tui.rs"]
mod agent_tui;

#[cfg(feature = "rich-tui")]
#[allow(dead_code)]
#[path = "../examples/agent_tui_rich.rs"]
mod agent_tui_rich;

use mdstream_protocol::DocumentLifecycle;

#[tokio::test]
async fn agent_tui_smoke_uses_the_actor_and_finishes_without_terminal_control() {
    let summary = agent_tui::run_smoke().await.unwrap();

    agent_tui::validate_smoke_summary(&summary).unwrap();

    assert_eq!(summary.lifecycle, DocumentLifecycle::Finalized);
    assert_eq!(summary.source, agent_tui::DEMO_MARKDOWN);
    assert_eq!(summary.input_capacity, agent_tui::INPUT_CAPACITY);
    assert_eq!(
        summary.commands_sent,
        agent_tui::DEMO_MARKDOWN.chars().count() as u64
    );
    assert!(summary.commands_sent > summary.input_capacity as u64);
    assert!(summary.batches > 0);
    assert!(summary.changes >= summary.batches);
    assert_eq!(summary.errors, 0);
}

#[test]
fn smoke_summary_rejects_actor_errors() {
    let mut summary = valid_summary();
    summary.errors = 1;

    let error = agent_tui::validate_smoke_summary(&summary).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("errors=1"));
}

#[test]
fn smoke_summary_rejects_an_open_document() {
    let mut summary = valid_summary();
    summary.lifecycle = DocumentLifecycle::Open;

    let error = agent_tui::validate_smoke_summary(&summary).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("lifecycle=Open"));
}

#[test]
fn smoke_summary_rejects_source_loss() {
    let mut summary = valid_summary();
    summary.source.pop();

    let error = agent_tui::validate_smoke_summary(&summary).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("source"));
}

#[test]
fn smoke_summary_rejects_an_incomplete_command_count() {
    let mut summary = valid_summary();
    summary.commands_sent -= 1;

    let error = agent_tui::validate_smoke_summary(&summary).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("commands_sent"));
}

#[test]
fn smoke_summary_rejects_an_unexpected_input_capacity() {
    let mut summary = valid_summary();
    summary.input_capacity += 1;

    let error = agent_tui::validate_smoke_summary(&summary).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("input_capacity"));
}

#[test]
fn smoke_summary_rejects_missing_output_batches() {
    let mut summary = valid_summary();
    summary.batches = 0;

    let error = agent_tui::validate_smoke_summary(&summary).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("batches=0"));
}

#[test]
fn smoke_summary_rejects_fewer_changes_than_batches() {
    let mut summary = valid_summary();
    summary.batches = 2;
    summary.changes = 1;

    let error = agent_tui::validate_smoke_summary(&summary).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("changes=1"));
    assert!(error.to_string().contains("batches=2"));
}

fn valid_summary() -> agent_tui::SmokeSummary {
    agent_tui::SmokeSummary {
        source: agent_tui::DEMO_MARKDOWN.to_string(),
        lifecycle: DocumentLifecycle::Finalized,
        input_capacity: agent_tui::INPUT_CAPACITY,
        commands_sent: agent_tui::DEMO_MARKDOWN.chars().count() as u64,
        batches: 1,
        changes: 1,
        errors: 0,
    }
}

#[cfg(feature = "rich-tui")]
#[tokio::test]
async fn rich_agent_tui_smoke_renders_semantic_content_and_settles_host_policy() {
    let summary = agent_tui_rich::run_smoke().await.unwrap();

    agent_tui_rich::validate_smoke_summary(&summary).unwrap();

    assert_eq!(summary.lifecycle, DocumentLifecycle::Finalized);
    assert_eq!(summary.source, agent_tui_rich::DEMO_MARKDOWN);
    assert_eq!(summary.input_capacity, agent_tui_rich::INPUT_CAPACITY);
    assert_eq!(
        summary.commands_sent,
        agent_tui_rich::DEMO_MARKDOWN.chars().count() as u64
    );
    assert!(summary.commands_sent > summary.input_capacity as u64);
    assert!(summary.batches > 0);
    assert!(summary.changes >= summary.batches);
    assert_eq!(summary.errors, 0);
    assert!(!summary.reduced_motion);
    assert!(summary.reconciliations > 0);
    assert!(summary.enqueued_lines > 0);
    assert!(summary.committed_lines > 0);
    assert_eq!(summary.queued_lines, 0);
    assert_eq!(summary.mutable_roots, 0);
    assert!(summary.catch_up_entries > 0);
    assert!(summary.max_queue_depth > 0);
    assert!(summary.stable_roots_rendered > 0);
    assert!(summary.stable_roots_reused > 0);
    assert!(summary.canonical_render_equal);
    assert!(summary.idle_without_tick);
    assert!(summary.semantic_lines >= 10);
    assert!(summary.highlighted_segments > 0);
    assert_eq!(summary.completed_activities, 3);
}

#[cfg(feature = "rich-tui")]
#[tokio::test]
async fn rich_agent_tui_reduced_motion_converges_without_a_paced_queue() {
    let summary = agent_tui_rich::run_smoke_with_motion(true).await.unwrap();

    agent_tui_rich::validate_smoke_summary(&summary).unwrap();

    assert!(summary.reduced_motion);
    assert_eq!(summary.queued_lines, 0);
    assert_eq!(summary.mutable_roots, 0);
    assert!(summary.canonical_render_equal);
    assert!(summary.idle_without_tick);
}

#[cfg(feature = "rich-tui")]
#[test]
fn rich_smoke_summary_rejects_missing_syntax_highlighting() {
    let mut summary = rich_valid_summary();
    summary.highlighted_segments = 0;

    let error = agent_tui_rich::validate_smoke_summary(&summary).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("Tree-sitter"));
}

#[cfg(feature = "rich-tui")]
fn rich_valid_summary() -> agent_tui_rich::SmokeSummary {
    agent_tui_rich::SmokeSummary {
        source: agent_tui_rich::DEMO_MARKDOWN.to_string(),
        lifecycle: DocumentLifecycle::Finalized,
        input_capacity: agent_tui_rich::INPUT_CAPACITY,
        commands_sent: agent_tui_rich::DEMO_MARKDOWN.chars().count() as u64,
        batches: 1,
        changes: 1,
        errors: 0,
        reduced_motion: false,
        reconciliations: 1,
        enqueued_lines: 10,
        committed_lines: 10,
        queued_lines: 0,
        mutable_roots: 0,
        catch_up_entries: 1,
        max_queue_depth: 10,
        stable_roots_rendered: 1,
        stable_roots_reused: 1,
        canonical_render_equal: true,
        idle_without_tick: true,
        semantic_lines: 10,
        highlighted_segments: 1,
        completed_activities: 3,
    }
}
