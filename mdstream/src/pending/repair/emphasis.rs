use crate::syntax::facts::is_space_or_tab;

use super::context::{
    is_inside_code_block, is_within_link_or_image_url, is_within_math_block, is_word_char,
    whitespace_or_markers_only,
};

pub(super) fn is_list_marker_at(text: &str, byte_index: usize) -> bool {
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

pub(super) fn is_horizontal_rule_line(text: &str, marker_index: usize, marker: u8) -> bool {
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

pub(super) fn count_triple_asterisks(text: &str) -> usize {
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

pub(super) fn should_skip_asterisk(text: &str, index: usize) -> bool {
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

pub(super) fn count_single_asterisks(text: &str) -> usize {
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

pub(super) fn should_skip_underscore(text: &str, index: usize) -> bool {
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

pub(super) fn count_single_underscores(text: &str) -> usize {
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

pub(super) fn handle_incomplete_bold(text: &str) -> String {
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

pub(super) fn handle_incomplete_double_underscore_italic(text: &str) -> String {
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

pub(super) fn handle_incomplete_single_asterisk_italic(text: &str) -> String {
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

pub(super) fn is_line_prefix_list_marker(text: &str, marker_index: usize) -> bool {
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

pub(super) fn insert_closing_underscore(text: &str) -> String {
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

pub(super) fn find_first_single_underscore_index(text: &str) -> Option<usize> {
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

pub(super) fn handle_trailing_asterisks_for_underscore(text: &str) -> Option<String> {
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

pub(super) fn handle_incomplete_single_underscore_italic(text: &str) -> String {
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

pub(super) fn bold_italic_markers_balanced(text: &str) -> bool {
    let pairs = text.match_indices("**").count();
    let single = count_single_asterisks(text);
    pairs % 2 == 0 && single % 2 == 0
}

pub(super) fn handle_incomplete_bold_italic(text: &str) -> String {
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
