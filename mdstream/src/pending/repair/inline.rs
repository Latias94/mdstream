use super::context::{
    is_inside_code_block, is_part_of_triple_backtick, whitespace_or_markers_only,
};

pub(super) fn balance_inline_code(text: &str) -> String {
    // Inline triple backticks (no newlines): ```code``` or ```code``
    if !text.contains('\n') && text.starts_with("```") {
        let bytes = text.as_bytes();
        let mut run = 0usize;
        for &b in bytes.iter().rev() {
            if b == b'`' {
                run += 1;
            } else {
                break;
            }
        }
        if run == 2 || run == 3 {
            let body_end = text.len().saturating_sub(run);
            if body_end >= 3 && !text[3..body_end].contains('`') {
                if run == 2 {
                    let mut out = String::with_capacity(text.len() + 1);
                    out.push_str(text);
                    out.push('`');
                    return out;
                }
                return text.to_string();
            }
        }
    }

    // Inside an incomplete multiline code block? (odd number of ``` substrings)
    let triple_count = text.match_indices("```").count();
    if triple_count % 2 == 1 {
        return text.to_string();
    }

    // Match /(`)([^`]*?)$/ for non-triple backticks
    let bytes = text.as_bytes();
    let mut marker_idx = None;
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'`' && !is_part_of_triple_backtick(text, i) {
            marker_idx = Some(i);
            break;
        }
    }
    let Some(marker_idx) = marker_idx else {
        return text.to_string();
    };
    if is_inside_code_block(text, marker_idx) {
        return text.to_string();
    }
    if text[marker_idx + 1..].contains('`') {
        return text.to_string();
    }
    let content_after = &text[marker_idx + 1..];
    if content_after.is_empty() || whitespace_or_markers_only(content_after) {
        return text.to_string();
    }

    // Count single backticks (excluding triple backticks)
    let mut count = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'`' && !is_part_of_triple_backtick(text, i) {
            count += 1;
        }
    }
    if count % 2 == 1 {
        let mut out = String::with_capacity(text.len() + 1);
        out.push_str(text);
        out.push('`');
        return out;
    }

    text.to_string()
}

pub(super) fn balance_strikethrough(text: &str) -> String {
    // /(~~)([^~]*?)$/
    let Some(marker_idx) = text.rfind("~~") else {
        return text.to_string();
    };
    if text[marker_idx + 2..].contains('~') {
        return text.to_string();
    }
    let content_after = &text[marker_idx + 2..];
    if content_after.is_empty() || whitespace_or_markers_only(content_after) {
        return text.to_string();
    }
    let pairs = text.match_indices("~~").count();
    if pairs % 2 == 1 {
        let mut out = String::with_capacity(text.len() + 2);
        out.push_str(text);
        out.push_str("~~");
        return out;
    }
    text.to_string()
}

pub(super) fn balance_katex_block(text: &str) -> String {
    // Streamdown counts $$ pairs outside inline code (`...`), ignoring triple backticks.
    let bytes = text.as_bytes();
    let mut dollar_pairs = 0usize;
    let mut in_inline_code = false;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'`' && !is_part_of_triple_backtick(text, i) {
            in_inline_code = !in_inline_code;
            i += 1;
            continue;
        }
        if !in_inline_code && bytes[i] == b'$' && bytes[i + 1] == b'$' {
            dollar_pairs += 1;
            i += 2;
            continue;
        }
        i += 1;
    }

    if dollar_pairs % 2 == 0 {
        return text.to_string();
    }

    let first = text.find("$$");
    let has_newline_after_start = first.is_some_and(|idx| text[idx..].contains('\n'));
    if has_newline_after_start && !text.ends_with('\n') {
        let mut out = String::with_capacity(text.len() + 3);
        out.push_str(text);
        out.push('\n');
        out.push_str("$$");
        return out;
    }

    let mut out = String::with_capacity(text.len() + 2);
    out.push_str(text);
    out.push_str("$$");
    out
}
