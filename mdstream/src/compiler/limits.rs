/// Work budgets owned by the content compiler rather than the Content IR protocol.
///
/// These limits constrain implementation-specific parsing and semantic state.
/// They are kept separate from [`mdstream_protocol::ProtocolLimits`] so protocol
/// consumers and reducers do not need to understand how a compiler produces a
/// change stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompilerLimits {
    /// Maximum parser events retained for one Markdown classification pass.
    pub max_markdown_events: usize,
    /// Maximum candidate/event intersections inspected while classifying footnotes.
    pub max_markdown_overlap_work: usize,
    /// Maximum definitions retained by the compiler's semantic registry.
    pub max_definitions: usize,
    /// Maximum reverse dependency edges retained for semantic correction.
    pub max_definition_edges: usize,
    /// Maximum UTF-8 bytes retained by definition keys and values.
    pub max_definition_metadata_bytes: usize,
}

impl Default for CompilerLimits {
    fn default() -> Self {
        Self {
            max_markdown_events: 300_000,
            max_markdown_overlap_work: 1_000_000,
            max_definitions: 100_000,
            max_definition_edges: 100_000,
            max_definition_metadata_bytes: 16 * 1024 * 1024,
        }
    }
}
