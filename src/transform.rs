use crate::types::BlockKind;

#[derive(Debug, Clone, Copy)]
pub struct PendingTransformInput<'a> {
    pub kind: BlockKind,
    pub raw: &'a str,
    pub display: &'a str,
}

pub trait PendingTransformer: Send + Sync {
    /// Transform the pending display string.
    ///
    /// - `kind` is a best-effort hint (block-level).
    /// - `raw` is the original pending text (never mutated).
    /// - `display` is the current pending display string (already includes built-in termination/repair).
    ///
    /// Return `Some(new_display)` to replace `display`, or `None` to leave it unchanged.
    fn transform(&self, input: PendingTransformInput<'_>) -> Option<String>;

    fn reset(&self) {}
}

pub struct FnPendingTransformer<F>(pub F);

impl<F> PendingTransformer for FnPendingTransformer<F>
where
    for<'a> F: Fn(PendingTransformInput<'a>) -> Option<String> + Send + Sync,
{
    fn transform(&self, input: PendingTransformInput<'_>) -> Option<String> {
        (self.0)(input)
    }
}
