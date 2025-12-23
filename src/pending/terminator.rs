#[derive(Debug, Clone)]
pub struct TerminatorOptions {
    pub setext_headings: bool,
    pub links: bool,
    pub images: bool,
    pub emphasis: bool,
    pub inline_code: bool,
    pub strikethrough: bool,
    pub katex_block: bool,
    pub incomplete_link_url: String,
    /// Tail-only scan window for termination logic.
    pub window_bytes: usize,
}

impl Default for TerminatorOptions {
    fn default() -> Self {
        Self {
            setext_headings: true,
            links: true,
            images: true,
            emphasis: true,
            inline_code: true,
            strikethrough: true,
            katex_block: true,
            incomplete_link_url: "streamdown:incomplete-link".to_string(),
            window_bytes: 16 * 1024,
        }
    }
}

fn is_space_or_tab(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn is_inside_incomplete_multiline_code_block(text: &str) -> bool {
    // Streamdown/remend behavior: treat odd occurrences of "```" as an incomplete multiline code block,
    // but only in the multiline context (must contain a newline).
    text.contains('\n') && text.match_indices("```").count() % 2 == 1
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c.is_alphanumeric()
}

fn whitespace_or_markers_only(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_whitespace() || matches!(c, '_' | '~' | '*' | '`'))
}

fn is_part_of_triple_backtick(text: &str, i: usize) -> bool {
    let bytes = text.as_bytes();
    if i + 2 < bytes.len() && &bytes[i..i + 3] == b"```" {
        return true;
    }
    if i >= 1 && i + 1 < bytes.len() && &bytes[i - 1..i + 2] == b"```" {
        return true;
    }
    if i >= 2 && &bytes[i - 2..i + 1] == b"```" {
        return true;
    }
    false
}

fn is_inside_code_block(text: &str, position: usize) -> bool {
    let bytes = text.as_bytes();
    let mut in_inline = false;
    let mut in_multiline = false;

    let mut i = 0usize;
    while i < position && i < bytes.len() {
        if i + 2 < bytes.len() && &bytes[i..i + 3] == b"```" {
            in_multiline = !in_multiline;
            i += 3;
            continue;
        }
        if !in_multiline && bytes[i] == b'`' {
            in_inline = !in_inline;
        }
        i += 1;
    }

    in_inline || in_multiline
}

fn tail_window(text: &str, window_bytes: usize) -> (&str, usize) {
    if text.len() <= window_bytes {
        return (text, 0);
    }
    let start = text.len() - window_bytes;
    // Move to char boundary.
    let mut s = start;
    while !text.is_char_boundary(s) {
        s += 1;
    }
    (&text[s..], s)
}

fn is_within_math_block(text: &str, position: usize) -> bool {
    // Toggle on $ and $$, skipping escaped \$.
    let bytes = text.as_bytes();
    let mut in_inline = false;
    let mut in_block = false;
    let mut i = 0usize;
    while i < position && i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            i += 2;
            continue;
        }
        if bytes[i] == b'$' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'$' {
                in_block = !in_block;
                in_inline = false;
                i += 2;
                continue;
            }
            if !in_block {
                in_inline = !in_inline;
            }
        }
        i += 1;
    }
    in_inline || in_block
}

fn is_within_link_or_image_url(text: &str, position: usize) -> bool {
    // Simple heuristic: scan backwards on the same line, find "(", ensure immediately preceded by "]".
    let bytes = text.as_bytes();
    let mut i = position;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'\n' => return false,
            b')' => return false,
            b'(' => {
                if i > 0 && bytes[i - 1] == b']' {
                    // Ensure there's a ')' after position before newline.
                    let mut j = position;
                    while j < bytes.len() {
                        if bytes[j] == b')' {
                            return true;
                        }
                        if bytes[j] == b'\n' {
                            return false;
                        }
                        j += 1;
                    }
                }
                return false;
            }
            _ => {}
        }
    }
    false
}

fn trim_trailing_single_space(text: &str) -> &str {
    if text.ends_with(' ') && !text.ends_with("  ") {
        &text[..text.len() - 1]
    } else {
        text
    }
}

