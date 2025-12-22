use crate::options::{FootnotesMode, Options};
use crate::pending::terminate_markdown;
use crate::types::{Block, BlockId, BlockKind, BlockStatus, Update};

#[derive(Debug, Clone)]
struct Line {
    start: usize,
    end: usize,        // end excluding '\n'
    has_newline: bool, // true if ended by '\n'
}

impl Line {
    fn as_str<'a>(&self, buffer: &'a str) -> &'a str {
        &buffer[self.start..self.end]
    }
    fn end_with_newline(&self) -> usize {
        if self.has_newline {
            self.end + 1
        } else {
            self.end
        }
    }
}

#[derive(Debug, Clone)]
enum BlockMode {
    Unknown,
    Paragraph,
    Heading,
    ThematicBreak,
    CodeFence {
        fence_char: char,
        fence_len: usize,
        info: Option<String>,
    },
    List,
    BlockQuote,
    HtmlBlock,
    Table,
    MathBlock { open_count: usize },
    FootnoteDefinition,
}

fn is_empty_line(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#') && trimmed[1..].starts_with(|c: char| c == ' ' || c == '\t' || c == '#')
}

fn is_thematic_break(line: &str) -> bool {
    let s = line.trim();
    if s.len() < 3 {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if first != '-' && first != '*' && first != '_' {
        return false;
    }
    chars.all(|c| c == first)
}

fn fence_start(line: &str) -> Option<(char, usize)> {
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

fn fence_end(line: &str, fence_char: char, fence_len: usize) -> bool {
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let trimmed = s.trim_end();
    trimmed.chars().all(|c| c == fence_char) && trimmed.chars().count() >= fence_len
}

fn is_blockquote_start(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('>')
}

fn is_list_item_start(line: &str) -> bool {
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
            (bytes[i] == b'.' || bytes[i] == b')') && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
        }
        _ => false,
    }
}

fn is_html_block_start(line: &str) -> bool {
    let s = line.trim_start();
    s.starts_with('<') && s.len() >= 3
}

fn is_footnote_definition_start(line: &str) -> bool {
    let s = line.trim_start();
    s.starts_with("[^") && s.contains("]:")
}

