#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeFenceHeader<'a> {
    pub fence_char: char,
    pub fence_len: usize,
    /// Entire info string (trimmed), excluding fence markers.
    pub info: &'a str,
    /// First token of `info`, lowercased if ASCII. Empty means "no language".
    pub language: Option<&'a str>,
}

fn is_space_or_tab(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

pub fn parse_code_fence_header(line: &str) -> Option<CodeFenceHeader<'_>> {
    // CommonMark-ish fence opening line:
    // - up to 3 leading spaces
    // - fence is ``` or ~~~ (>=3)
    // - info string is the rest of the line after the fence run
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }

    let bytes = s.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let fence_char = bytes[0] as char;
    if fence_char != '`' && fence_char != '~' {
        return None;
    }
    let mut fence_len = 0usize;
    while fence_len < bytes.len() && bytes[fence_len] == bytes[0] {
        fence_len += 1;
    }
    if fence_len < 3 {
        return None;
    }

    let info = s[fence_len..].trim();
    let language = info
        .split_whitespace()
        .next()
        .and_then(|tok| if tok.is_empty() { None } else { Some(tok) });

    Some(CodeFenceHeader {
        fence_char,
        fence_len,
        info,
        language,
    })
}

pub fn parse_code_fence_header_from_block(text: &str) -> Option<CodeFenceHeader<'_>> {
    let first_line = text.split('\n').next().unwrap_or(text);
    parse_code_fence_header(first_line)
}

pub fn is_code_fence_closing_line(line: &str, fence_char: char, fence_len: usize) -> bool {
    // Mirrors `src/stream.rs` fence_end behavior, but exported for consumers.
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let trimmed = s.trim_end();
    let mut count = 0usize;
    for ch in trimmed.chars() {
        if ch != fence_char {
            return false;
        }
        count += 1;
    }
    count >= fence_len
}

pub fn is_list_marker_line_prefix(line: &str) -> bool {
    // Equivalent to remend listItemPattern: /^[\s]*[-*+][\s]+$/
    // This is exposed for adapters that want to replicate remend-like heuristics.
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && is_space_or_tab(bytes[i]) {
        i += 1;
    }
    if i >= bytes.len() {
        return false;
    }
    let marker = bytes[i];
    if marker != b'-' && marker != b'*' && marker != b'+' {
        return false;
    }
    i += 1;
    if i >= bytes.len() {
        return false;
    }
    let mut has_ws = false;
    while i < bytes.len() {
        if is_space_or_tab(bytes[i]) {
            has_ws = true;
            i += 1;
            continue;
        }
        return false;
    }
    has_ws
}

