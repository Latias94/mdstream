use mdstream::TagBoundaryPlugin;
use mdstream::{AnalyzedStream, Options, TaggedBlockAnalyzer};

#[test]
fn tagged_block_analyzer_extracts_tag_and_content() {
    let a = TaggedBlockAnalyzer {
        allowed_tags: Some(vec!["thinking".to_string(), "custom_block".to_string()]),
        ..Default::default()
    };

    let mut s = AnalyzedStream::new(Options::default(), a);

    let u1 = s.append("<thinking>\nstep 1\n");
    let m1 = u1.pending_meta.expect("pending meta").meta;
    assert_eq!(m1.tag, "thinking");
    assert_eq!(m1.attributes, None);
    assert!(!m1.closed);
    assert_eq!(m1.content, "step 1\n");

    let u2 = s.append("</thinking>\n\nAfter\n");
    // The tag block should be committed now.
    assert!(
        u2.update
            .committed
            .iter()
            .any(|b| b.raw.contains("<thinking>"))
    );
    let committed_meta = u2
        .committed_meta
        .iter()
        .find(|m| m.meta.tag == "thinking")
        .expect("committed thinking meta");
    assert!(committed_meta.meta.closed);
    assert_eq!(committed_meta.meta.content, "step 1\n");
}

#[test]
fn tagged_block_analyzer_ignores_non_standalone_closing_tag() {
    let mut s = AnalyzedStream::new(Options::default(), TaggedBlockAnalyzer::default());
    let u = s.append("<thinking>\na</thinking> trailing\n");
    // `<thinking>` is treated as an HTML block by the stream; verify the analyzer still extracts meta.
    let m = u
        .committed_meta
        .into_iter()
        .next()
        .expect("committed meta")
        .meta;
    assert_eq!(m.tag, "thinking");
    assert!(!m.closed);
}

#[test]
fn tagged_block_analyzer_handles_tool_call_tag_with_boundary_plugin() {
    let a = TaggedBlockAnalyzer {
        allowed_tags: Some(vec!["custom_block".to_string()]),
        ..Default::default()
    };
    let mut s = AnalyzedStream::new(Options::default(), a);
    s.inner_mut()
        .push_boundary_plugin(TagBoundaryPlugin::new("custom_block"));

    let u1 = s.append("<custom_block>\n{\"name\":\"x\"");
    let m1 = u1.pending_meta.expect("pending meta").meta;
    assert_eq!(m1.tag, "custom_block");
    assert!(!m1.closed);

    let u2 = s.append("\n}\n</custom_block>\n");
    assert!(
        u2.update
            .committed
            .iter()
            .any(|b| b.raw.contains("<custom_block>"))
    );
    let m2 = u2
        .committed_meta
        .iter()
        .find(|m| m.meta.tag == "custom_block")
        .expect("committed custom_block meta");
    assert!(m2.meta.closed);
    assert!(m2.meta.content.contains("{\"name\""));
}