fn is_footnote_continuation(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

fn count_double_dollars(line: &str) -> usize {
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

fn detect_footnotes(text: &str) -> bool {
    // Very small, streaming-friendly detector:
    // - references: [^id] (not followed by :)
    // - definitions: [^id]:
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'^' {
            // find closing ]
            let mut j = i + 2;
            while j < bytes.len() && bytes[j] != b']' && bytes[j] != b'\n' && bytes[j] != b'\r' {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b']' {
                // definition if followed by :
                if bytes.get(j + 1) == Some(&b':') {
                    return true;
                }
                // reference if not followed by :
                if bytes.get(j + 1) != Some(&b':') {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

#[derive(Debug)]
pub struct MdStream {
    opts: Options,
    buffer: String,
    lines: Vec<Line>,

    committed: Vec<Block>,
    processed_line: usize,
    current_block_start_line: usize,
    current_block_id: BlockId,
    next_block_id: u64,
    current_mode: BlockMode,

    pending_display_cache: Option<String>,
    footnotes_detected: bool,
}

impl MdStream {
    pub fn new(opts: Options) -> Self {
        let mut opts = opts;
        // Keep the window in one place: Options and TerminatorOptions should agree.
        opts.terminator.window_bytes = opts.terminator_window_bytes;
        Self {
            opts,
            buffer: String::new(),
            lines: vec![Line {
                start: 0,
                end: 0,
                has_newline: false,
            }],
            committed: Vec::new(),
            processed_line: 0,
            current_block_start_line: 0,
            current_block_id: BlockId(1),
            next_block_id: 2,
            current_mode: BlockMode::Unknown,
            pending_display_cache: None,
            footnotes_detected: false,
        }
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn snapshot_blocks(&self) -> Vec<Block> {
        let mut blocks = self.committed.clone();
        // Pending is computed without mutating state.
        if let Some(p) = self.pending_block_snapshot() {
            blocks.push(p);
        }
        blocks
    }

    fn normalize_newlines(chunk: &str) -> String {
        if !chunk.contains('\r') {
            return chunk.to_string();
        }
        let mut out = String::with_capacity(chunk.len());
        let mut chars = chunk.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            } else {
                out.push(c);
            }
        }
        out
    }

    fn append_to_lines(&mut self, chunk: &str) {
        let start_offset = self.buffer.len();
        self.buffer.push_str(chunk);

        if let Some(max) = self.opts.max_buffer_bytes {
            if self.buffer.len() > max {
                // MVP policy: keep buffer (no truncation) but could be changed to error/compaction later.
            }
        }

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

    fn start_mode_for_line(line: &str) -> BlockMode {
        if is_heading(line) {
            return BlockMode::Heading;
        }
        if is_thematic_break(line) {
            return BlockMode::ThematicBreak;
        }
        if let Some((ch, len)) = fence_start(line) {
            // Parse optional info string after the fence.
            let s = line.trim_start();
            // Skip leading fence markers.
            let mut idx = 0usize;
            while idx < s.len() && s.as_bytes()[idx] == ch as u8 {
                idx += 1;
            }
            let info = s[idx..].trim();
            let info = if info.is_empty() {
                None
            } else {
                Some(info.to_string())
            };
            return BlockMode::CodeFence {
                fence_char: ch,
                fence_len: len,
                info,
            };
        }
        if is_footnote_definition_start(line) {
            return BlockMode::FootnoteDefinition;
        }
        if is_blockquote_start(line) {
            return BlockMode::BlockQuote;
        }
        if is_list_item_start(line) {
            return BlockMode::List;
        }
        if is_html_block_start(line) {
            return BlockMode::HtmlBlock;
        }
        let dollars = count_double_dollars(line);
        if dollars % 2 == 1 && line.trim_start().starts_with("$$") {
            return BlockMode::MathBlock { open_count: dollars };
        }
        BlockMode::Paragraph
    }

    fn kind_for_mode(mode: &BlockMode) -> BlockKind {
        match mode {
            BlockMode::Paragraph => BlockKind::Paragraph,
            BlockMode::Heading => BlockKind::Heading,
            BlockMode::ThematicBreak => BlockKind::ThematicBreak,
            BlockMode::CodeFence { .. } => BlockKind::CodeFence,
            BlockMode::List => BlockKind::List,
            BlockMode::BlockQuote => BlockKind::BlockQuote,
            BlockMode::HtmlBlock => BlockKind::HtmlBlock,
            BlockMode::Table => BlockKind::Table,
            BlockMode::MathBlock { .. } => BlockKind::MathBlock,
            BlockMode::FootnoteDefinition => BlockKind::FootnoteDefinition,
            BlockMode::Unknown => BlockKind::Unknown,
        }
    }

    fn commit_block(&mut self, end_line_inclusive: usize, update: &mut Update) {
        if self.current_block_start_line >= self.lines.len() {
            return;
        }
        if end_line_inclusive < self.current_block_start_line {
            return;
        }
        let start_off = self.lines[self.current_block_start_line].start;
        let end_off = self.lines[end_line_inclusive].end_with_newline();
        if end_off <= start_off {
            return;
        }

        let raw = self.buffer[start_off..end_off].to_string();
        let block = Block {
            id: self.current_block_id,
            status: BlockStatus::Committed,
            kind: Self::kind_for_mode(&self.current_mode),
            raw,
            display: None,
        };
        self.committed.push(block.clone());
        update.committed.push(block);

        self.current_block_start_line = end_line_inclusive + 1;
        self.current_block_id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        self.current_mode = BlockMode::Unknown;
        self.pending_display_cache = None;
    }

    fn maybe_commit_single_line(&mut self, line_index: usize, update: &mut Update) {
        match self.current_mode {
            BlockMode::Heading | BlockMode::ThematicBreak => {
                self.commit_block(line_index, update);
            }
            _ => {}
        }
    }

    fn line_str(&self, line_index: usize) -> &str {
        self.lines[line_index].as_str(&self.buffer)
    }

    fn process_line(&mut self, line_index: usize, update: &mut Update) {
        // Skip if this line does not yet end with newline; we can't do stable boundary checks.
        if !self.lines[line_index].has_newline {
            return;
        }

        // If we're in SingleBlock footnote mode, we bypass block splitting.
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            return;
        }

        if line_index == self.current_block_start_line {
            let new_mode = {
                let line = self.line_str(line_index);
                if matches!(self.current_mode, BlockMode::Unknown) {
                    Some(Self::start_mode_for_line(line))
                } else {
                    None
                }
            };
            if let Some(m) = new_mode {
                self.current_mode = m;
            }
            self.maybe_commit_single_line(line_index, update);
            return;
        }

        let (boundary, next_mode) = {
            let prev = self.line_str(line_index - 1);
            let curr = self.line_str(line_index);
            let boundary = self.is_new_block_boundary(prev, curr, line_index);
            let next_mode = if boundary {
                Some(Self::start_mode_for_line(curr))
            } else {
                None
            };
            (boundary, next_mode)
        };

        // Decide if current line starts a new block; if so, commit the previous block at prev line.
        if boundary {
            self.commit_block(line_index - 1, update);
            if let Some(m) = next_mode {
                self.current_mode = m;
            }
            self.maybe_commit_single_line(line_index, update);
            return;
        }

        // Update per-block mode state transitions.
        self.update_mode_with_line(line_index, update);
    }

    fn is_new_block_boundary(&self, prev: &str, curr: &str, curr_line_index: usize) -> bool {
        // Never split inside fenced code blocks.
        if let BlockMode::CodeFence { .. } = self.current_mode {
            return false;
        }
        if let BlockMode::MathBlock { open_count } = self.current_mode {
            if open_count % 2 == 1 {
                return false;
            }
        }

        // Footnote definition: continuation lines should remain in the same block.
        if let BlockMode::FootnoteDefinition = self.current_mode {
            if is_empty_line(curr) || is_footnote_continuation(curr) {
                return false;
            }
        }

        // A new block can start after an empty line.
        if is_empty_line(prev) && !is_empty_line(curr) {
            return true;
        }

        // Certain block starters can interrupt paragraphs/lists/quotes.
        if is_heading(curr) || is_thematic_break(curr) {
            return true;
        }
        if fence_start(curr).is_some() {
            return true;
        }
        if is_footnote_definition_start(curr) {
            return true;
        }
        if is_blockquote_start(curr) && !is_blockquote_start(prev) {
            return true;
        }
        if is_list_item_start(curr) && !is_list_item_start(prev) {
            return true;
        }

        // Table detection: if current line is a delimiter and previous line contains pipes,
        // consider starting a table block at the previous line.
        if matches!(self.current_mode, BlockMode::Paragraph | BlockMode::Unknown) {
            if self.is_table_delimiter(curr) && prev.contains('|') {
                // table starts at prev line, so boundary at prev-1 if block started earlier.
                if curr_line_index >= 1 && self.current_block_start_line < curr_line_index - 1 {
                    return true;
                }
            }
        }

        false
    }

    fn is_table_delimiter(&self, line: &str) -> bool {
        let s = line.trim();
        if s.is_empty() {
            return false;
        }
        // Simple delimiter pattern: contains '-' and optional pipes/colons.
        let mut has_dash = false;
        for c in s.chars() {
            match c {
                '|' | ':' | ' ' | '\t' => {}
                '-' => has_dash = true,
                _ => return false,
            }
        }
        has_dash
    }

    fn update_mode_with_line(&mut self, line_index: usize, update: &mut Update) {
        let (start, end) = {
            let l = &self.lines[line_index];
            (l.start, l.end)
        };
        let line = &self.buffer[start..end];
        match &mut self.current_mode {
            BlockMode::Unknown => {
                self.current_mode = Self::start_mode_for_line(line);
                self.maybe_commit_single_line(line_index, update);
            }
            BlockMode::CodeFence {
                fence_char,
                fence_len,
                ..
            } => {
                if fence_end(line, *fence_char, *fence_len) {
                    self.commit_block(line_index, update);
                }
            }
            BlockMode::MathBlock { open_count } => {
                *open_count += count_double_dollars(line);
                if *open_count % 2 == 0 {
                    self.commit_block(line_index, update);
                }
            }
            BlockMode::Paragraph => {
                // Upgrade to table mode if delimiter row appears.
                if self.is_table_delimiter(line) && line_index > 0 {
                    let prev = self.lines[line_index - 1].as_str(&self.buffer);
                    if prev.contains('|') {
                        self.current_mode = BlockMode::Table;
                    }
                }
            }
            BlockMode::Table => {
                // End table when an empty line is followed by a non-table line.
                // This is handled by boundary detection on next line arrival.
            }
            BlockMode::HtmlBlock => {
                // Conservative: end HTML block on blank line boundary (handled by boundary logic).
                // More precise tag-stack handling can be added post-MVP.
            }
            BlockMode::FootnoteDefinition => {
                // Continuation handled by boundary logic.
            }
            BlockMode::List | BlockMode::BlockQuote => {
                // Conservative: rely on boundary logic on next line arrival.
            }
            BlockMode::Heading | BlockMode::ThematicBreak => {}
        }
    }

    fn pending_block_snapshot(&self) -> Option<Block> {
        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            let raw = self.buffer.clone();
            if raw.is_empty() {
                return None;
            }
            let display = terminate_markdown(&raw, &self.opts.terminator);
            return Some(Block {
                id: BlockId(1),
                status: BlockStatus::Pending,
                kind: BlockKind::Unknown,
                raw,
                display: Some(display),
            });
        }

        if self.current_block_start_line >= self.lines.len() {
            return None;
        }
        let start_off = self.lines[self.current_block_start_line].start;
        if start_off >= self.buffer.len() {
            return None;
        }
        let raw = self.buffer[start_off..].to_string();
        if raw.is_empty() {
            return None;
        }
        let mut display = terminate_markdown(&raw, &self.opts.terminator);
        display = self.maybe_repair_fenced_json_display(&raw, display, &self.current_mode);
        Some(Block {
            id: self.current_block_id,
            status: BlockStatus::Pending,
            kind: Self::kind_for_mode(&self.current_mode),
            raw,
            display: Some(display),
        })
    }

    fn current_pending_block(&mut self) -> Option<Block> {
        if let Some(cached) = &self.pending_display_cache {
            // Fast path: pending raw still needs to be refreshed.
            if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
                let raw = self.buffer.clone();
                if raw.is_empty() {
                    return None;
                }
                return Some(Block {
                    id: BlockId(1),
                    status: BlockStatus::Pending,
                    kind: BlockKind::Unknown,
                    raw,
                    display: Some(cached.clone()),
                });
            }

            if self.current_block_start_line >= self.lines.len() {
                return None;
            }
            let start_off = self.lines[self.current_block_start_line].start;
            if start_off >= self.buffer.len() {
                return None;
            }
            let raw = self.buffer[start_off..].to_string();
            if raw.is_empty() {
                return None;
            }
            return Some(Block {
                id: self.current_block_id,
                status: BlockStatus::Pending,
                kind: Self::kind_for_mode(&self.current_mode),
                raw,
                display: Some(cached.clone()),
            });
        }

        let p = self.pending_block_snapshot();
        if let Some(p) = &p {
            if let Some(d) = &p.display {
                self.pending_display_cache = Some(d.clone());
            }
        }
        p
    }

    fn maybe_repair_fenced_json_display(&self, raw: &str, display: String, mode: &BlockMode) -> String {
        if !self.opts.json_repair_in_fences {
            return display;
        }
        let BlockMode::CodeFence { info, .. } = mode else {
            return display;
        };
        let Some(info) = info.as_deref() else {
            return display;
        };
        let lang = info.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        if !matches!(lang.as_str(), "json" | "jsonc" | "json5" | "jsonl" | "jsonp") {
            return display;
        }

        #[cfg(feature = "jsonrepair")]
        {
            // Extract code body (best-effort): text after the first newline, stopping before a closing fence line.
            let Some(first_nl) = raw.find('\n') else {
                return display;
            };
            let mut body = &raw[first_nl + 1..];
            if let Some(close_line_start) = body
                .match_indices('\n')
                .map(|(i, _)| i + 1)
                .find(|&i| {
                    let line = &body[i..body[i..].find('\n').map(|r| i + r).unwrap_or(body.len())];
                    fence_start(line).is_some()
                })
            {
                body = &body[..close_line_start];
            }
            let repaired = match jsonrepair::repair_json(body, &jsonrepair::Options::default()) {
                Ok(s) => s,
                Err(_) => return display,
            };
            // Rebuild: keep opening fence line, then repaired content, then keep the rest (if any).
            let mut out = String::with_capacity(display.len());
            let open_line = &raw[..first_nl + 1];
            out.push_str(open_line);
            out.push_str(&repaired);
            return out;
        }

        #[cfg(not(feature = "jsonrepair"))]
        {
            let _ = raw;
            display
        }
    }

    pub fn append(&mut self, chunk: &str) -> Update {
        let mut update = Update::empty();
        if chunk.is_empty() {
            update.pending = self.current_pending_block();
            return update;
        }

        let chunk = Self::normalize_newlines(chunk);
        self.append_to_lines(&chunk);
        self.pending_display_cache = None;

        if !self.footnotes_detected && detect_footnotes(&self.buffer) {
            self.footnotes_detected = true;
        }

        // Process newly completed lines.
        while self.processed_line < self.lines.len() {
            if !self.lines[self.processed_line].has_newline {
                break;
            }
            self.process_line(self.processed_line, &mut update);
            self.processed_line += 1;
        }

        update.pending = self.current_pending_block();
        update
    }

    pub fn finalize(&mut self) -> Update {
        let mut update = Update::empty();

        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            if !self.buffer.is_empty() {
                let block = Block {
                    id: BlockId(1),
                    status: BlockStatus::Committed,
                    kind: BlockKind::Unknown,
                    raw: self.buffer.clone(),
                    display: None,
                };
                self.committed.push(block.clone());
                update.committed.push(block);
            }
            update.pending = None;
            return update;
        }

        if self.current_block_start_line < self.lines.len() {
            let end_line = self.lines.len() - 1;
            let start_off = self.lines[self.current_block_start_line].start;
            let end_off = self.buffer.len();
            if end_off > start_off {
                // Commit the remaining pending block.
                let raw = self.buffer[start_off..end_off].to_string();
                let block = Block {
                    id: self.current_block_id,
                    status: BlockStatus::Committed,
                    kind: Self::kind_for_mode(&self.current_mode),
                    raw,
                    display: None,
                };
                self.committed.push(block.clone());
                update.committed.push(block);
                // Reset to empty.
                self.current_block_start_line = end_line + 1;
            }
        }
        update.pending = None;
        update
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.lines.clear();
        self.lines.push(Line {
            start: 0,
            end: 0,
            has_newline: false,
        });
        self.committed.clear();
        self.processed_line = 0;
        self.current_block_start_line = 0;
        self.current_block_id = BlockId(1);
        self.next_block_id = 2;
        self.current_mode = BlockMode::Unknown;
        self.pending_display_cache = None;
        self.footnotes_detected = false;
    }
}
