mod budget;
mod definition_topology;
mod diagnostic;
mod frame;
mod limits;
mod normalization;
mod parser;
mod unresolved_footnotes;

#[cfg(test)]
mod tests;

pub(crate) use budget::DraftUsage;
pub use diagnostic::MarkdownDiagnostic;
pub(crate) use diagnostic::MarkdownError;
pub(crate) use limits::validate_draft_limits;
pub(crate) use parser::{MarkdownConfig, compile_markdown_with_custom};
