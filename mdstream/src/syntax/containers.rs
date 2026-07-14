use super::facts::strip_up_to_three_leading_spaces;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TagOpening<'a> {
    pub(crate) name: &'a str,
    pub(crate) attributes: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TagClosing<'a> {
    pub(crate) name: &'a str,
    pub(crate) standalone: bool,
    pub(crate) complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FenceContainerLine {
    pub(crate) marker_length: usize,
    pub(crate) standalone_tail: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectiveContainerLine<'a> {
    pub(crate) marker_length: usize,
    pub(crate) name: Option<&'a str>,
    pub(crate) attributes: Option<&'a str>,
}

impl DirectiveContainerLine<'_> {
    pub(crate) fn is_end(self) -> bool {
        self.name.is_none() && self.attributes.is_none()
    }
}

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

pub(crate) fn parse_tag_opening(line: &str) -> Option<TagOpening<'_>> {
    let s = strip_up_to_three_leading_spaces(line).trim_end();
    if !s.starts_with('<') || s.starts_with("</") {
        return None;
    }
    let gt = s.find('>')?;
    let inside = &s[1..gt];
    if inside.starts_with(['/', '!', '?']) {
        return None;
    }

    let (name, rest) = parse_tag_name(inside)?;
    let attributes = rest.trim();
    Some(TagOpening {
        name,
        attributes: (!attributes.is_empty()).then_some(attributes),
    })
}

pub(crate) fn parse_tag_closing(line: &str) -> Option<TagClosing<'_>> {
    let s = strip_up_to_three_leading_spaces(line).trim_end();
    if !s.starts_with("</") {
        return None;
    }
    let after_open = &s[2..];
    let (name, rest) = parse_tag_name(after_open)?;
    let Some(gt_offset) = rest.find('>') else {
        return Some(TagClosing {
            name,
            standalone: false,
            complete: false,
        });
    };
    let before_gt = &rest[..gt_offset];
    let after_gt = &rest[gt_offset + 1..];
    let standalone = before_gt.trim().is_empty() && after_gt.trim().is_empty();
    Some(TagClosing {
        name,
        standalone,
        complete: true,
    })
}

pub(crate) fn parse_fence_container_line(line: &str, fence_char: char) -> FenceContainerLine {
    let s = strip_up_to_three_leading_spaces(line);
    let s = s.trim_end_matches([' ', '\t']);
    let bytes = s.as_bytes();
    let ch = fence_char as u8;
    let mut len = 0usize;
    while len < bytes.len() && bytes[len] == ch {
        len += 1;
    }
    FenceContainerLine {
        marker_length: len,
        standalone_tail: s[len..].trim().is_empty(),
    }
}

fn is_container_name_start(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_container_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

pub(crate) fn parse_directive_container_line(
    line: &str,
    marker: char,
    min_marker_length: usize,
) -> Option<DirectiveContainerLine<'_>> {
    let s = line.trim_end().trim_start();
    let bytes = s.as_bytes();
    let marker = marker as u8;
    let mut i = 0usize;
    while i < bytes.len() && bytes[i] == marker {
        i += 1;
    }
    if i < min_marker_length {
        return None;
    }

    let marker_length = i;
    let mut rest = s[i..].trim_end_matches([' ', '\t']);
    if rest.is_empty() {
        return Some(DirectiveContainerLine {
            marker_length,
            name: None,
            attributes: None,
        });
    }

    // Incremark containers require whitespace between markers and name/attrs.
    if !rest
        .as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_whitespace())
    {
        return None;
    }
    rest = rest.trim_start_matches([' ', '\t']);

    let rest_bytes = rest.as_bytes();
    let mut name_end = 0usize;
    if rest_bytes
        .first()
        .is_some_and(|b| is_container_name_start(*b))
    {
        name_end = 1;
        while name_end < rest_bytes.len() && is_container_name_char(rest_bytes[name_end]) {
            name_end += 1;
        }
    }

    let name = (name_end > 0).then_some(&rest[..name_end]);
    let attributes = rest[name_end..].trim();
    let attributes = (!attributes.is_empty()).then_some(attributes);

    Some(DirectiveContainerLine {
        marker_length,
        name,
        attributes,
    })
}

pub(crate) fn names_match(actual: &str, expected: &str, case_insensitive: bool) -> bool {
    if case_insensitive {
        actual.eq_ignore_ascii_case(expected)
    } else {
        actual == expected
    }
}

pub(crate) fn normalize_name(name: &str, case_insensitive: bool) -> String {
    if case_insensitive {
        name.to_ascii_lowercase()
    } else {
        name.to_string()
    }
}
