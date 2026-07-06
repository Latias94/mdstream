pub(super) fn is_footnote_definition_start(line: &str) -> bool {
    let s = line.trim_start();
    s.starts_with("[^") && s.contains("]:")
}

pub(super) fn is_footnote_continuation(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}
