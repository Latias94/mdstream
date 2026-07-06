mod context;
mod emphasis;
mod inline;
mod links;
mod setext;

use crate::syntax::facts::tail_window;

use super::TerminatorOptions;
use context::is_inside_incomplete_multiline_code_block;
use emphasis::{
    handle_incomplete_bold, handle_incomplete_bold_italic,
    handle_incomplete_double_underscore_italic, handle_incomplete_single_asterisk_italic,
    handle_incomplete_single_underscore_italic,
};
use inline::{balance_inline_code, balance_katex_block, balance_strikethrough};
pub(crate) use links::fix_incomplete_link_or_image;
use setext::{apply_setext_heading_protection, trim_trailing_single_space};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PendingRepair<'a> {
    opts: &'a TerminatorOptions,
}

impl<'a> PendingRepair<'a> {
    pub(crate) fn new(opts: &'a TerminatorOptions) -> Self {
        Self { opts }
    }

    pub(crate) fn apply(&self, text: &str) -> String {
        let opts = self.opts;
        if text.is_empty() {
            return String::new();
        }

        let text = trim_trailing_single_space(text);
        let (window, offset) = tail_window(text, opts.window_bytes);

        // Work on the tail window but keep a stable prefix.
        let prefix = &text[..offset];
        let mut tail = window.to_string();

        if opts.setext_headings {
            tail = apply_setext_heading_protection(&tail);
        }

        if is_inside_incomplete_multiline_code_block(&tail) {
            // If the tail is currently inside an unclosed fenced code block, avoid other termination.
            let mut out = String::with_capacity(prefix.len() + tail.len());
            out.push_str(prefix);
            out.push_str(&tail);
            return out;
        }

        if opts.links || opts.images {
            if let Some(processed) = fix_incomplete_link_or_image(
                &tail,
                &opts.incomplete_link_url,
                opts.links,
                opts.images,
            ) {
                if processed.ends_with(&format!("]({})", opts.incomplete_link_url)) {
                    let mut out = String::with_capacity(prefix.len() + processed.len());
                    out.push_str(prefix);
                    out.push_str(&processed);
                    return out;
                }
                tail = processed;
            }
        }

        if opts.emphasis {
            tail = handle_incomplete_bold_italic(&tail);
            tail = handle_incomplete_bold(&tail);
            tail = handle_incomplete_double_underscore_italic(&tail);
            tail = handle_incomplete_single_asterisk_italic(&tail);
            tail = handle_incomplete_single_underscore_italic(&tail);
        }
        if opts.inline_code {
            tail = balance_inline_code(&tail);
        }
        if opts.strikethrough {
            tail = balance_strikethrough(&tail);
        }
        if opts.katex_block {
            tail = balance_katex_block(&tail);
        }

        let mut out = String::with_capacity(prefix.len() + tail.len());
        out.push_str(prefix);
        out.push_str(&tail);
        out
    }
}
