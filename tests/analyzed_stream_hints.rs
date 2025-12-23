use mdstream::{AnalyzedStream, BlockHintAnalyzer, BlockHintMeta, Options};

#[test]
fn hint_marks_transformed_pending_display() {
    let mut s = AnalyzedStream::new(Options::default(), BlockHintAnalyzer);
    let u = s.append("**bold");
    let meta = u.pending_meta.expect("pending meta").meta;
    assert!(meta.has(BlockHintMeta::DISPLAY_TRANSFORMED));
    assert!(meta.likely_incomplete());
}

#[test]
fn hint_marks_unclosed_code_fence() {
    let mut s = AnalyzedStream::new(Options::default(), BlockHintAnalyzer);
    let u1 = s.append("```js\nconst x = 1;\n");
    let meta1 = u1.pending_meta.expect("pending meta").meta;
    assert!(meta1.has(BlockHintMeta::UNCLOSED_CODE_FENCE));
    assert!(meta1.likely_incomplete());

    // Closing fence without trailing newline: should be considered closed, even though it's still pending
    // until finalize (streaming-friendly behavior).
    let u2 = s.append("```");
    let meta2 = u2.pending_meta.expect("pending meta").meta;
    assert!(!meta2.has(BlockHintMeta::UNCLOSED_CODE_FENCE));
}

#[test]
fn hint_marks_unbalanced_math_block() {
    let mut s = AnalyzedStream::new(Options::default(), BlockHintAnalyzer);
    let u1 = s.append("$$\nE = mc^2\n");
    let meta1 = u1.pending_meta.expect("pending meta").meta;
    assert!(meta1.has(BlockHintMeta::UNBALANCED_MATH));
    assert!(meta1.likely_incomplete());

    let u2 = s.append("$$");
    let meta2 = u2.pending_meta.expect("pending meta").meta;
    assert!(!meta2.has(BlockHintMeta::UNBALANCED_MATH));
}