fn apply_setext_heading_protection(text: &str) -> String {
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

fn find_matching_open_bracket(text: &str, close_index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut i = close_index;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b']' => depth += 1,
            b'[' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_matching_close_bracket(text: &str, open_index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut i = open_index + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

pub(crate) fn fix_incomplete_link_or_image(
    text: &str,
    incomplete_url: &str,
    links_enabled: bool,
    images_enabled: bool,
) -> Option<String> {
    // 1) incomplete URL: scan for the last eligible occurrence of "](" with no ")" after it.
    //
    // We cannot just take `rfind("](")` because callers may want to process only links or only images.
    let mut search = text.len();
    while let Some(idx) = text[..search].rfind("](") {
        search = idx;
        if is_inside_code_block(text, idx) {
            continue;
        }
        let after = &text[idx + 2..];
        if after.contains(')') {
            continue;
        }
        let Some(open_bracket) = find_matching_open_bracket(text, idx) else {
            continue;
        };
        if is_inside_code_block(text, open_bracket) {
            continue;
        }
        let is_image = open_bracket > 0 && text.as_bytes()[open_bracket - 1] == b'!';
        if is_image && !images_enabled {
            continue;
        }
        if !is_image && !links_enabled {
            continue;
        }
        let start = if is_image {
            open_bracket - 1
        } else {
            open_bracket
        };
        let before = &text[..start];
        if is_image {
            return Some(before.to_string());
        }
        let link_text = &text[open_bracket + 1..idx];
        return Some(format!("{before}[{link_text}]({incomplete_url})"));
    }

    // 2) incomplete link text: search backwards for '[' without a matching closing ']'
    let bytes = text.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'[' && !is_inside_code_block(text, i) {
            let is_image = i > 0 && bytes[i - 1] == b'!';
            let open_index = if is_image { i - 1 } else { i };
            if is_image && !images_enabled {
                continue;
            }
            if !is_image && !links_enabled {
                continue;
            }

            let after_open = &text[i + 1..];
            if !after_open.contains(']') {
                if is_image {
                    return Some(text[..open_index].to_string());
                }
                return Some(format!("{text}]({incomplete_url})"));
            }

            if find_matching_close_bracket(text, i).is_none() {
                if is_image {
                    return Some(text[..open_index].to_string());
                }
                return Some(format!("{text}]({incomplete_url})"));
            }
        }
    }

    None
}

fn is_list_marker_at(text: &str, byte_index: usize) -> bool {
    // Detect common list marker patterns at start of line:
    // ^\s{0,3}[*+-]\s+  or ^\s{0,3}\d+[.)]\s+
    let bytes = text.as_bytes();
    let mut i = byte_index;
    while i > 0 && bytes[i - 1] != b'\n' {
        i -= 1;
    }
    let line_start = i;
    let mut j = line_start;
    let mut spaces = 0;
    while j < bytes.len() && spaces < 3 && bytes[j] == b' ' {
        spaces += 1;
        j += 1;
    }
    if j >= bytes.len() {
        return false;
    }
    if j == byte_index && (bytes[j] == b'*' || bytes[j] == b'+' || bytes[j] == b'-') {
        return bytes.get(j + 1).is_some_and(|b| is_space_or_tab(*b));
    }
    if j <= byte_index && byte_index < bytes.len() && bytes[byte_index].is_ascii_digit() {
        // ordered list marker like "1." or "1)"
        let mut k = j;
        while k < bytes.len() && bytes[k].is_ascii_digit() {
            k += 1;
        }
        if k > j && k == byte_index && matches!(bytes.get(k), Some(b'.' | b')')) {
            return bytes.get(k + 1).is_some_and(|b| is_space_or_tab(*b));
        }
    }
    false
}

fn is_horizontal_rule_line(text: &str, marker_index: usize, marker: u8) -> bool {
    // Marker must be on its own line with >=3 markers and no other non-whitespace chars.
    let bytes = text.as_bytes();
    let mut line_start = marker_index;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let mut line_end = marker_index;
    while line_end < bytes.len() && bytes[line_end] != b'\n' {
        line_end += 1;
    }
    let line = &bytes[line_start..line_end];
    let mut marker_count = 0usize;
    for &b in line {
        if b == marker {
            marker_count += 1;
        } else if b != b' ' && b != b'\t' {
            return false;
        }
    }
    marker_count >= 3
}

fn count_triple_asterisks(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0usize;
    let mut consecutive = 0usize;
    for &b in bytes {
        if b == b'*' {
            consecutive += 1;
        } else {
            if consecutive >= 3 {
                count += consecutive / 3;
            }
            consecutive = 0;
        }
    }
    if consecutive >= 3 {
        count += consecutive / 3;
    }
    count
}

fn should_skip_asterisk(text: &str, index: usize) -> bool {
    let bytes = text.as_bytes();
    let prev = if index > 0 { bytes[index - 1] } else { 0 };
    let next = if index + 1 < bytes.len() {
        bytes[index + 1]
    } else {
        0
    };

    if prev == b'\\' {
        return true;
    }

    if is_inside_code_block(text, index) {
        return true;
    }

    if text.contains('$') && is_within_math_block(text, index) {
        return true;
    }

    // Special handling for *** sequences:
    // - first '*' in '***' is counted as a single asterisk
    // - first '*' in '**' is skipped
    if prev != b'*' && next == b'*' {
        let next_next = if index + 2 < bytes.len() {
            bytes[index + 2]
        } else {
            0
        };
        if next_next == b'*' {
            return false;
        }
        return true;
    }

    // second or later '*' in a run
    if prev == b'*' {
        return true;
    }

    // word-internal
    if prev != 0 && next != 0 {
        let prev_c = prev as char;
        let next_c = next as char;
        if is_word_char(prev_c) && is_word_char(next_c) {
            return true;
        }
    }

    if is_list_marker_at(text, index) {
        return true;
    }

    false
}

fn count_single_asterisks(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b != b'*' {
            continue;
        }
        if !should_skip_asterisk(text, i) {
            count += 1;
        }
    }
    count
}

fn should_skip_underscore(text: &str, index: usize) -> bool {
    let bytes = text.as_bytes();
    let prev = if index > 0 { bytes[index - 1] } else { 0 };
    let next = if index + 1 < bytes.len() {
        bytes[index + 1]
    } else {
        0
    };

    if prev == b'\\' {
        return true;
    }
    if is_inside_code_block(text, index) {
        return true;
    }
    if text.contains('$') && is_within_math_block(text, index) {
        return true;
    }
    if is_within_link_or_image_url(text, index) {
        return true;
    }
    if prev == b'_' || next == b'_' {
        return true;
    }
    if prev != 0 && next != 0 && is_word_char(prev as char) && is_word_char(next as char) {
        return true;
    }
    false
}

fn count_single_underscores(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b != b'_' {
            continue;
        }
        if !should_skip_underscore(text, i) {
            count += 1;
        }
    }
    count
}

