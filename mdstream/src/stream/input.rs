use std::borrow::Cow;

#[derive(Debug, Clone)]
pub(super) struct Line {
    start: usize,
    end: usize,        // end excluding '\n'
    has_newline: bool, // true if ended by '\n'
}

impl Line {
    fn new(start: usize, end: usize, has_newline: bool) -> Self {
        Self {
            start,
            end,
            has_newline,
        }
    }

    fn empty() -> Self {
        Self::new(0, 0, false)
    }

    pub(super) fn start(&self) -> usize {
        self.start
    }

    pub(super) fn end(&self) -> usize {
        self.end
    }

    pub(super) fn has_newline(&self) -> bool {
        self.has_newline
    }

    fn as_str<'a>(&self, buffer: &'a str) -> &'a str {
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

#[derive(Debug, Clone)]
pub(super) struct LineBuffer {
    text: String,
    lines: Vec<Line>,
    pending_cr: bool,
}

impl Default for LineBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LineBuffer {
    pub(super) fn new() -> Self {
        Self {
            text: String::new(),
            lines: vec![Line::empty()],
            pending_cr: false,
        }
    }

    pub(super) fn as_str(&self) -> &str {
        &self.text
    }

    pub(super) fn len(&self) -> usize {
        self.text.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub(super) fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub(super) fn has_pending_cr(&self) -> bool {
        self.pending_cr
    }

    pub(super) fn line(&self, line_index: usize) -> Option<&Line> {
        self.lines.get(line_index)
    }

    pub(super) fn line_str(&self, line_index: usize) -> &str {
        self.lines[line_index].as_str(&self.text)
    }

    pub(super) fn line_start(&self, line_index: usize) -> Option<usize> {
        self.line(line_index).map(Line::start)
    }

    pub(super) fn line_end_with_newline(&self, line_index: usize) -> Option<usize> {
        self.line(line_index).map(Line::end_with_newline)
    }

    pub(super) fn line_has_newline(&self, line_index: usize) -> bool {
        self.line(line_index).is_some_and(Line::has_newline)
    }

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

    pub(super) fn append_normalized(&mut self, chunk: &str) {
        let start_offset = self.text.len();
        self.text.push_str(chunk);

        // Ensure we have a "current line" slot.
        if self.lines.is_empty() {
            self.lines.push(Line::empty());
        }

        // Extend the last line end.
        let last_index = self.lines.len() - 1;
        self.lines[last_index].end = self.text.len();

        // Scan for newlines in the appended chunk.
        let bytes = self.text.as_bytes();
        let mut i = start_offset;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                // Finalize current line at i (excluding '\n').
                let last = self.lines.len() - 1;
                self.lines[last].end = i;
                self.lines[last].has_newline = true;

                // Start a new line after '\n'.
                let next_start = i + 1;
                self.lines.push(Line::new(next_start, bytes.len(), false));
            }
            i += 1;
        }
    }

    pub(super) fn flush_pending_cr_at_eof(&mut self) {
        if !self.pending_cr {
            return;
        }
        self.append_normalized("\n");
        self.pending_cr = false;
    }

    pub(super) fn compact_from(&mut self, mut keep_from: usize) -> usize {
        if keep_from == 0 || keep_from > self.text.len() {
            return 0;
        }

        while keep_from < self.text.len() && !self.text.is_char_boundary(keep_from) {
            keep_from += 1;
        }

        if keep_from >= self.text.len() {
            self.text.clear();
        } else {
            self.text = self.text[keep_from..].to_string();
        }

        self.rebuild_lines_from_buffer();
        keep_from
    }

    pub(super) fn reset(&mut self) {
        self.text.clear();
        self.lines.clear();
        self.lines.push(Line::empty());
        self.pending_cr = false;
    }

    fn rebuild_lines_from_buffer(&mut self) {
        self.lines.clear();
        self.lines.push(Line::new(0, self.text.len(), false));
        if self.text.is_empty() {
            return;
        }

        let bytes = self.text.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            if bytes[i] == b'\n' {
                let last = self.lines.len() - 1;
                self.lines[last].end = i;
                self.lines[last].has_newline = true;
                let next_start = i + 1;
                self.lines.push(Line::new(next_start, bytes.len(), false));
            }
            i += 1;
        }
    }
}
