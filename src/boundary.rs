#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryUpdate {
    Continue,
    Close,
}

/// Participate in line-scoped context updates and stable boundary detection.
///
/// A boundary plugin can claim that a line starts a custom "container-like" block and then keep
/// the stream inside that block until it decides the block is closed.
///
/// This is designed for streaming LLM output where application-specific tags or directives should
/// not cause flickering re-parses.
pub trait BoundaryPlugin: Send {
    /// Pure predicate: return `true` if `line` can start this custom block.
    ///
    /// This method must not mutate internal state.
    fn matches_start(&self, line: &str) -> bool;

    /// Called exactly once when the current block is determined to start at `line`.
    fn start(&mut self, line: &str);

    /// Called for each line in the block (including the starting line) while this plugin is active.
    ///
    /// Return `BoundaryUpdate::Close` to close the block at the end of this line.
    fn update(&mut self, line: &str) -> BoundaryUpdate;

    fn reset(&mut self) {}
}

fn strip_up_to_three_leading_spaces(line: &str) -> &str {
    let mut s = line;
    let mut spaces = 0usize;
    while spaces < 3 && s.starts_with(' ') {
        s = &s[1..];
        spaces += 1;
    }
    s
}

/// A simple fence-like container plugin.
///
/// Typical usage is directives such as:
///
/// ```text
/// :::warning
/// content...
/// :::
/// ```
///
/// Behavior:
///
/// - Start: `fence_char` repeated `>= min_len` at the beginning of a line (after up to 3 spaces).
/// - End: `fence_char` repeated `>= opened_len` and (when `require_standalone_end`) nothing else
///   on the line besides whitespace.
#[derive(Debug, Clone)]
pub struct FenceBoundaryPlugin {
    pub fence_char: char,
    pub min_len: usize,
    pub require_standalone_end: bool,
    opened_len: Option<usize>,
    just_started: bool,
}

impl FenceBoundaryPlugin {
    pub fn new(fence_char: char, min_len: usize) -> Self {
        Self {
            fence_char,
            min_len,
            require_standalone_end: true,
            opened_len: None,
            just_started: false,
        }
    }

    pub fn triple_colon() -> Self {
        Self::new(':', 3)
    }

    fn fence_len_at_start(&self, line: &str) -> usize {
        let s = strip_up_to_three_leading_spaces(line);
        let bytes = s.as_bytes();
        let ch = self.fence_char as u8;
        let mut len = 0usize;
        while len < bytes.len() && bytes[len] == ch {
            len += 1;
        }
        len
    }

    fn is_end_line(&self, line: &str, opened_len: usize) -> bool {
        let s = strip_up_to_three_leading_spaces(line);
        let s = s.trim_end_matches(|c| c == ' ' || c == '\t');
        let bytes = s.as_bytes();
        let ch = self.fence_char as u8;
        let mut len = 0usize;
        while len < bytes.len() && bytes[len] == ch {
            len += 1;
        }
        if len < opened_len {
            return false;
        }
        if !self.require_standalone_end {
            return true;
        }
        s[len..].trim().is_empty()
    }
}

impl Default for FenceBoundaryPlugin {
    fn default() -> Self {
        Self::triple_colon()
    }
}

impl BoundaryPlugin for FenceBoundaryPlugin {
    fn matches_start(&self, line: &str) -> bool {
        self.fence_len_at_start(line) >= self.min_len
    }

    fn start(&mut self, line: &str) {
        let len = self.fence_len_at_start(line);
        if len >= self.min_len {
            self.opened_len = Some(len);
            self.just_started = true;
        } else {
            self.opened_len = None;
            self.just_started = false;
        }
    }

    fn update(&mut self, line: &str) -> BoundaryUpdate {
        let Some(opened) = self.opened_len else {
            return BoundaryUpdate::Continue;
        };
        if self.just_started {
            self.just_started = false;
            return BoundaryUpdate::Continue;
        }
        if self.is_end_line(line, opened) {
            self.opened_len = None;
            return BoundaryUpdate::Close;
        }
        BoundaryUpdate::Continue
    }

    fn reset(&mut self) {
        self.opened_len = None;
        self.just_started = false;
    }
}

/// A paired-tag container plugin.
///
/// Example:
///
/// ```text
/// <thinking>
/// ...
/// </thinking>
/// ```
///
/// This plugin is intentionally conservative:
///
/// - Start must be at the beginning of a line (after up to 3 spaces).
/// - The start tag must be complete on the line (must contain `>`).
/// - End must be a standalone closing tag line (after up to 3 spaces), unless
///   `require_standalone_end` is set to `false`.
#[derive(Debug, Clone)]
pub struct TagBoundaryPlugin {
    pub tag: String,
    pub case_insensitive: bool,
    pub allow_attributes: bool,
    pub require_standalone_end: bool,
    active: bool,
}

impl TagBoundaryPlugin {
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            case_insensitive: true,
            allow_attributes: true,
            require_standalone_end: true,
            active: false,
        }
    }

    pub fn thinking() -> Self {
        Self::new("thinking")
    }

    fn is_tag_name_char(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':'
    }

    fn norm_tag<'a>(&self, tag: &'a str) -> std::borrow::Cow<'a, str> {
        if self.case_insensitive {
            std::borrow::Cow::Owned(tag.to_ascii_lowercase())
        } else {
            std::borrow::Cow::Borrowed(tag)
        }
    }

    fn matches_opening(&self, line: &str) -> bool {
        let s = strip_up_to_three_leading_spaces(line).trim_end();
        if !s.starts_with('<') {
            return false;
        }
        // Require the tag to be complete on this line.
        let Some(gt) = s.find('>') else {
            return false;
        };
        let inside = &s[1..gt];
        if inside.starts_with('/') || inside.starts_with('!') || inside.starts_with('?') {
            return false;
        }

        let bytes = inside.as_bytes();
        if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
            return false;
        }
        let mut name_end = 1usize;
        while name_end < bytes.len() && Self::is_tag_name_char(bytes[name_end]) {
            name_end += 1;
        }
        let name = &inside[..name_end];
        let name = self.norm_tag(name);
        let want = self.norm_tag(self.tag.as_str());
        if name != want {
            return false;
        }

        let rest = inside[name_end..].trim();
        if rest.is_empty() {
            return true;
        }
        if !self.allow_attributes {
            return false;
        }
        true
    }

    fn matches_closing(&self, line: &str) -> bool {
        let s = strip_up_to_three_leading_spaces(line).trim_end();
        if !s.starts_with("</") {
            return false;
        }
        let want = self.norm_tag(self.tag.as_str());

        let after = &s[2..];
        let bytes = after.as_bytes();
        if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
            return false;
        }
        let mut name_end = 1usize;
        while name_end < bytes.len() && Self::is_tag_name_char(bytes[name_end]) {
            name_end += 1;
        }
        let name = self.norm_tag(&after[..name_end]);
        if name != want {
            return false;
        }

        let rest = after[name_end..].trim();
        if self.require_standalone_end {
            rest == ">"
        } else {
            rest.contains('>')
        }
    }
}

impl BoundaryPlugin for TagBoundaryPlugin {
    fn matches_start(&self, line: &str) -> bool {
        self.matches_opening(line)
    }

    fn start(&mut self, _line: &str) {
        self.active = true;
    }

    fn update(&mut self, line: &str) -> BoundaryUpdate {
        if !self.active {
            return BoundaryUpdate::Continue;
        }
        if self.matches_closing(line) {
            self.active = false;
            return BoundaryUpdate::Close;
        }
        BoundaryUpdate::Continue
    }

    fn reset(&mut self) {
        self.active = false;
    }
}