fn handle_incomplete_bold(text: &str) -> String {
    // boldPattern: /(\*\*)([^*]*?)$/
    let Some(marker_idx) = text.rfind("**") else {
        return text.to_string();
    };
    if text[marker_idx + 2..].contains('*') {
        return text.to_string();
    }
    if is_inside_code_block(text, marker_idx) {
        return text.to_string();
    }
    let content_after = &text[marker_idx + 2..];
    if content_after.is_empty() || whitespace_or_markers_only(content_after) {
        return text.to_string();
    }
    if is_horizontal_rule_line(text, marker_idx, b'*') {
        return text.to_string();
    }

    // Streamdown/remend: if a bold marker appears right after a list marker
    // and spans multiple lines, skip auto-closing (avoid cross-line list artifacts).
    if content_after.contains('\n') && is_line_prefix_list_marker(text, marker_idx) {
        return text.to_string();
    }

    let pairs = text.match_indices("**").count();
    if pairs % 2 == 1 {
        let mut out = String::with_capacity(text.len() + 2);
        out.push_str(text);
        out.push_str("**");
        return out;
    }
    text.to_string()
}

fn handle_incomplete_double_underscore_italic(text: &str) -> String {
    // italicPattern: /(__)([^_]*?)$/
    let Some(marker_idx) = text.rfind("__") else {
        return text.to_string();
    };
    if text[marker_idx + 2..].contains('_') {
        return text.to_string();
    }
    if is_inside_code_block(text, marker_idx) {
        return text.to_string();
    }
    let content_after = &text[marker_idx + 2..];
    if content_after.is_empty() || whitespace_or_markers_only(content_after) {
        return text.to_string();
    }
    if is_horizontal_rule_line(text, marker_idx, b'_') {
        return text.to_string();
    }

    // Streamdown/remend: if a __ marker appears right after a list marker and spans multiple
    // lines, skip auto-closing.
    if content_after.contains('\n') && is_line_prefix_list_marker(text, marker_idx) {
        return text.to_string();
    }

    let pairs = text.match_indices("__").count();
    if pairs % 2 == 1 {
        let mut out = String::with_capacity(text.len() + 2);
        out.push_str(text);
        out.push_str("__");
        return out;
    }
    text.to_string()
}

