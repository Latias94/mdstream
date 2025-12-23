mod support;

use mdstream::{ContainerBoundaryPlugin, MdStream, Options};

#[test]
fn detects_container_start_and_end() {
    let markdown = "Intro\n\n::: warning\nA\n:::\n\nAfter\n";
    let blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(ContainerBoundaryPlugin::default()),
    );
    assert_eq!(
        blocks,
        vec![
            "Intro\n\n".to_string(),
            "::: warning\nA\n:::\n".to_string(),
            "After\n".to_string(),
        ]
    );
}

#[test]
fn longer_markers_allow_nesting_depth() {
    let markdown = "::: outer\nA\n::::: inner\nB\n:::\nC\n:::\nAfter\n";
    let blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(ContainerBoundaryPlugin::default()),
    );
    assert_eq!(
        blocks,
        vec![
            "::: outer\nA\n::::: inner\nB\n:::\nC\n:::\n".to_string(),
            "After\n".to_string(),
        ]
    );
}

#[test]
fn allowed_names_filters_opening() {
    let mut plugin = ContainerBoundaryPlugin::default();
    plugin.allowed_names = Some(vec!["warning".to_string(), "info".to_string()]);

    let markdown = "::: danger\nA\n:::\n\nAfter\n";
    let blocks = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(plugin),
    );

    // Not detected as a container -> split by blank line only.
    assert_eq!(
        blocks,
        vec!["::: danger\nA\n:::\n\n".to_string(), "After\n".to_string()]
    );
}

#[test]
fn chunking_invariance_for_containers() {
    let markdown = "Intro\n\n::: note attr=1\nA\n\nB\n:::\n\nAfter\n";
    let plugin = ContainerBoundaryPlugin::default();

    let blocks_whole = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(plugin.clone()),
    );
    let blocks_lines = support::collect_final_raw_with_stream(
        support::chunk_lines(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(plugin.clone()),
    );
    let blocks_chars = support::collect_final_raw_with_stream(
        support::chunk_chars(markdown),
        MdStream::new(Options::default()).with_boundary_plugin(plugin.clone()),
    );
    let blocks_rand = support::collect_final_raw_with_stream(
        support::chunk_pseudo_random(markdown, "chunking_invariance_for_containers", 0, 40),
        MdStream::new(Options::default()).with_boundary_plugin(plugin.clone()),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_chars, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);
}
