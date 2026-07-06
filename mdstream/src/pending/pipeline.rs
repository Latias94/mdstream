use crate::syntax::facts::code_fence_suffix;
use crate::transform::{PendingTransformInput, PendingTransformer};
use crate::types::BlockKind;

use super::{TerminatorOptions, terminate_markdown};

#[derive(Default)]
pub(crate) struct PendingDisplayPipeline {
    cache: Option<String>,
    code_fence_suffix: Option<String>,
}

impl std::fmt::Debug for PendingDisplayPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingDisplayPipeline")
            .field("cached", &self.is_cached())
            .field("code_fence_fast_path", &self.has_code_fence_fast_path())
            .finish()
    }
}

impl PendingDisplayPipeline {
    pub(crate) fn clear(&mut self) {
        self.cache = None;
        self.code_fence_suffix = None;
    }

    pub(crate) fn display(&self) -> Option<&str> {
        self.cache.as_deref()
    }

    pub(crate) fn is_cached(&self) -> bool {
        self.cache.is_some()
    }

    pub(crate) fn has_code_fence_fast_path(&self) -> bool {
        self.code_fence_suffix.is_some()
    }

    pub(crate) fn set_owned_display(&mut self, display: String) {
        self.cache = Some(display);
        self.code_fence_suffix = None;
    }

    pub(crate) fn ensure_for(
        &mut self,
        kind: BlockKind,
        raw: &str,
        code_fence: Option<(char, usize)>,
        terminator: &TerminatorOptions,
        transformers: &mut [Box<dyn PendingTransformer>],
    ) {
        if matches!(kind, BlockKind::CodeFence) {
            if let Some((fence_char, fence_len)) = code_fence {
                if self.cache.is_some() && self.code_fence_suffix.is_some() {
                    return;
                }
                let suffix = code_fence_suffix(raw.ends_with('\n'), fence_char, fence_len);
                let mut display = String::with_capacity(raw.len() + suffix.len());
                display.push_str(raw);
                display.push_str(&suffix);
                self.cache = Some(display);
                self.code_fence_suffix = Some(suffix);
                return;
            }
        }

        if self.cache.is_some() {
            return;
        }
        let display = terminate_markdown(raw, terminator);
        let display = transform_pending_display(kind, raw, display, transformers);
        self.cache = Some(display);
        self.code_fence_suffix = None;
    }

    pub(crate) fn try_incremental_code_fence_append(
        &mut self,
        appended: &str,
        code_fence: Option<(char, usize)>,
    ) -> bool {
        let Some(suffix) = self.code_fence_suffix.as_ref() else {
            return false;
        };
        let Some(display) = self.cache.as_mut() else {
            self.code_fence_suffix = None;
            return false;
        };
        let Some((fence_char, fence_len)) = code_fence else {
            self.clear();
            return false;
        };

        let prev_raw_ended_with_nl = !suffix.starts_with('\n');
        let new_raw_ended_with_nl = if appended.is_empty() {
            prev_raw_ended_with_nl
        } else {
            appended.ends_with('\n')
        };

        let base_len = display.len().saturating_sub(suffix.len());
        display.truncate(base_len);
        display.push_str(appended);

        let new_suffix = code_fence_suffix(new_raw_ended_with_nl, fence_char, fence_len);
        display.push_str(&new_suffix);
        self.code_fence_suffix = Some(new_suffix);
        true
    }
}

pub(crate) fn render_pending_display(
    kind: BlockKind,
    raw: &str,
    terminator: &TerminatorOptions,
    transformers: &mut [Box<dyn PendingTransformer>],
) -> String {
    let display = terminate_markdown(raw, terminator);
    transform_pending_display(kind, raw, display, transformers)
}

fn transform_pending_display(
    kind: BlockKind,
    raw: &str,
    mut display: String,
    transformers: &mut [Box<dyn PendingTransformer>],
) -> String {
    for transformer in transformers {
        if let Some(next) = transformer.transform(PendingTransformInput {
            kind,
            raw,
            display: &display,
        }) {
            display = next;
        }
    }
    display
}
