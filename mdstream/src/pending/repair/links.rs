use super::context::is_inside_code_block;

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
