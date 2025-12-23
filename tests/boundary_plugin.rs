mod support;

use mdstream::{FenceBoundaryPlugin, MdStream, Options};

#[test]
fn triple_colon_container_is_single_block() {
    let markdown = "Intro\n\n:::warning\nA\n\nB\n:::\n\nAfter\n";

    let mut s =
        MdStream::new(Options::default()).with_boundary_plugin(FenceBoundaryPlugin::triple_colon());
    let u1 = s.append(markdown);
    let u2 = s.finalize();
    let blocks: Vec<String> = u1
        .committed
        .into_iter()
        .chain(u2.committed)
        .map(|b| b.raw)
        .collect();

    assert_eq!(
        blocks,
        vec![
            "Intro\n\n".to_string(),
            ":::warning\nA\n\nB\n:::\n".to_string(),
            "After\n".to_string(),
        ]
    );
}

#[test]
fn triple_colon_container_chunking_invariance() {
    let markdown = "Intro\n\n:::note\nA\n\nB\n:::\n\nAfter\n";
    let opts = Options::default();

    let blocks_whole = support::collect_final_raw_with_stream(
        support::chunk_whole(markdown),
        MdStream::new(opts.clone()).with_boundary_plugin(FenceBoundaryPlugin::triple_colon()),
    );
    let blocks_lines = support::collect_final_raw_with_stream(
        support::chunk_lines(markdown),
        MdStream::new(opts.clone()).with_boundary_plugin(FenceBoundaryPlugin::triple_colon()),
    );
    let blocks_chars = support::collect_final_raw_with_stream(
        support::chunk_chars(markdown),
        MdStream::new(opts.clone()).with_boundary_plugin(FenceBoundaryPlugin::triple_colon()),
    );
    let blocks_rand = support::collect_final_raw_with_stream(
        support::chunk_pseudo_random(
            markdown,
            "triple_colon_container_chunking_invariance",
            0,
            40,
        ),
        MdStream::new(opts).with_boundary_plugin(FenceBoundaryPlugin::triple_colon()),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_chars, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);
}

#[test]
fn reset_clears_boundary_plugin_state() {
    let mut s =
        MdStream::new(Options::default()).with_boundary_plugin(FenceBoundaryPlugin::triple_colon());
    s.append(":::\nA\n");
    s.reset();

    let u = s.append("A\n\nB\n");
    assert_eq!(u.committed.len(), 1);
    assert_eq!(u.committed[0].raw, "A\n\n");
    assert_eq!(u.pending.as_ref().unwrap().raw, "B\n");
}
