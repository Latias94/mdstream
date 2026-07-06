pub(super) fn is_inside_incomplete_multiline_code_block(text: &str) -> bool {
    // Streamdown/remend behavior: treat odd occurrences of "```" as an incomplete multiline code block,
    // but only in the multiline context (must contain a newline).
    text.contains('\n') && text.match_indices("```").count() % 2 == 1
}

pub(super) fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c.is_alphanumeric()
}

pub(super) fn whitespace_or_markers_only(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_whitespace() || matches!(c, '_' | '~' | '*' | '`'))
}

pub(super) fn is_part_of_triple_backtick(text: &str, i: usize) -> bool {
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

pub(super) fn is_inside_code_block(text: &str, position: usize) -> bool {
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

pub(super) fn is_within_math_block(text: &str, position: usize) -> bool {
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

pub(super) fn is_within_link_or_image_url(text: &str, position: usize) -> bool {
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
