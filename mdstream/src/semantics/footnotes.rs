#[derive(Debug, Default)]
pub(crate) struct FootnoteSemantics {
    detected: bool,
    scan_tail: String,
}

impl FootnoteSemantics {
    pub(crate) fn detected(&self) -> bool {
        self.detected
    }

    pub(crate) fn observe_chunk(&mut self, chunk: &str) {
        if self.detected {
            return;
        }
        if detect_footnotes(chunk) {
            self.detected = true;
            return;
        }

        // Keep a small tail window to detect patterns across chunk boundaries.
        const MAX_TAIL: usize = 256;
        let chunk_prefix = take_prefix_at_char_boundary(chunk, MAX_TAIL);
        if !self.scan_tail.is_empty() && !chunk_prefix.is_empty() {
            let mut combined = String::with_capacity(self.scan_tail.len() + chunk_prefix.len());
            combined.push_str(&self.scan_tail);
            combined.push_str(chunk_prefix);
            if detect_footnotes(&combined) {
                self.detected = true;
            }
        }
        if !self.detected {
            update_tail(&mut self.scan_tail, chunk, MAX_TAIL);
        }
    }

    pub(crate) fn reset(&mut self) {
        self.detected = false;
        self.scan_tail.clear();
    }
}

fn detect_footnotes(text: &str) -> bool {
    // Very small, streaming-friendly detector:
    // - references: [^id] (not followed by :)
    // - definitions: [^id]:
    //
    // Compatibility notes:
    // - Align with Streamdown/Incremark: identifiers must not contain whitespace, and must be non-empty.
    // - Keep a conservative identifier length cap to avoid pathological scans.
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'^' {
            const MAX_ID_LEN: usize = 200;
            // Find closing `]` while validating identifier rules.
            let mut j = i + 2;
            let mut id_len = 0usize;
            while j < bytes.len() {
                let b = bytes[j];
                if b == b']' {
                    break;
                }
                if b == b'\n' || b == b'\r' || b == b' ' || b == b'\t' {
                    // Invalid footnote identifier; do not treat as footnote.
                    id_len = 0;
                    break;
                }
                id_len += 1;
                if id_len > MAX_ID_LEN {
                    id_len = 0;
                    break;
                }
                j += 1;
            }
            if id_len > 0 && j < bytes.len() && bytes[j] == b']' {
                // Either a reference (`[^id]`) or a definition (`[^id]:`) should trigger single-block mode.
                return true;
            }
        }
        i += 1;
    }
    false
}

fn take_prefix_at_char_boundary(s: &str, max_bytes: usize) -> &str {
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

fn update_tail(tail: &mut String, chunk: &str, max_bytes: usize) {
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
