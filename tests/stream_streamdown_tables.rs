mod support;

use mdstream::{BlockKind, Options};

#[test]
fn streamdown_benchmark_simple_table_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("Tables").
    let markdown = "\n| Header 1 | Header 2 |\n|----------|----------|\n| Cell 1   | Cell 2   |\n| Cell 3   | Cell 4   |\n";

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            markdown,
            "streamdown_benchmark_simple_table_chunking_invariance",
            0,
            40,
        ),
        opts,
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 1);
    assert_eq!(blocks_whole[0].0, BlockKind::Table);
    assert!(blocks_whole[0].1.contains("| Header 1 | Header 2 |"));
}

#[test]
fn streamdown_benchmark_large_table_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("Tables").
    let mut markdown = String::new();
    markdown.push_str("\n| H1 | H2 | H3 | H4 | H5 |\n|----|----|----|----|-------|\n");
    for i in 0..100 {
        markdown.push_str(&format!("| C{i}1 | C{i}2 | C{i}3 | C{i}4 | C{i}5 |\n"));
    }

    let opts = Options::default();
    let blocks_whole = support::collect_final_blocks(support::chunk_whole(&markdown), opts.clone());
    let blocks_lines = support::collect_final_blocks(support::chunk_lines(&markdown), opts.clone());
    let blocks_rand = support::collect_final_blocks(
        support::chunk_pseudo_random(
            &markdown,
            "streamdown_benchmark_large_table_chunking_invariance",
            0,
            40,
        ),
        opts,
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);

    assert_eq!(blocks_whole.len(), 1);
    assert_eq!(blocks_whole[0].0, BlockKind::Table);
    assert!(blocks_whole[0].1.contains("| C991 | C992 | C993 | C994 | C995 |"));
}

