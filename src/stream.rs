use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::boundary::{BoundaryPlugin, BoundaryUpdate};
use crate::options::{FootnotesMode, Options, ReferenceDefinitionsMode};
use crate::pending::terminate_markdown;
use crate::transform::{PendingTransformInput, PendingTransformer};
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
    },
    CustomBoundary {
        plugin_index: usize,
        started: bool,
    },
    List,
    BlockQuote,
    HtmlBlock {
        stack: Vec<String>,
        in_comment: bool,
    },
    Table,
    MathBlock {
        open_count: usize,
    },
    FootnoteDefinition,
}

fn is_empty_line(line: &str) -> bool {
    line.trim().is_empty()
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('#') && trimmed[1..].starts_with([' ', '\t', '#'])
}

fn thematic_break_char(line: &str) -> Option<char> {
    // CommonMark-like thematic break:
    // - up to 3 leading spaces
    // - one of '-', '*', '_' repeated >= 3
    // - spaces/tabs may appear between markers
    // - no other characters
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let s = s.trim_end_matches([' ', '\t']);
    let mut it = s.chars();
    let first = it.next()?;
    if first != '-' && first != '*' && first != '_' {
        return None;
    }
    let mut count = 1usize;
    for c in it {
        if c == first {
            count += 1;
            continue;
        }
        if c == ' ' || c == '\t' {
            continue;
        }
        return None;
    }
    if count >= 3 { Some(first) } else { None }
}

fn is_thematic_break(line: &str) -> bool {
    thematic_break_char(line).is_some()
}

fn setext_underline_char(line: &str) -> Option<char> {
    // Best-effort setext underline:
    // - up to 3 leading spaces
    // - '=' or '-' repeated >= 2
    // - spaces/tabs may appear between markers
    // - no other characters
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let s = s.trim_end_matches([' ', '\t']);
    let mut it = s.chars();
    let first = it.next()?;
    if first != '=' && first != '-' {
        return None;
    }
    let mut count = 1usize;
    for c in it {
        if c == first {
            count += 1;
            continue;
        }
        if c == ' ' || c == '\t' {
            continue;
        }
        return None;
    }
    if count >= 2 { Some(first) } else { None }
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
            (bytes[i] == b'.' || bytes[i] == b')')
                && (bytes[i + 1] == b' ' || bytes[i + 1] == b'\t')
        }
        _ => false,
    }
}

fn is_list_continuation(line: &str) -> bool {
    // Best-effort continuation line for lists:
    // - indented content (>=2 spaces or a tab)
    // - or a nested list item starter
    if is_list_item_start(line) {
        return true;
    }
    let bytes = line.as_bytes();
    if bytes.first() == Some(&b'\t') {
        return true;
    }
    let mut spaces = 0usize;
    for &b in bytes {
        if b == b' ' {
            spaces += 1;
            if spaces >= 2 {
                return true;
            }
            continue;
        }
        break;
    }
    false
}

fn is_list_item_start_prefix(line: &str) -> bool {
    // Streaming-only heuristic: treat certain "prefix" lines as potential list item starters.
    // This prevents premature block commits when the marker is split across chunks (e.g. "-" then " item").
    let s = line.trim_start();
    if s.is_empty() {
        return false;
    }
    let bytes = s.as_bytes();
    match bytes[0] {
        b'-' | b'+' | b'*' => s.len() == 1,
        b'0'..=b'9' => {
            let mut i = 0usize;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == 0 {
                return false;
            }
            if i == bytes.len() {
                // Digits only: could become "1." / "1)" with more input.
                return true;
            }
            if bytes[i] != b'.' && bytes[i] != b')' {
                return false;
            }
            if i + 1 == bytes.len() {
                // "1." or "1)" without the required whitespace yet.
                return true;
            }
            false
        }
        _ => false,
    }
}

fn is_footnote_definition_start(line: &str) -> bool {
    let s = line.trim_start();
    s.starts_with("[^") && s.contains("]:")
}

