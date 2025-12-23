use mdstream::{AnalyzedStream, Options, TagBoundaryPlugin, ToolCallJsonAnalyzer};

#[test]
fn tool_call_json_analyzer_extracts_candidate() {
    let mut a = ToolCallJsonAnalyzer::default();
    a.max_bytes = 8 * 1024;

    let mut s = AnalyzedStream::new(Options::default(), a);
    s.inner_mut()
        .push_boundary_plugin(TagBoundaryPlugin::new("tool_call"));

    let u1 = s.append("<tool_call>\n{\"name\":\"x\",");
    let m1 = u1.pending_meta.expect("pending meta").meta;
    assert!(!m1.closed);
    assert!(!m1.truncated);
    assert!(m1.candidate.as_ref().unwrap().starts_with('{'));

    let u2 = s.append("\"args\":{\"a\":1}}\n</tool_call>\n");
    let m2 = u2
        .committed_meta
        .iter()
        .find(|m| m.meta.closed)
        .expect("committed tool_call meta")
        .meta
        .clone();
    assert!(m2.closed);
    assert!(m2.candidate.as_ref().unwrap().contains("\"args\""));
}

#[test]
fn tool_call_json_analyzer_respects_max_bytes() {
    let mut a = ToolCallJsonAnalyzer::default();
    a.max_bytes = 10;
    let mut s = AnalyzedStream::new(Options::default(), a);
    s.inner_mut()
        .push_boundary_plugin(TagBoundaryPlugin::new("tool_call"));

    let u = s.append("<tool_call>\n{\"name\":\"this-is-long\"}\n</tool_call>\n");
    let m = u.committed_meta.into_iter().next().expect("meta").meta;
    assert!(m.truncated);
    assert!(m.candidate.is_none());
}

#[cfg(feature = "jsonrepair")]
#[test]
fn tool_call_json_analyzer_repairs_with_jsonrepair() {
    let mut s = AnalyzedStream::new(Options::default(), ToolCallJsonAnalyzer::default());
    s.inner_mut()
        .push_boundary_plugin(TagBoundaryPlugin::new("tool_call"));

    let u = s.append("<tool_call>\n{\"a\":1,}\n</tool_call>\n");
    let m = u.committed_meta.into_iter().next().expect("meta").meta;
    assert!(m.repaired.is_some());
    assert_ne!(m.repaired.as_ref().unwrap(), m.candidate.as_ref().unwrap());
}

#[cfg(feature = "serde-json")]
#[test]
fn tool_call_json_analyzer_parses_value_with_serde_json() {
    let mut s = AnalyzedStream::new(Options::default(), ToolCallJsonAnalyzer::default());
    s.inner_mut()
        .push_boundary_plugin(TagBoundaryPlugin::new("tool_call"));
    let u = s.append("<tool_call>\n{\"a\":1}\n</tool_call>\n");
    let m = u.committed_meta.into_iter().next().expect("meta").meta;
    assert!(m.value.is_some());
    assert_eq!(m.value.unwrap()["a"], 1);
}