fn handle_incomplete_single_asterisk_italic(text: &str) -> String {
    // Find first single asterisk (not part of **), not escaped, not within math, not word-internal.
    let bytes = text.as_bytes();
    let mut first = None;
    for i in 0..bytes.len() {
        if bytes[i] != b'*' {
            continue;
        }
        if is_inside_code_block(text, i) {
            continue;
        }
        let prev = if i > 0 { bytes[i - 1] } else { 0 };
        let next = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        if prev == b'*' || next == b'*' || prev == b'\\' {
            continue;
        }
        if text.contains('$') && is_within_math_block(text, i) {
            continue;
        }
        if prev != 0 && next != 0 && is_word_char(prev as char) && is_word_char(next as char) {
            continue;
        }
        if is_list_marker_at(text, i) {
            continue;
        }
        first = Some(i);
        break;
    }
    let Some(first_idx) = first else {
        return text.to_string();
    };
    if is_inside_code_block(text, first_idx) {
        return text.to_string();
    }
    let content_after = &text[first_idx + 1..];
    if content_after.is_empty() || whitespace_or_markers_only(content_after) {
        return text.to_string();
    }
    let single = count_single_asterisks(text);
    if single % 2 == 1 {
        let mut out = String::with_capacity(text.len() + 1);
        out.push_str(text);
        out.push('*');
        return out;
    }
    text.to_string()
}

fn is_line_prefix_list_marker(text: &str, marker_index: usize) -> bool {
    // Match remend listItemPattern: /^[\s]*[-*+][\s]+$/
    // We apply it to the substring before the marker on the same line.
    let bytes = text.as_bytes();
    let mut line_start = marker_index;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let prefix = &text[line_start..marker_index];
    let mut i = 0usize;
    let pbytes = prefix.as_bytes();
    while i < pbytes.len() && (pbytes[i] == b' ' || pbytes[i] == b'\t') {
        i += 1;
    }
    if i >= pbytes.len() {
        return false;
    }
    let marker = pbytes[i];
    if marker != b'-' && marker != b'*' && marker != b'+' {
        return false;
    }
    i += 1;
    if i >= pbytes.len() {
        return false;
    }
    let mut has_ws = false;
    while i < pbytes.len() {
        if pbytes[i] == b' ' || pbytes[i] == b'\t' {
            has_ws = true;
            i += 1;
            continue;
        }
        return false;
    }
    has_ws
}

fn insert_closing_underscore(text: &str) -> String {
    // Insert '_' before trailing newlines.
    let mut end = text.len();
    while end > 0 && text.as_bytes()[end - 1] == b'\n' {
        end -= 1;
    }
    if end < text.len() {
        let mut out = String::with_capacity(text.len() + 1);
        out.push_str(&text[..end]);
        out.push('_');
        out.push_str(&text[end..]);
        out
    } else {
        let mut out = String::with_capacity(text.len() + 1);
        out.push_str(text);
        out.push('_');
        out
    }
}

fn find_first_single_underscore_index(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] != b'_' {
            continue;
        }
        if is_inside_code_block(text, i) {
            continue;
        }
        let prev = if i > 0 { bytes[i - 1] } else { 0 };
        let next = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        if prev == b'_' || next == b'_' || prev == b'\\' {
            continue;
        }
        if text.contains('$') && is_within_math_block(text, i) {
            continue;
        }
        if is_within_link_or_image_url(text, i) {
            continue;
        }
        if prev != 0 && next != 0 && is_word_char(prev as char) && is_word_char(next as char) {
            continue;
        }
        return Some(i);
    }
    None
}

fn handle_trailing_asterisks_for_underscore(text: &str) -> Option<String> {
    if !text.ends_with("**") {
        return None;
    }
    let without = &text[..text.len() - 2];
    let pairs = without.match_indices("**").count();
    if pairs % 2 != 1 {
        return None;
    }
    let first_double = without.find("**")?;
    let underscore_idx = find_first_single_underscore_index(without)?;
    if first_double < underscore_idx {
        return Some(format!("{without}_**"));
    }
    None
}

