use crate::types::BlockKind;

#[derive(Debug, Clone, Copy)]
pub struct PendingTransformInput<'a> {
    pub kind: BlockKind,
    pub raw: &'a str,
    pub display: &'a str,
}

#[cfg(feature = "sync")]
pub trait PendingTransformer: Send + Sync {
    /// Transform the pending display string.
    ///
    /// - `kind` is a best-effort hint (block-level).
    /// - `raw` is the original pending text (never mutated).
    /// - `display` is the current pending display string (already includes built-in termination/repair).
    ///
    /// Return `Some(new_display)` to replace `display`, or `None` to leave it unchanged.
    fn transform(&mut self, input: PendingTransformInput<'_>) -> Option<String>;

    fn reset(&mut self) {}
}

#[cfg(not(feature = "sync"))]
pub trait PendingTransformer: Send {
    /// Transform the pending display string.
    ///
    /// - `kind` is a best-effort hint (block-level).
    /// - `raw` is the original pending text (never mutated).
    /// - `display` is the current pending display string (already includes built-in termination/repair).
    ///
    /// Return `Some(new_display)` to replace `display`, or `None` to leave it unchanged.
    fn transform(&mut self, input: PendingTransformInput<'_>) -> Option<String>;

    fn reset(&mut self) {}
}

pub struct FnPendingTransformer<F>(pub F);

#[cfg(not(feature = "sync"))]
impl<F> PendingTransformer for FnPendingTransformer<F>
where
    for<'a> F: FnMut(PendingTransformInput<'a>) -> Option<String> + Send,
{
    fn transform(&mut self, input: PendingTransformInput<'_>) -> Option<String> {
        (self.0)(input)
    }
}

#[cfg(feature = "sync")]
impl<F> PendingTransformer for FnPendingTransformer<F>
where
    for<'a> F: FnMut(PendingTransformInput<'a>) -> Option<String> + Send + Sync,
{
    fn transform(&mut self, input: PendingTransformInput<'_>) -> Option<String> {
        (self.0)(input)
    }
}

fn tail_window(text: &str, window_bytes: usize) -> (&str, usize) {
    if text.len() <= window_bytes {
        return (text, 0);
    }
    let start = text.len() - window_bytes;
    let mut s = start;
    while !text.is_char_boundary(s) {
        s += 1;
    }
    (&text[s..], s)
}

#[derive(Debug, Clone)]
pub struct IncompleteLinkPlaceholderTransformer {
    pub incomplete_link_url: String,
    pub window_bytes: usize,
}

impl Default for IncompleteLinkPlaceholderTransformer {
    fn default() -> Self {
        Self {
            incomplete_link_url: "streamdown:incomplete-link".to_string(),
            window_bytes: 16 * 1024,
        }
    }
}

impl PendingTransformer for IncompleteLinkPlaceholderTransformer {
    fn transform(&mut self, input: PendingTransformInput<'_>) -> Option<String> {
        // Avoid touching code fences entirely.
        if matches!(input.kind, BlockKind::CodeFence) {
            return None;
        }
        let (window, offset) = tail_window(input.display, self.window_bytes);
        let fixed = crate::pending::fix_incomplete_link_or_image(
            window,
            &self.incomplete_link_url,
            true,
            false,
        )?;
        if fixed == window {
            return None;
        }
        let mut out = String::with_capacity(offset + fixed.len());
        out.push_str(&input.display[..offset]);
        out.push_str(&fixed);
        Some(out)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IncompleteImageDropTransformer {
    pub window_bytes: usize,
}

impl Default for IncompleteImageDropTransformer {
    fn default() -> Self {
        Self {
            window_bytes: 16 * 1024,
        }
    }
}

impl PendingTransformer for IncompleteImageDropTransformer {
    fn transform(&mut self, input: PendingTransformInput<'_>) -> Option<String> {
        if matches!(input.kind, BlockKind::CodeFence) {
            return None;
        }
        let (window, offset) = tail_window(input.display, self.window_bytes);
        let fixed = crate::pending::fix_incomplete_link_or_image(window, "", false, true)?;
        if fixed == window {
            return None;
        }
        let mut out = String::with_capacity(offset + fixed.len());
        out.push_str(&input.display[..offset]);
        out.push_str(&fixed);
        Some(out)
    }
}