fn html_block_start_state(line: &str) -> Option<(Vec<String>, bool)> {
    // Best-effort HTML block start (block-level):
    // - up to 3 leading spaces
    // - starts with an HTML tag open/close or comment opener
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let s = s.trim_end();
    if !s.starts_with('<') || s.len() < 3 {
        return None;
    }
    // Recognize a tag-like start. This rejects autolinks like "<https://...>" (':' after name).
    let _ = parse_tag_at(s, 0)?;
    // We intentionally start with an empty stack; the per-line state update will process tags
    // (including the opening tag on the first line) in a single place.
    Some((Vec::new(), false))
}

#[derive(Debug, Clone)]
enum HtmlTag {
    Opening { name: String, self_closing: bool },
    Closing { name: String },
    CommentOpen,
}

fn is_ascii_tag_name_char(b: u8) -> bool {
    // Streamdown uses `<(\\w+)[\\s>]` for tag names (`\\w` includes `_`).
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_void_html_tag(name: &str) -> bool {
    // Common void elements that never have closing tags.
    // This list is intentionally small and can be expanded post-MVP.
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn parse_tag_at(s: &str, lt_index: usize) -> Option<(HtmlTag, &str)> {
    // Parse a tag starting at '<' (at byte offset lt_index within s).
    let bytes = s.as_bytes();
    if lt_index >= bytes.len() || bytes[lt_index] != b'<' {
        return None;
    }
    if s[lt_index..].starts_with("<!--") {
        return Some((HtmlTag::CommentOpen, &s[lt_index + 4..]));
    }
    let mut i = lt_index + 1;
    if i >= bytes.len() {
        return None;
    }
    let is_closing = bytes[i] == b'/';
    if is_closing {
        i += 1;
    }
    if i >= bytes.len() || !bytes[i].is_ascii_alphabetic() {
        return None;
    }
    let name_start = i;
    i += 1;
    while i < bytes.len() && is_ascii_tag_name_char(bytes[i]) {
        i += 1;
    }
    let name = &s[name_start..i];
    // Must be followed by whitespace, '>', or '/' to be tag-like.
    let next = bytes.get(i).copied().unwrap_or(b'\0');
    if !(next == b' ' || next == b'\t' || next == b'>' || next == b'/') {
        return None;
    }
    // Find end of tag '>' on this line segment.
    let close_rel = s[i..].find('>')?;
    let close = i + close_rel;

    if is_closing {
        return Some((
            HtmlTag::Closing {
                name: name.to_ascii_lowercase(),
            },
            &s[close + 1..],
        ));
    }

    // Determine self-closing by checking '/' before '>' (ignoring trailing whitespace).
    let mut j = close;
    while j > i && matches!(bytes[j - 1], b' ' | b'\t') {
        j -= 1;
    }
    let self_closing =
        (j > i && bytes[j - 1] == b'/') || is_void_html_tag(&name.to_ascii_lowercase());
    Some((
        HtmlTag::Opening {
            name: name.to_ascii_lowercase(),
            self_closing,
        },
        &s[close + 1..],
    ))
}

fn apply_tag_to_stack(tag: &HtmlTag, rest: &str, stack: &mut Vec<String>, in_comment: &mut bool) {
    match tag {
        HtmlTag::CommentOpen => {
            // If the comment closes on the same line, do not enter comment mode.
            if !rest.contains("-->") {
                *in_comment = true;
            }
        }
        HtmlTag::Opening { name, self_closing } => {
            if !*self_closing {
                stack.push(name.clone());
            }
        }
        HtmlTag::Closing { name } => {
            if stack.last().is_some_and(|t| t == name) {
                stack.pop();
            } else {
                // Best-effort: do not attempt arbitrary stack rewrites.
            }
        }
    }
}

fn update_html_block_state(line: &str, stack: &mut Vec<String>, in_comment: &mut bool) {
    let mut s = line;
    loop {
        if *in_comment {
            let Some(pos) = s.find("-->") else {
                return;
            };
            *in_comment = false;
            s = &s[pos + 3..];
            continue;
        }

        let Some(lt_rel) = s.find('<') else {
            return;
        };
        let lt = lt_rel;
        let after = &s[lt..];
        let Some((tag, rest)) = parse_tag_at(after, 0) else {
            s = &s[lt + 1..];
            continue;
        };
        apply_tag_to_stack(&tag, rest, stack, in_comment);

        // Continue scanning after the parsed tag opener/closer.
        s = rest;
    }
}

fn is_footnote_continuation(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

fn extract_reference_definition_label(line: &str) -> Option<String> {
    // CommonMark-ish reference definition, single line only:
    // up to 3 leading spaces, then "[label]:"
    //
    // We purposely keep this lightweight and streaming-friendly; multi-line definitions
    // can be supported later via a dedicated block mode.
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    let bytes = s.as_bytes();
    if bytes.len() < 4 || bytes[0] != b'[' {
        return None;
    }
    let close = s.find(']')?;
    if close == 1 {
        return None;
    }
    if s.as_bytes().get(close + 1) != Some(&b':') {
        return None;
    }
    let label = &s[1..close];
    // Exclude footnote definitions like "[^1]:"
    if label.starts_with('^') {
        return None;
    }
    normalize_reference_label(label)
}

fn normalize_reference_label(label: &str) -> Option<String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Keep a conservative cap similar to Streamdown's footnote patterns.
    if trimmed.len() > 200 {
        return None;
    }
    let mut out = String::with_capacity(trimmed.len());
    let mut last_was_ws = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            last_was_ws = true;
            continue;
        }
        if last_was_ws && !out.is_empty() {
            out.push(' ');
        }
        last_was_ws = false;
        for lc in ch.to_lowercase() {
            out.push(lc);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn extract_reference_usages(text: &str) -> HashSet<String> {
    // Best-effort extractor for reference-style link labels:
    // - [text][label]
    // - [label][]
    // - [label] (shortcut)
    //
    // We intentionally over-approximate: false positives only cause extra invalidations.
    let bytes = text.as_bytes();
    let mut out = HashSet::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let Some(close1_rel) = text[i + 1..].find(']') else {
            break;
        };
        let close1 = i + 1 + close1_rel;
        let label1 = &text[i + 1..close1];
        // Skip footnote-ish labels.
        if label1.starts_with('^') {
            i = close1 + 1;
            continue;
        }

        // Inline links/images: [text](...) / ![alt](...)
        if bytes.get(close1 + 1) == Some(&b'(') {
            i = close1 + 1;
            continue;
        }
        // Definition: [label]: ...
        if bytes.get(close1 + 1) == Some(&b':') {
            i = close1 + 1;
            continue;
        }

        // Reference form: [text][label] or [label][]
        if bytes.get(close1 + 1) == Some(&b'[') {
            let start2 = close1 + 2;
            if start2 >= bytes.len() {
                break;
            }
            let Some(close2_rel) = text[start2..].find(']') else {
                break;
            };
            let close2 = start2 + close2_rel;
            let label2 = &text[start2..close2];
            let chosen = if label2.trim().is_empty() {
                label1
            } else {
                label2
            };
            if let Some(norm) = normalize_reference_label(chosen) {
                out.insert(norm);
            }
            i = close2 + 1;
            continue;
        }

        // Shortcut reference: [label]
        if let Some(norm) = normalize_reference_label(label1) {
            out.insert(norm);
        }
        i = close1 + 1;
    }
    out
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
    pending_transformers: Vec<Box<dyn PendingTransformer>>,
    boundary_plugins: Vec<Box<dyn BoundaryPlugin>>,
    active_boundary_plugin: Option<usize>,
    footnotes_detected: bool,
    footnote_scan_tail: String,
    pending_cr: bool,
    last_finalized_buffer_len: usize,

    reference_usage_index: HashMap<String, HashSet<BlockId>>,
}

impl std::fmt::Debug for MdStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MdStream")
            .field("buffer_len", &self.buffer.len())
            .field("lines_len", &self.lines.len())
            .field("committed_len", &self.committed.len())
            .field("processed_line", &self.processed_line)
            .field("current_block_start_line", &self.current_block_start_line)
            .field("current_block_id", &self.current_block_id)
            .field("next_block_id", &self.next_block_id)
            .field(
                "pending_display_cache",
                &self.pending_display_cache.is_some(),
            )
            .field("pending_transformers_len", &self.pending_transformers.len())
            .field("boundary_plugins_len", &self.boundary_plugins.len())
            .field("active_boundary_plugin", &self.active_boundary_plugin)
            .field("footnotes_detected", &self.footnotes_detected)
            .field("last_finalized_buffer_len", &self.last_finalized_buffer_len)
            .finish()
    }
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
            pending_transformers: Vec::new(),
            boundary_plugins: Vec::new(),
            active_boundary_plugin: None,
            footnotes_detected: false,
            footnote_scan_tail: String::new(),
            pending_cr: false,
            last_finalized_buffer_len: 0,
            reference_usage_index: HashMap::new(),
        }
    }

    /// Construct a stream with Streamdown-compatible defaults for incomplete links/images.
    ///
    /// This keeps the built-in terminator for emphasis/inline code/etc, but delegates incomplete
    /// link/image handling to the built-in pending transformers.
    pub fn streamdown_defaults() -> Self {
        // Use the transformers for link/image behavior so consumers can swap them out.
        let opts = Options {
            terminator: crate::pending::TerminatorOptions {
                links: false,
                images: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut s = MdStream::new(opts.clone());
        s.push_pending_transformer(crate::transform::IncompleteLinkPlaceholderTransformer {
            incomplete_link_url: opts.terminator.incomplete_link_url,
            window_bytes: opts.terminator_window_bytes,
        });
        s.push_pending_transformer(crate::transform::IncompleteImageDropTransformer {
            window_bytes: opts.terminator_window_bytes,
        });
        s
    }

    pub fn push_pending_transformer<T>(&mut self, transformer: T)
    where
        T: PendingTransformer + 'static,
    {
        self.pending_transformers.push(Box::new(transformer));
        self.pending_display_cache = None;
    }

    pub fn with_pending_transformer<T>(mut self, transformer: T) -> Self
    where
        T: PendingTransformer + 'static,
    {
        self.push_pending_transformer(transformer);
        self
    }

    pub fn push_boundary_plugin<T>(&mut self, plugin: T)
    where
        T: BoundaryPlugin + 'static,
    {
        self.boundary_plugins.push(Box::new(plugin));
        self.pending_display_cache = None;
    }

    pub fn with_boundary_plugin<T>(mut self, plugin: T) -> Self
    where
        T: BoundaryPlugin + 'static,
    {
        self.push_boundary_plugin(plugin);
        self
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

    fn normalize_newlines_cow<'a>(&mut self, chunk: &'a str) -> Cow<'a, str> {
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

    fn start_mode_for_line(&self, line: &str) -> BlockMode {
        if let Some(idx) = self
            .boundary_plugins
            .iter()
            .position(|p| p.matches_start(line))
        {
            return BlockMode::CustomBoundary {
                plugin_index: idx,
                started: false,
            };
        }
        if is_heading(line) {
            return BlockMode::Heading;
        }
        if is_thematic_break(line) {
            return BlockMode::ThematicBreak;
        }
        if let Some((ch, len)) = fence_start(line) {
            return BlockMode::CodeFence {
                fence_char: ch,
                fence_len: len,
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
        if let Some((stack, in_comment)) = html_block_start_state(line) {
            return BlockMode::HtmlBlock { stack, in_comment };
        }
        let dollars = count_double_dollars(line);
        if dollars % 2 == 1 && line.trim_start().starts_with("$$") {
            // `open_count` is tracked via `update_mode_with_line`, including the opening line.
            return BlockMode::MathBlock { open_count: 0 };
        }
        BlockMode::Paragraph
    }

    fn kind_for_mode(mode: &BlockMode) -> BlockKind {
        match mode {
            BlockMode::Paragraph => BlockKind::Paragraph,
            BlockMode::Heading => BlockKind::Heading,
            BlockMode::ThematicBreak => BlockKind::ThematicBreak,
            BlockMode::CodeFence { .. } => BlockKind::CodeFence,
            BlockMode::CustomBoundary { .. } => BlockKind::Unknown,
            BlockMode::List => BlockKind::List,
            BlockMode::BlockQuote => BlockKind::BlockQuote,
            BlockMode::HtmlBlock { .. } => BlockKind::HtmlBlock,
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
        if raw.trim().is_empty() {
            // Never emit whitespace-only blocks. Keep stable behavior by advancing the block cursor.
            self.current_block_start_line = end_line_inclusive + 1;
            self.current_block_id = BlockId(self.next_block_id);
            self.next_block_id += 1;
            self.current_mode = BlockMode::Unknown;
            self.active_boundary_plugin = None;
            self.pending_display_cache = None;
            return;
        }
        let block = Block {
            id: self.current_block_id,
            status: BlockStatus::Committed,
            kind: Self::kind_for_mode(&self.current_mode),
            raw,
            display: None,
        };
        self.push_committed_block(block, update);

        self.current_block_start_line = end_line_inclusive + 1;
        self.current_block_id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        self.current_mode = BlockMode::Unknown;
        self.active_boundary_plugin = None;
        self.pending_display_cache = None;
    }

    fn push_committed_block(&mut self, block: Block, update: &mut Update) {
        // Index usages for invalidation-based adapters.
        if block.kind != BlockKind::CodeFence && block.raw.contains('[') {
            let used = extract_reference_usages(&block.raw);
            if !used.is_empty() {
                for label in used {
                    self.reference_usage_index
                        .entry(label)
                        .or_default()
                        .insert(block.id);
                }
            }
        }

        // Emit invalidations when new reference definitions arrive.
        if self.opts.reference_definitions == ReferenceDefinitionsMode::Invalidate
            && block.kind != BlockKind::CodeFence
            && block.raw.contains("]:")
        {
            let mut invalidated = HashSet::new();
            for line in block.raw.split('\n') {
                let Some(label) = extract_reference_definition_label(line) else {
                    continue;
                };
                if let Some(ids) = self.reference_usage_index.get(&label) {
                    for id in ids {
                        if *id != block.id {
                            invalidated.insert(*id);
                        }
                    }
                }
            }
            if !invalidated.is_empty() {
                let mut ids: Vec<BlockId> = invalidated.into_iter().collect();
                ids.sort_by_key(|id| id.0);
                update.invalidated.extend(ids);
            }
        }

        self.committed.push(block.clone());
        update.committed.push(block);
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
            // Defensive: the first line of a block is the single source of truth for the block mode.
            // This avoids stale-mode edge cases where `current_mode` is not `Unknown` at a new start.
            self.current_mode = self.start_mode_for_line(self.line_str(line_index));
            self.maybe_commit_single_line(line_index, update);
            // Even on the first line, some modes need to update internal state (e.g. HTML tag stack).
            self.update_mode_with_line(line_index, update);
            return;
        }

        let (boundary, next_mode) = {
            let prev = self.line_str(line_index - 1);
            let curr = self.line_str(line_index);
            let boundary = self.is_new_block_boundary(prev, curr, line_index);
            let next_mode = if boundary {
                Some(self.start_mode_for_line(curr))
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
            // If we started a new mode on this line, we must also update its per-line state.
            // This is required for modes like HTML/math where the opening line affects context.
            self.update_mode_with_line(line_index, update);
            return;
        }

        // Update per-block mode state transitions.
        self.update_mode_with_line(line_index, update);
    }

    fn process_incomplete_tail_boundary(&mut self, update: &mut Update) {
        if self.lines.len() < 2 {
            return;
        }
        let last = self.lines.len() - 1;
        if self.lines[last].has_newline {
            return;
        }
        if !self.lines[last - 1].has_newline {
            return;
        }

        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            return;
        }

        let boundary = {
            let prev = self.line_str(last - 1);
            let curr = self.line_str(last);
            self.is_new_block_boundary(prev, curr, last)
        };

        if boundary {
            self.commit_block(last - 1, update);
            self.current_mode = self.start_mode_for_line(self.line_str(last));
        }
    }

    fn is_new_block_boundary(&self, prev: &str, curr: &str, curr_line_index: usize) -> bool {
        // Never split inside fenced code blocks.
        if let BlockMode::CodeFence { .. } = self.current_mode {
            return false;
        }
        if let BlockMode::CustomBoundary { .. } = self.current_mode {
            return false;
        }
        if let BlockMode::MathBlock { open_count } = self.current_mode {
            if open_count % 2 == 1 {
                return false;
            }
        }
        if let BlockMode::HtmlBlock { stack, in_comment } = &self.current_mode {
            if *in_comment || !stack.is_empty() {
                return false;
            }
        }

        // Footnote definition: continuation lines should remain in the same block.
        if let BlockMode::FootnoteDefinition = self.current_mode {
            if is_empty_line(curr) || is_footnote_continuation(curr) {
                return false;
            }
            // A non-indented, non-empty line ends the footnote definition even without a blank line.
            return true;
        }

        // A new block can start after an empty line.
        if is_empty_line(prev) && !is_empty_line(curr) {
            // Be robust against mode drift in streaming scenarios: the current block's "start line"
            // is the source of truth for whether we're inside a list/quote container.
            let block_start_mode =
                self.start_mode_for_line(self.line_str(self.current_block_start_line));
            let in_list = matches!(self.current_mode, BlockMode::List)
                || matches!(block_start_mode, BlockMode::List);
            let in_blockquote = matches!(self.current_mode, BlockMode::BlockQuote)
                || matches!(block_start_mode, BlockMode::BlockQuote);
            // Lists can legally contain blank lines between items and within an item's continuation.
            if in_list && (is_list_continuation(curr) || is_list_item_start_prefix(curr)) {
                return false;
            }
            // Blockquotes can continue after blank lines only if the marker is present.
            if in_blockquote && is_blockquote_start(curr) {
                return false;
            }
            return true;
        }

        // Setext heading underline is part of the current paragraph block, not a new block boundary.
        if matches!(self.current_mode, BlockMode::Paragraph | BlockMode::Unknown)
            && setext_underline_char(curr).is_some()
            && !is_empty_line(prev)
            && self.current_block_start_line + 1 == curr_line_index
        {
            return false;
        }

        // Certain block starters can interrupt paragraphs/lists/quotes.
        if is_heading(curr) || is_thematic_break(curr) {
            return true;
        }
        if fence_start(curr).is_some() {
            return true;
        }
        if self.boundary_plugins.iter().any(|p| p.matches_start(curr)) {
            return true;
        }
        if is_footnote_definition_start(curr) {
            return true;
        }
        if is_blockquote_start(curr)
            && !is_blockquote_start(prev)
            && !matches!(self.current_mode, BlockMode::BlockQuote)
        {
            return true;
        }
        if is_list_item_start(curr)
            && !is_list_item_start(prev)
            && !matches!(self.current_mode, BlockMode::List)
        {
            return true;
        }

        // Table detection: if current line is a delimiter and previous line contains pipes,
        // consider starting a table block at the previous line.
        if matches!(self.current_mode, BlockMode::Paragraph | BlockMode::Unknown)
            && self.is_table_delimiter(curr)
            && prev.contains('|')
            // table starts at prev line, so boundary at prev-1 if block started earlier.
            && curr_line_index >= 1
            && self.current_block_start_line < curr_line_index - 1
        {
            return true;
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
                self.current_mode = self.start_mode_for_line(line);
                self.maybe_commit_single_line(line_index, update);
            }
            BlockMode::CodeFence {
                fence_char,
                fence_len,
            } => {
                if fence_end(line, *fence_char, *fence_len) {
                    self.commit_block(line_index, update);
                }
            }
            BlockMode::CustomBoundary {
                plugin_index,
                started,
            } => {
                let idx = *plugin_index;
                if idx >= self.boundary_plugins.len() {
                    return;
                }
                self.active_boundary_plugin = Some(idx);
                if !*started {
                    self.boundary_plugins[idx].start(line);
                    *started = true;
                }
                if self.boundary_plugins[idx].update(line) == BoundaryUpdate::Close {
                    self.active_boundary_plugin = None;
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
                // Upgrade to setext heading if underline appears right after a single paragraph line.
                if setext_underline_char(line).is_some()
                    && self.current_block_start_line + 1 == line_index
                    && line_index > 0
                {
                    let prev = self.lines[line_index - 1].as_str(&self.buffer);
                    if !is_empty_line(prev) {
                        self.current_mode = BlockMode::Heading;
                        self.commit_block(line_index, update);
                        return;
                    }
                }
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
            BlockMode::HtmlBlock { stack, in_comment } => {
                update_html_block_state(line, stack, in_comment);
                if !*in_comment && stack.is_empty() {
                    self.commit_block(line_index, update);
                }
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
            let kind = BlockKind::Unknown;
            let display = self.transform_pending_display(
                kind,
                &raw,
                terminate_markdown(&raw, &self.opts.terminator),
            );
            return Some(Block {
                id: BlockId(1),
                status: BlockStatus::Pending,
                kind,
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
        let kind = if matches!(self.current_mode, BlockMode::Unknown) {
            let mode = self.start_mode_for_line(self.line_str(self.current_block_start_line));
            Self::kind_for_mode(&mode)
        } else {
            Self::kind_for_mode(&self.current_mode)
        };
        let mut display = terminate_markdown(&raw, &self.opts.terminator);
        display = self.transform_pending_display(kind, &raw, display);
        Some(Block {
            id: self.current_block_id,
            status: BlockStatus::Pending,
            kind,
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

    fn transform_pending_display(&self, kind: BlockKind, raw: &str, mut display: String) -> String {
        if self.pending_transformers.is_empty() {
            return display;
        }
        for t in &self.pending_transformers {
            if let Some(next) = t.transform(PendingTransformInput {
                kind,
                raw,
                display: &display,
            }) {
                display = next;
            }
        }
        display
    }

    pub fn append(&mut self, chunk: &str) -> Update {
        let mut update = Update::empty();
        if chunk.is_empty() && !self.pending_cr {
            update.pending = self.current_pending_block();
            return update;
        }

        let footnotes_before = self.footnotes_detected;
        let chunk = self.normalize_newlines_cow(chunk);

        if !self.footnotes_detected {
            if detect_footnotes(chunk.as_ref()) {
                self.footnotes_detected = true;
            } else {
                // Keep a small tail window to detect patterns across chunk boundaries.
                const MAX_TAIL: usize = 256;
                let chunk_prefix = take_prefix_at_char_boundary(chunk.as_ref(), MAX_TAIL);
                if !self.footnote_scan_tail.is_empty() && !chunk_prefix.is_empty() {
                    let mut combined =
                        String::with_capacity(self.footnote_scan_tail.len() + chunk_prefix.len());
                    combined.push_str(&self.footnote_scan_tail);
                    combined.push_str(chunk_prefix);
                    if detect_footnotes(&combined) {
                        self.footnotes_detected = true;
                    }
                }
                if !self.footnotes_detected {
                    update_tail(&mut self.footnote_scan_tail, chunk.as_ref(), MAX_TAIL);
                }
            }
        }

        let enter_single_block_footnotes = !footnotes_before
            && self.footnotes_detected
            && self.opts.footnotes == FootnotesMode::SingleBlock;

        self.append_to_lines(chunk.as_ref());
        self.pending_display_cache = None;

        if enter_single_block_footnotes {
            self.reset_for_single_block_footnotes(&mut update);
            update.pending = self.current_pending_block();
            return update;
        }

        // Process newly completed lines.
        while self.processed_line < self.lines.len() {
            if !self.lines[self.processed_line].has_newline {
                break;
            }
            self.process_line(self.processed_line, &mut update);
            self.processed_line += 1;
        }

        // Even if the current last line has no newline yet, we may have enough information to
        // commit the previous block (eg after a blank line).
        self.process_incomplete_tail_boundary(&mut update);

        update.pending = self.current_pending_block();
        update
    }

    fn reset_for_single_block_footnotes(&mut self, update: &mut Update) {
        update.reset = true;

        self.committed.clear();
        self.reference_usage_index.clear();
        self.pending_display_cache = None;
        self.active_boundary_plugin = None;

        // Re-start IDs so consumers can treat it as a new document.
        self.current_block_start_line = 0;
        self.current_block_id = BlockId(1);
        self.next_block_id = 2;
        self.current_mode = BlockMode::Unknown;

        // We intentionally stop line processing in this mode.
        self.processed_line = self.lines.len();
    }

    pub fn finalize(&mut self) -> Update {
        if !self.pending_cr && self.buffer.len() == self.last_finalized_buffer_len {
            return Update::empty();
        }

        let mut update = Update::empty();

        if self.pending_cr {
            // Treat a trailing '\r' at EOF as a newline.
            self.append_to_lines("\n");
            self.pending_cr = false;
        }

        if self.opts.footnotes == FootnotesMode::SingleBlock && self.footnotes_detected {
            if !self.buffer.is_empty() {
                if self.buffer.trim().is_empty() {
                    update.pending = None;
                    return update;
                }
                let block = Block {
                    id: BlockId(1),
                    status: BlockStatus::Committed,
                    kind: BlockKind::Unknown,
                    raw: self.buffer.clone(),
                    display: None,
                };
                self.push_committed_block(block, &mut update);
            }
            update.pending = None;
            self.last_finalized_buffer_len = self.buffer.len();
            return update;
        }

        if self.current_block_start_line < self.lines.len() {
            let end_line = self.lines.len() - 1;
            let start_off = self.lines[self.current_block_start_line].start;
            let end_off = self.buffer.len();
            if end_off > start_off {
                // Commit the remaining pending block.
                if matches!(self.current_mode, BlockMode::Unknown) {
                    self.current_mode =
                        self.start_mode_for_line(self.line_str(self.current_block_start_line));
                }
                let raw = self.buffer[start_off..end_off].to_string();
                if raw.trim().is_empty() {
                    update.pending = None;
                    return update;
                }
                let block = Block {
                    id: self.current_block_id,
                    status: BlockStatus::Committed,
                    kind: Self::kind_for_mode(&self.current_mode),
                    raw,
                    display: None,
                };
                self.push_committed_block(block, &mut update);
                // Reset to empty.
                self.current_block_start_line = end_line + 1;
            }
        }
        update.pending = None;
        self.last_finalized_buffer_len = self.buffer.len();
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
        for t in &self.pending_transformers {
            t.reset();
        }
        for p in self.boundary_plugins.iter_mut() {
            p.reset();
        }
        self.active_boundary_plugin = None;
        self.footnotes_detected = false;
        self.footnote_scan_tail.clear();
        self.pending_cr = false;
        self.last_finalized_buffer_len = 0;
        self.reference_usage_index.clear();
    }
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

impl Default for MdStream {
    fn default() -> Self {
        Self::new(Options::default())
    }
}

#[cfg(test)]
mod html_state_tests {
    use super::*;

    #[test]
    fn html_stack_tracks_section_with_nested_p() {
        let mut stack = Vec::<String>::new();
        let mut in_comment = false;
        update_html_block_state("<section>", &mut stack, &mut in_comment);
        assert_eq!(stack, vec!["section".to_string()]);
        update_html_block_state("  <p>Second block</p>", &mut stack, &mut in_comment);
        assert_eq!(stack, vec!["section".to_string()]);
        update_html_block_state("</section>", &mut stack, &mut in_comment);
        assert!(stack.is_empty());
    }
}
