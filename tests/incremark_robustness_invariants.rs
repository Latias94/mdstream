mod support;

use mdstream::{BlockKind, MdStream, Options};

#[test]
fn empty_input_produces_no_blocks() {
    let mut s = MdStream::new(Options::default());
    let u = s.append("");
    assert!(u.committed.is_empty());
    assert!(u.pending.is_none());
    assert!(!u.reset);

    let u = s.finalize();
    assert!(u.committed.is_empty());
    assert!(u.pending.is_none());
    assert!(!u.reset);
}

#[test]
fn many_empty_appends_produce_no_blocks() {
    let mut s = MdStream::new(Options::default());
    for _ in 0..100 {
        let u = s.append("");
        assert!(u.committed.is_empty());
        assert!(u.pending.is_none());
        assert!(!u.reset);
    }
    let u = s.finalize();
    assert!(u.committed.is_empty());
    assert!(u.pending.is_none());
    assert!(!u.reset);
}

#[test]
fn whitespace_only_input_is_ignored() {
    let mut s = MdStream::new(Options::default());
    let _ = s.append("   \n\n\t\t\n   \n");
    let u = s.finalize();
    assert!(u.committed.is_empty());
}

#[test]
fn single_char_becomes_single_committed_block_on_finalize() {
    let mut s = MdStream::new(Options::default());
    let u1 = s.append("a");
    assert!(u1.pending.is_some());
    assert_eq!(u1.pending.as_ref().unwrap().raw, "a");

    let u2 = s.finalize();
    assert_eq!(u2.committed.len(), 1);
    assert_eq!(u2.committed[0].raw, "a");
}

#[test]
fn newline_normalization_handles_crlf_and_cr() {
    let opts = Options::default();

    let lf =
        support::collect_final_blocks(support::chunk_whole("# Title\n\nParagraph\n"), opts.clone());
    let crlf = support::collect_final_blocks(
        support::chunk_whole("# Title\r\n\r\nParagraph\r\n"),
        opts.clone(),
    );
    let cr = support::collect_final_blocks(support::chunk_whole("# Title\r\rParagraph\r"), opts);

    assert_eq!(crlf, lf);
    assert_eq!(cr, lf);
}

#[test]
fn finalize_is_idempotent_without_new_input() {
    let mut s = MdStream::new(Options::default());
    let _ = s.append("# Hello");
    let u1 = s.finalize();
    assert_eq!(u1.committed.len(), 1);
    assert_eq!(u1.committed[0].kind, BlockKind::Heading);

    let u2 = s.finalize();
    assert!(u2.committed.is_empty());
    assert!(u2.pending.is_none());

    let u3 = s.finalize();
    assert!(u3.committed.is_empty());
    assert!(u3.pending.is_none());
}

#[test]
fn footnote_detection_mid_stream_triggers_reset_and_single_pending_block() {
    let mut s = MdStream::new(Options::default());

    // First append commits normal blocks.
    let u1 = s.append("# H\n\nParagraph\n\n");
    assert!(!u1.reset);
    assert!(!u1.committed.is_empty());

    // Later, a footnote appears. Streamdown would switch to [full markdown] as one block.
    let u2 = s.append("With footnote[^1].\n");
    assert!(u2.reset);
    assert!(u2.committed.is_empty());

    let pending = u2.pending.as_ref().expect("single pending block");
    assert_eq!(pending.id.0, 1);
    assert!(pending.raw.contains("Paragraph"));
    assert!(pending.raw.contains("[^1]"));

    // The current snapshot must contain exactly the single pending block (no duplicate committed blocks).
    let snapshot = s.snapshot_blocks();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].status, mdstream::BlockStatus::Pending);
    assert_eq!(snapshot[0].id.0, 1);
    assert!(snapshot[0].raw.contains("[^1]"));

    // Finalize must produce exactly one committed block (idempotent afterwards).
    let u3 = s.finalize();
    assert!(!u3.reset);
    assert_eq!(u3.committed.len(), 1);
    assert_eq!(u3.committed[0].id.0, 1);
    assert!(u3.committed[0].raw.contains("[^1]"));
    let u4 = s.finalize();
    assert!(u4.committed.is_empty());
}
