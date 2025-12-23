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

type MatchStartFn = dyn Fn(&str) -> bool + Send + Sync;
type StartFn = dyn FnMut(&str) + Send;
type UpdateFn = dyn FnMut(&str) -> BoundaryUpdate + Send;
type ResetFn = dyn FnMut() + Send;

/// A lightweight adapter to implement `BoundaryPlugin` via closures.
///
/// Notes:
///
/// - `update` is called for every line in the block, including the starting line.
/// - If you need state, capture it in the `FnMut` closures.
pub struct FnBoundaryPlugin {
    matches_start: Box<MatchStartFn>,
    start: Option<Box<StartFn>>,
    update: Box<UpdateFn>,
    reset: Option<Box<ResetFn>>,
}

impl FnBoundaryPlugin {
    pub fn new<M, U>(matches_start: M, update: U) -> Self
    where
        M: Fn(&str) -> bool + Send + Sync + 'static,
        U: FnMut(&str) -> BoundaryUpdate + Send + 'static,
    {
        Self {
            matches_start: Box::new(matches_start),
            start: None,
            update: Box::new(update),
            reset: None,
        }
    }

    pub fn with_start<S>(mut self, start: S) -> Self
    where
        S: FnMut(&str) + Send + 'static,
    {
        self.start = Some(Box::new(start));
        self
    }

    pub fn with_reset<R>(mut self, reset: R) -> Self
    where
        R: FnMut() + Send + 'static,
    {
        self.reset = Some(Box::new(reset));
        self
    }
}

impl BoundaryPlugin for FnBoundaryPlugin {
    fn matches_start(&self, line: &str) -> bool {
        (self.matches_start)(line)
    }

    fn start(&mut self, line: &str) {
        if let Some(f) = self.start.as_mut() {
            (f)(line);
        }
    }

    fn update(&mut self, line: &str) -> BoundaryUpdate {
        (self.update)(line)
    }

    fn reset(&mut self) {
        if let Some(f) = self.reset.as_mut() {
            (f)();
        }
    }
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
        let s = s.trim_end_matches([' ', '\t']);
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

#[derive(Debug, Clone)]
struct ContainerMatch {
    marker_length: usize,
    is_end: bool,
}

fn is_container_name_start(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_container_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// An Incremark-compatible `:::` container plugin.
///
/// This mirrors Incremark's `detectContainer()` behavior:
///
/// - `::: name` starts a container
/// - `:::` ends a container
/// - longer markers like `:::::` are allowed (useful for nesting)
/// - nesting depth is tracked; each end marker closes one level
#[derive(Debug, Clone)]
pub struct ContainerBoundaryPlugin {
    pub marker: char,
    pub min_marker_length: usize,
    pub allowed_names: Option<Vec<String>>,
    pub allow_attributes: bool,

    base_marker_length: Option<usize>,
    depth: usize,
    just_started: bool,
}

impl Default for ContainerBoundaryPlugin {
    fn default() -> Self {
        Self::new(':', 3)
    }
}

impl ContainerBoundaryPlugin {
    pub fn new(marker: char, min_marker_length: usize) -> Self {
        Self {
            marker,
            min_marker_length,
            allowed_names: None,
            allow_attributes: true,
            base_marker_length: None,
            depth: 0,
            just_started: false,
        }
    }

    fn detect_container(&self, line: &str) -> Option<ContainerMatch> {
        // Equivalent to Incremark's:
        // ^(\s*)(:{3,})(?:\s+(\w[\w-]*))?(?:\s+(.*))?\s*$
        let s = line.trim_end();
        let s = s.trim_start();
        let bytes = s.as_bytes();
        let marker = self.marker as u8;
        let mut i = 0usize;
        while i < bytes.len() && bytes[i] == marker {
            i += 1;
        }
        if i < self.min_marker_length {
            return None;
        }
        let marker_length = i;
        let mut rest = s[i..].trim_end_matches([' ', '\t']);
        if rest.is_empty() {
            return Some(ContainerMatch {
                marker_length,
                is_end: true,
            });
        }

        // Incremark requires at least one whitespace before name/attrs.
        if !rest
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            return None;
        }
        rest = rest.trim_start_matches([' ', '\t']);

        // Parse optional name.
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

        let name = if name_end > 0 {
            rest[..name_end].to_string()
        } else {
            String::new()
        };

        let attrs = rest[name_end..].trim();
        let has_attrs = !attrs.is_empty();
        if has_attrs && !self.allow_attributes {
            return None;
        }

        let is_end = name.is_empty() && !has_attrs;
        if !is_end {
            if let Some(allowed) = &self.allowed_names {
                if !allowed.is_empty() && !allowed.iter().any(|n| n == &name) {
                    return None;
                }
            }
        }

        Some(ContainerMatch {
            marker_length,
            is_end,
        })
    }
}

impl BoundaryPlugin for ContainerBoundaryPlugin {
    fn matches_start(&self, line: &str) -> bool {
        self.detect_container(line).is_some_and(|m| !m.is_end)
    }

    fn start(&mut self, line: &str) {
        let Some(m) = self.detect_container(line) else {
            self.base_marker_length = None;
            self.depth = 0;
            self.just_started = false;
            return;
        };
        if m.is_end {
            self.base_marker_length = None;
            self.depth = 0;
            self.just_started = false;
            return;
        }
        self.base_marker_length = Some(m.marker_length);
        self.depth = 1;
        self.just_started = true;
    }

    fn update(&mut self, line: &str) -> BoundaryUpdate {
        if self.depth == 0 {
            return BoundaryUpdate::Continue;
        }
        let Some(base) = self.base_marker_length else {
            return BoundaryUpdate::Continue;
        };
        if self.just_started {
            self.just_started = false;
            return BoundaryUpdate::Continue;
        }
        let Some(m) = self.detect_container(line) else {
            return BoundaryUpdate::Continue;
        };
        if m.is_end && m.marker_length >= base {
            self.depth = self.depth.saturating_sub(1);
            if self.depth == 0 {
                self.base_marker_length = None;
                return BoundaryUpdate::Close;
            }
            return BoundaryUpdate::Continue;
        }
        if !m.is_end {
            self.depth += 1;
        }
        BoundaryUpdate::Continue
    }

    fn reset(&mut self) {
        self.base_marker_length = None;
        self.depth = 0;
        self.just_started = false;
    }
}
