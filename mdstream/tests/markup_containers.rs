mod support;

use mdstream::{
    AnalyzedStream, ContainerBoundaryPlugin, FenceBoundaryPlugin, MdStream, Options,
    TagBoundaryPlugin, TaggedBlockAnalyzer,
};

#[test]
fn tag_boundary_accepts_up_to_three_leading_spaces() {
    let markdown = "Intro\n\n   <thinking>\nA\n   </thinking>\n\nAfter\n";
    let blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(TagBoundaryPlugin::thinking()),
    );

    assert_eq!(
        blocks,
        vec![
            "Intro\n\n".to_string(),
            "   <thinking>\nA\n   </thinking>\n".to_string(),
            "After\n".to_string(),
        ]
    );
}

#[test]
fn tag_boundary_does_not_accept_four_leading_spaces() {
    let markdown = "Intro\n\n    <thinking>\nA\n    </thinking>\n\nAfter\n";
    let blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(TagBoundaryPlugin::thinking()),
    );

    assert_eq!(
        blocks,
        vec![
            "Intro\n\n".to_string(),
            "    <thinking>\nA\n    </thinking>\n\n".to_string(),
            "After\n".to_string(),
        ]
    );
}

#[test]
fn tagged_block_analyzer_does_not_analyze_indented_code_like_tag_block() {
    let mut s = AnalyzedStream::new(Options::default(), TaggedBlockAnalyzer::default());
    let u = s.append("    <thinking>\nA\n    </thinking>\n");

    assert!(u.committed_meta.is_empty());
    assert!(u.pending_meta.is_none());

    let u = s.finalize();
    assert!(u.committed_meta.is_empty());
    assert!(u.pending_meta.is_none());
}

#[test]
fn tagged_block_analyzer_treats_trailing_closing_text_as_open() {
    let mut s = AnalyzedStream::new(Options::default(), TaggedBlockAnalyzer::default());
    let u = s.append("<thinking>\nA\n</thinking> trailing\n");
    let meta = u.committed_meta.into_iter().next().expect("tag meta").meta;

    assert_eq!(meta.tag, "thinking");
    assert!(!meta.closed);
    assert!(meta.content.contains("</thinking> trailing"));
}

#[test]
fn directive_container_and_fence_container_keep_distinct_start_syntax() {
    let directive = "Intro\n\n::: warning\nA\n:::\n\nAfter\n";
    let fence = "Intro\n\n:::warning\nA\n:::\n\nAfter\n";

    let directive_blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(directive),
        MdStream::new(Options::default()).with_boundary_plugin(ContainerBoundaryPlugin::default()),
    );
    let fence_blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(fence),
        MdStream::new(Options::default()).with_boundary_plugin(FenceBoundaryPlugin::triple_colon()),
    );
    let directive_plugin_on_fence_syntax = support::collect_final_raw_with_stream(
        support::chunk_whole(fence),
        MdStream::new(Options::default()).with_boundary_plugin(ContainerBoundaryPlugin::default()),
    );

    assert_eq!(directive_blocks[1], "::: warning\nA\n:::\n");
    assert_eq!(fence_blocks[1], ":::warning\nA\n:::\n");
    assert_eq!(
        directive_plugin_on_fence_syntax,
        vec![
            "Intro\n\n".to_string(),
            ":::warning\nA\n:::\n\n".to_string(),
            "After\n".to_string()
        ]
    );
}

#[test]
fn directive_container_nested_longer_markers_stay_in_one_block() {
    let markdown = ":::: outer\nA\n:::::: inner\nB\n::::\nC\n::::\nAfter\n";
    let blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(ContainerBoundaryPlugin::default()),
    );

    assert_eq!(
        blocks,
        vec![
            ":::: outer\nA\n:::::: inner\nB\n::::\nC\n::::\n".to_string(),
            "After\n".to_string(),
        ]
    );
}
