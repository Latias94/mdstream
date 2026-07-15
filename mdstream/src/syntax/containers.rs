fn is_tag_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':'
}

pub(crate) fn parse_tag_name(input: &str) -> Option<(&str, &str)> {
    let bytes = input.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let mut name_end = 1usize;
    while name_end < bytes.len() && is_tag_name_char(bytes[name_end]) {
        name_end += 1;
    }
    Some((&input[..name_end], &input[name_end..]))
}

pub(crate) fn names_match(actual: &str, expected: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        actual.eq_ignore_ascii_case(expected)
    } else {
        actual == expected
    }
}
