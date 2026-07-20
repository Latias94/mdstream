#[allow(dead_code)]
#[path = "../examples/agent_tui.rs"]
mod agent_tui;

use mdstream_protocol::DocumentLifecycle;

#[tokio::test]
async fn agent_tui_smoke_uses_the_actor_and_finishes_without_terminal_control() {
    let summary = agent_tui::run_smoke().await.unwrap();

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
