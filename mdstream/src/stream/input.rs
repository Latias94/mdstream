use std::borrow::Cow;

use super::MdStream;

#[derive(Debug, Clone)]
pub(super) struct Line {
    pub(super) start: usize,
    pub(super) end: usize,        // end excluding '\n'
    pub(super) has_newline: bool, // true if ended by '\n'
}

impl Line {
    pub(super) fn as_str<'a>(&self, buffer: &'a str) -> &'a str {
        &buffer[self.start..self.end]
    }

    pub(super) fn end_with_newline(&self) -> usize {
        if self.has_newline {
            self.end + 1
        } else {
            self.end
        }
    }
}

pub(super) fn take_prefix_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn take_suffix_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

pub(super) fn update_tail(tail: &mut String, chunk: &str, max_bytes: usize) {
    if chunk.is_empty() {
        return;
    }
    if chunk.len() >= max_bytes {
        *tail = take_suffix_at_char_boundary(chunk, max_bytes).to_string();
        return;
    }
    if tail.len() + chunk.len() <= max_bytes {
        tail.push_str(chunk);
        return;
    }
    let mut combined = String::with_capacity(max_bytes + 4);
    combined.push_str(tail);
    combined.push_str(chunk);
    *tail = take_suffix_at_char_boundary(&combined, max_bytes).to_string();
}

impl MdStream {
    pub(super) fn normalize_newlines_cow<'a>(&mut self, chunk: &'a str) -> Cow<'a, str> {
        if !chunk.contains('\r') && !self.pending_cr {
            return Cow::Borrowed(chunk);
        }

        let mut out = String::with_capacity(chunk.len() + 1);
        let mut chars = chunk.chars().peekable();

        if self.pending_cr {
            // Previous chunk ended with '\r' (possibly CRLF across boundary).
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
            self.pending_cr = false;
        }

        while let Some(c) = chars.next() {
            if c != '\r' {
                out.push(c);
                continue;
            }
            if chars.peek() == Some(&'\n') {
                chars.next();
                out.push('\n');
                continue;
            }
            if chars.peek().is_none() {
                // Defer decision: this may be a CRLF pair split across chunks.
                self.pending_cr = true;
                continue;
            }
            out.push('\n');
        }

        Cow::Owned(out)
    }

    pub(super) fn append_to_lines(&mut self, chunk: &str) {
        let start_offset = self.buffer.len();
        self.buffer.push_str(chunk);

        // Ensure we have a "current line" slot.
        if self.lines.is_empty() {
            self.lines.push(Line {
                start: 0,
                end: 0,
                has_newline: false,
            });
        }

        // Extend the last line end.
        let last_index = self.lines.len() - 1;
        self.lines[last_index].end = self.buffer.len();

        // Scan for newlines in the appended chunk.
        let bytes = self.buffer.as_bytes();
        let mut i = start_offset;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                // Finalize current line at i (excluding '\n').
                let last = self.lines.len() - 1;
                self.lines[last].end = i;
                self.lines[last].has_newline = true;

                // Start a new line after '\n'.
                let next_start = i + 1;
                self.lines.push(Line {
                    start: next_start,
                    end: bytes.len(),
                    has_newline: false,
                });
            }
            i += 1;
        }
    }

    pub(super) fn rebuild_lines_from_buffer(&mut self) {
        self.lines.clear();
        self.lines.push(Line {
            start: 0,
            end: self.buffer.len(),
            has_newline: false,
        });
        if self.buffer.is_empty() {
            return;
        }

        let bytes = self.buffer.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                let last = self.lines.len() - 1;
                self.lines[last].end = i;
                self.lines[last].has_newline = true;
                let next_start = i + 1;
                self.lines.push(Line {
                    start: next_start,
                    end: bytes.len(),
                    has_newline: false,
                });
            }
            i += 1;
        }
    }
}
