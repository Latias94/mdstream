mod support;

use mdstream::{MdStream, Options, TagBoundaryPlugin};

#[test]
fn thinking_tag_container_is_single_block() {
    let markdown = "Intro\n\n<thinking>\nA\n\nB\n</thinking>\n\nAfter\n";
    let blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(TagBoundaryPlugin::thinking()),
    );
    assert_eq!(
        blocks,
        vec![
            "Intro\n\n".to_string(),
            "<thinking>\nA\n\nB\n</thinking>\n".to_string(),
            "After\n".to_string(),
        ]
    );
}

#[test]
fn thinking_tag_container_chunking_invariance() {
    let markdown = "Intro\n\n<thinking>\nA\n\nB\n</thinking>\n\nAfter\n";
    let blocks_whole = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(TagBoundaryPlugin::thinking()),
    );
    let blocks_lines = support::collect_final_raw_with_stream(
        support::chunk_lines(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(TagBoundaryPlugin::thinking()),
    );
    let blocks_chars = support::collect_final_raw_with_stream(
        support::chunk_chars(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(TagBoundaryPlugin::thinking()),
    );
    let blocks_rand = support::collect_final_raw_with_stream(
        support::chunk_pseudo_random(
            markdown,
            "thinking_tag_container_chunking_invariance",
            0,
            40,
        ),
        MdStream::new(Options::default()).with_boundary_plugin(TagBoundaryPlugin::thinking()),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_chars, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);
}

#[test]
fn tag_plugin_reset_clears_state() {
    let mut s =
        MdStream::new(Options::default()).with_boundary_plugin(TagBoundaryPlugin::thinking());
    s.append("<thinking>\nA\n");
    s.reset();
    let u = s.append("A\n\nB\n");
    assert_eq!(u.committed.len(), 1);
    assert_eq!(u.committed[0].raw, "A\n\n");
    assert_eq!(u.pending.as_ref().unwrap().raw, "B\n");
}
