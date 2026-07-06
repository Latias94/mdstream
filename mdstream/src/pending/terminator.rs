#[derive(Debug, Clone)]
pub struct TerminatorOptions {
    pub setext_headings: bool,
    pub links: bool,
    pub images: bool,
    pub emphasis: bool,
    pub inline_code: bool,
    pub strikethrough: bool,
    pub katex_block: bool,
    pub incomplete_link_url: String,
    /// Tail-only scan window for termination logic.
    pub window_bytes: usize,
}

impl Default for TerminatorOptions {
    fn default() -> Self {
        Self {
            setext_headings: true,
            links: true,
            images: true,
            emphasis: true,
            inline_code: true,
            strikethrough: true,
            katex_block: true,
            incomplete_link_url: "streamdown:incomplete-link".to_string(),
            window_bytes: 16 * 1024,
        }
    }
}

/// Terminate a streaming Markdown tail to avoid partial rendering artifacts.
///
/// This function is intentionally conservative and only modifies the pending tail.
pub fn terminate_markdown(text: &str, opts: &TerminatorOptions) -> String {
    super::repair::PendingRepair::new(opts).apply(text)
}
