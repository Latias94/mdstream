use crate::pending::TerminatorOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FootnotesMode {
    /// If footnotes are detected, treat the whole document as a single block.
    SingleBlock,
    /// Keep blocks but allow adapters to selectively re-parse via invalidation events.
    ///
    /// Note: Invalidation support is planned post-MVP.
    Invalidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceDefinitionsMode {
    /// Keep blocks stable; reference definitions may be interpreted late by adapters.
    StabilityFirst,
    /// Emit invalidation events so adapters can selectively re-parse affected blocks.
    ///
    /// Note: Invalidation support is planned post-MVP.
    Invalidate,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub footnotes: FootnotesMode,
    pub reference_definitions: ReferenceDefinitionsMode,
    pub terminator: TerminatorOptions,
    pub terminator_window_bytes: usize,
    /// Optional hard cap for the internal buffer.
    pub max_buffer_bytes: Option<usize>,
    /// If enabled (and the `jsonrepair` feature is on), repair JSON inside fenced JSON blocks
    /// for the pending display string.
    pub json_repair_in_fences: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            footnotes: FootnotesMode::SingleBlock,
            reference_definitions: ReferenceDefinitionsMode::Invalidate,
            terminator: TerminatorOptions::default(),
            terminator_window_bytes: 16 * 1024,
            max_buffer_bytes: None,
            json_repair_in_fences: false,
        }
    }
}

