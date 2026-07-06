pub(crate) fn is_space_or_tab(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

pub(crate) fn strip_up_to_three_leading_spaces(line: &str) -> &str {
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    s
}

pub(crate) fn is_blank_line(line: &str) -> bool {
    line.trim().is_empty()
}

pub(crate) fn is_atx_heading_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#') && trimmed[1..].starts_with([' ', '\t', '#'])
}

pub(crate) fn thematic_break_char(line: &str) -> Option<char> {
    // CommonMark-like thematic break:
    // - up to 3 leading spaces
    // - one of '-', '*', '_' repeated >= 3
    // - spaces/tabs may appear between markers
    // - no other characters
    let s = strip_up_to_three_leading_spaces(line);
    let s = s.trim_end_matches([' ', '\t']);
    let mut it = s.chars();
    let first = it.next()?;
    if first != '-' && first != '*' && first != '_' {
        return None;
    }
    let mut count = 1usize;
    for c in it {
        if c == first {
            count += 1;
            continue;
        }
        if c == ' ' || c == '\t' {
            continue;
        }
        return None;
    }
    if count >= 3 { Some(first) } else { None }
}

pub(crate) fn is_thematic_break(line: &str) -> bool {
    thematic_break_char(line).is_some()
}

pub(crate) fn setext_underline_char(line: &str) -> Option<char> {
    // Best-effort setext underline:
    // - up to 3 leading spaces
    // - '=' or '-' repeated >= 2
    // - spaces/tabs may appear between markers
    // - no other characters
    let s = strip_up_to_three_leading_spaces(line);
    let s = s.trim_end_matches([' ', '\t']);
    let mut it = s.chars();
    let first = it.next()?;
    if first != '=' && first != '-' {
        return None;
    }
    let mut count = 1usize;
    for c in it {
        if c == first {
            count += 1;
            continue;
        }
        if c == ' ' || c == '\t' {
            continue;
        }
        return None;
    }
    if count >= 2 { Some(first) } else { None }
}

pub(crate) fn fence_start(line: &str) -> Option<(char, usize)> {
    let s = strip_up_to_three_leading_spaces(line);
    let bytes = s.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let ch = bytes[0] as char;
    if ch != '`' && ch != '~' {
        return None;
    }
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == bytes[0] {
        len += 1;
    }
    if len < 3 {
        return None;
    }
    Some((ch, len))
}

pub(crate) fn fence_end(line: &str, fence_char: char, fence_len: usize) -> bool {
    let s = strip_up_to_three_leading_spaces(line);
    let trimmed = s.trim_end();
    trimmed.chars().all(|c| c == fence_char) && trimmed.chars().count() >= fence_len
}

pub(crate) fn code_fence_suffix(
    raw_ended_with_newline: bool,
    fence_char: char,
    fence_len: usize,
) -> String {
    let mut out = String::new();
    if !raw_ended_with_newline {
        out.push('\n');
    }
    for _ in 0..fence_len {
        out.push(fence_char);
    }
    out.push('\n');
    out
}

pub(crate) fn is_blockquote_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('>')
}

pub(crate) fn is_list_item_start(line: &str) -> bool {
    let s = line.trim_start();
    if s.len() < 2 {
        return false;
    }
    let bytes = s.as_bytes();
    match bytes[0] {
        b'-' | b'+' | b'*' => bytes[1] == b' ' || bytes[1] == b'\t',
        b'0'..=b'9' => {
            let mut i = 0usize;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == 0 || i + 1 >= bytes.len() {
                return false;
            }
            (bytes[i] == b'.' || bytes[i] == b')')
                && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
        }
        _ => false,
    }
}

pub(crate) fn is_list_continuation(line: &str) -> bool {
    // Best-effort continuation line for lists:
    // - indented content (>=2 spaces or a tab)
    // - or a nested list item starter
    if is_list_item_start(line) {
        return true;
    }
    let bytes = line.as_bytes();
    if bytes.first() == Some(&b'\t') {
        return true;
    }
    let mut spaces = 0usize;
    for &b in bytes {
        if b == b' ' {
            spaces += 1;
            if spaces >= 2 {
                return true;
            }
            continue;
        }
        break;
    }
    false
}

pub(crate) fn is_list_item_start_prefix(line: &str) -> bool {
    // Streaming-only heuristic: treat certain "prefix" lines as potential list item starters.
    // This prevents premature block commits when the marker is split across chunks (e.g. "-" then " item").
    let s = line.trim_start();
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    match bytes[0] {
        b'-' | b'+' | b'*' => s.len() == 1,
        b'0'..=b'9' => {
            let mut i = 0usize;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == 0 {
                return false;
            }
            if i == bytes.len() {
                // Digits only: could become "1." / "1)" with more input.
                return true;
            }
            if bytes[i] != b'.' && bytes[i] != b')' {
                return false;
            }
            if i + 1 == bytes.len() {
                // "1." or "1)" without the required whitespace yet.
                return true;
            }
            false
        }
        _ => false,
    }
}

pub(crate) fn count_double_dollars(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'$' {
            if i > 0 && bytes[i - 1] == b'\\' {
                i += 2;
                continue;
            }
            count += 1;
            i += 2;
            continue;
        }
        i += 1;
    }
    count
}

pub(crate) fn tail_window(text: &str, window_bytes: usize) -> (&str, usize) {
    if text.len() <= window_bytes {
        return (text, 0);
    }
    let start = text.len() - window_bytes;
    let mut s = start;
    while !text.is_char_boundary(s) {
        s += 1;
    }
    (&text[s..], s)
}
