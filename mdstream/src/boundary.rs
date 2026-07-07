use crate::syntax::containers::{
    names_match, parse_directive_container_line, parse_fence_container_line, parse_tag_closing,
    parse_tag_opening,
};

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
#[cfg(feature = "sync")]
pub trait BoundaryPlugin: Send + Sync {
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

#[cfg(not(feature = "sync"))]
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

#[cfg(feature = "sync")]
type StartFn = dyn FnMut(&str) + Send + Sync;
#[cfg(not(feature = "sync"))]
type StartFn = dyn FnMut(&str) + Send;

#[cfg(feature = "sync")]
type UpdateFn = dyn FnMut(&str) -> BoundaryUpdate + Send + Sync;
#[cfg(not(feature = "sync"))]
type UpdateFn = dyn FnMut(&str) -> BoundaryUpdate + Send;

#[cfg(feature = "sync")]
type ResetFn = dyn FnMut() + Send + Sync;
#[cfg(not(feature = "sync"))]
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
    #[cfg(not(feature = "sync"))]
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

    #[cfg(feature = "sync")]
    pub fn new<M, U>(matches_start: M, update: U) -> Self
    where
        M: Fn(&str) -> bool + Send + Sync + 'static,
        U: FnMut(&str) -> BoundaryUpdate + Send + Sync + 'static,
    {
        Self {
            matches_start: Box::new(matches_start),
            start: None,
            update: Box::new(update),
            reset: None,
        }
    }

    #[cfg(not(feature = "sync"))]
    pub fn with_start<S>(mut self, start: S) -> Self
    where
        S: FnMut(&str) + Send + 'static,
    {
        self.start = Some(Box::new(start));
        self
    }

    #[cfg(feature = "sync")]
    pub fn with_start<S>(mut self, start: S) -> Self
    where
        S: FnMut(&str) + Send + Sync + 'static,
    {
        self.start = Some(Box::new(start));
        self
    }

    #[cfg(not(feature = "sync"))]
    pub fn with_reset<R>(mut self, reset: R) -> Self
    where
        R: FnMut() + Send + 'static,
    {
        self.reset = Some(Box::new(reset));
        self
    }

    #[cfg(feature = "sync")]
    pub fn with_reset<R>(mut self, reset: R) -> Self
    where
        R: FnMut() + Send + Sync + 'static,
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
        parse_fence_container_line(line, self.fence_char).marker_length
    }

    fn is_end_line(&self, line: &str, opened_len: usize) -> bool {
        let parsed = parse_fence_container_line(line, self.fence_char);
        if parsed.marker_length < opened_len {
            return false;
        }
        !self.require_standalone_end || parsed.standalone_tail
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

    fn matches_opening(&self, line: &str) -> bool {
        let Some(opening) = parse_tag_opening(line) else {
            return false;
        };

        if !names_match(opening.name, self.tag.as_str(), self.case_insensitive) {
            return false;
        }
        opening.attributes.is_none() || self.allow_attributes
    }

    fn matches_closing(&self, line: &str) -> bool {
        let Some(closing) = parse_tag_closing(line) else {
            return false;
        };
        if !names_match(closing.name, self.tag.as_str(), self.case_insensitive) {
            return false;
        }
        if self.require_standalone_end {
            closing.standalone
        } else {
            closing.complete
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

    fn detect_container<'a>(
        &self,
        line: &'a str,
    ) -> Option<crate::syntax::containers::DirectiveContainerLine<'a>> {
        // Equivalent to Incremark's:
        // ^(\s*)(:{3,})(?:\s+(\w[\w-]*))?(?:\s+(.*))?\s*$
        let parsed = parse_directive_container_line(line, self.marker, self.min_marker_length)?;
        if parsed.attributes.is_some() && !self.allow_attributes {
            return None;
        }

        if !parsed.is_end() {
            if let Some(allowed) = &self.allowed_names {
                let name = parsed.name.unwrap_or("");
                if !allowed.is_empty() && !allowed.iter().any(|n| n == name) {
                    return None;
                }
            }
        }

        Some(parsed)
    }
}

impl BoundaryPlugin for ContainerBoundaryPlugin {
    fn matches_start(&self, line: &str) -> bool {
        self.detect_container(line).is_some_and(|m| !m.is_end())
    }

    fn start(&mut self, line: &str) {
        let Some(m) = self.detect_container(line) else {
            self.base_marker_length = None;
            self.depth = 0;
            self.just_started = false;
            return;
        };
        if m.is_end() {
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
        if m.is_end() && m.marker_length >= base {
            self.depth = self.depth.saturating_sub(1);
            if self.depth == 0 {
                self.base_marker_length = None;
                return BoundaryUpdate::Close;
            }
            return BoundaryUpdate::Continue;
        }
        if !m.is_end() {
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
