pub(crate) fn strip_up_to_three_leading_spaces(line: &str) -> &str {
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    s
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
    let trimmed = s.trim_end_matches([' ', '\t']);
    trimmed.chars().all(|c| c == fence_char) && trimmed.chars().count() >= fence_len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_fence_accepts_only_ascii_space_or_tab_after_the_marker() {
        assert!(fence_end("   ``` \t", '`', 3));
        assert!(!fence_end("```\u{a0}", '`', 3));
    }
}
