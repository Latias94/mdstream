use crate::syntax::facts::strip_up_to_three_leading_spaces;

pub(crate) fn normalize_reference_label(label: &str) -> Option<String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() > 200 {
        return None;
    }
    let mut out = String::with_capacity(trimmed.len());
    let mut last_was_ws = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            last_was_ws = true;
            continue;
        }
        if last_was_ws && !out.is_empty() {
            out.push(' ');
        }
        last_was_ws = false;
        for lc in ch.to_lowercase() {
            out.push(lc);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

pub(crate) fn extract_reference_definition_label(line: &str) -> Option<String> {
    // CommonMark-ish reference definition, single line only:
    // up to 3 leading spaces, then "[label]:"
    //
    // We purposely keep this lightweight and streaming-friendly; multi-line definitions
    // can be supported later via a dedicated block mode.
    let s = strip_up_to_three_leading_spaces(line);
    let bytes = s.as_bytes();
    if bytes.len() < 4 || bytes[0] != b'[' {
        return None;
    }
    let close = s.find(']')?;
    if close == 1 {
        return None;
    }
    if s.as_bytes().get(close + 1) != Some(&b':') {
        return None;
    }
    let label = &s[1..close];
    // Exclude footnote definitions like "[^1]:"
    if label.starts_with('^') {
        return None;
    }
    normalize_reference_label(label)
}

#[cfg(feature = "pulldown")]
pub(crate) fn extract_reference_definition_line(line: &str) -> Option<(String, String)> {
    let label = extract_reference_definition_label(line)?;
    Some((label, line.trim_end().to_string()))
}