fn handle_incomplete_single_underscore_italic(text: &str) -> String {
    let Some(first_idx) = find_first_single_underscore_index(text) else {
        return text.to_string();
    };
    let content_after = &text[first_idx + 1..];
    if content_after.is_empty() || whitespace_or_markers_only(content_after) {
        return text.to_string();
    }
    let single = count_single_underscores(text);
    if single % 2 == 1 {
        if let Some(nested) = handle_trailing_asterisks_for_underscore(text) {
            return nested;
        }
        return insert_closing_underscore(text);
    }
    text.to_string()
}

fn bold_italic_markers_balanced(text: &str) -> bool {
    let pairs = text.match_indices("**").count();
    let single = count_single_asterisks(text);
    pairs % 2 == 0 && single % 2 == 0
}

fn handle_incomplete_bold_italic(text: &str) -> String {
    // Don't process if text is only asterisks and has 4+.
    let t = text.trim();
    if !t.is_empty() && t.chars().all(|c| c == '*') && t.len() >= 4 {
        return text.to_string();
    }

    let Some(marker_idx) = text.rfind("***") else {
        return text.to_string();
    };
    if text[marker_idx + 3..].contains('*') {
        return text.to_string();
    }
    let content_after = &text[marker_idx + 3..];
    if content_after.is_empty() || whitespace_or_markers_only(content_after) {
        return text.to_string();
    }
    if is_inside_code_block(text, marker_idx) {
        return text.to_string();
    }
    if is_horizontal_rule_line(text, marker_idx, b'*') {
        return text.to_string();
    }

    let triple = count_triple_asterisks(text);
    if triple % 2 == 1 {
        if bold_italic_markers_balanced(text) {
            return text.to_string();
        }
        let mut out = String::with_capacity(text.len() + 3);
        out.push_str(text);
        out.push_str("***");
        return out;
    }
    text.to_string()
}

fn balance_inline_code(text: &str) -> String {
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

fn balance_strikethrough(text: &str) -> String {
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

fn balance_katex_block(text: &str) -> String {
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

/// Terminate a streaming Markdown tail to avoid partial rendering artifacts.
///
/// This function is intentionally conservative and only modifies the pending tail.
pub fn terminate_markdown(text: &str, opts: &TerminatorOptions) -> String {
    if text.is_empty() {
        return String::new();
    }

    let text = trim_trailing_single_space(text);
    let (window, offset) = tail_window(text, opts.window_bytes);

    // Work on the tail window but keep a stable prefix.
    let prefix = &text[..offset];
    let mut tail = window.to_string();

    if opts.setext_headings {
        tail = apply_setext_heading_protection(&tail);
    }

    if is_inside_incomplete_multiline_code_block(&tail) {
        // If the tail is currently inside an unclosed fenced code block, avoid other termination.
        let mut out = String::with_capacity(prefix.len() + tail.len());
        out.push_str(prefix);
        out.push_str(&tail);
        return out;
    }

    if opts.links || opts.images {
        if let Some(processed) =
            fix_incomplete_link_or_image(&tail, &opts.incomplete_link_url, opts.links, opts.images)
        {
            if processed.ends_with(&format!("]({})", opts.incomplete_link_url)) {
                let mut out = String::with_capacity(prefix.len() + processed.len());
                out.push_str(prefix);
                out.push_str(&processed);
                return out;
            }
            tail = processed;
        }
    }

    if opts.emphasis {
        tail = handle_incomplete_bold_italic(&tail);
        tail = handle_incomplete_bold(&tail);
        tail = handle_incomplete_double_underscore_italic(&tail);
        tail = handle_incomplete_single_asterisk_italic(&tail);
        tail = handle_incomplete_single_underscore_italic(&tail);
    }
    if opts.inline_code {
        tail = balance_inline_code(&tail);
    }
    if opts.strikethrough {
        tail = balance_strikethrough(&tail);
    }
    if opts.katex_block {
        tail = balance_katex_block(&tail);
    }

    let mut out = String::with_capacity(prefix.len() + tail.len());
    out.push_str(prefix);
    out.push_str(&tail);
    out
}
