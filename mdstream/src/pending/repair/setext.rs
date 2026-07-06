pub(super) fn trim_trailing_single_space(text: &str) -> &str {
    if text.ends_with(' ') && !text.ends_with("  ") {
        &text[..text.len() - 1]
    } else {
        text
    }
}

pub(super) fn apply_setext_heading_protection(text: &str) -> String {
    let trimmed = trim_trailing_single_space(text);
    let Some(last_nl) = trimmed.rfind('\n') else {
        return trimmed.to_string();
    };

    let prev = &trimmed[..last_nl];
    if prev.is_empty() {
        return trimmed.to_string();
    }
    if prev.ends_with('\n') {
        // Previous line empty.
        return trimmed.to_string();
    }

    // Streamdown/remend behavior:
    // - Match the last line after trimming BOTH ends (so leading whitespace is allowed).
    // - Only protect 1-2 dashes/equals.
    // - If the marker already has trailing whitespace, skip (it's already broken).
    let last_line = &trimmed[last_nl + 1..];
    let trimmed_last_line = last_line.trim();

    let is_ambiguous_dashes = trimmed_last_line == "-" || trimmed_last_line == "--";
    let is_ambiguous_equals = trimmed_last_line == "=" || trimmed_last_line == "==";

    let has_trailing_ws_after_marker = last_line.ends_with(' ') || last_line.ends_with('\t');

    if (is_ambiguous_dashes || is_ambiguous_equals) && !has_trailing_ws_after_marker {
        // Check if the previous line has content (required for setext headings).
        let prev_line = prev.rsplit('\n').next().unwrap_or("");
        if !prev_line.trim().is_empty() {
            let mut out = String::with_capacity(trimmed.len() + 3);
            out.push_str(trimmed);
            out.push('\u{200B}');
            return out;
        }
    }

    trimmed.to_string()
}
