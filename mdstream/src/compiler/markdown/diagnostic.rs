use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MarkdownDiagnostic {
    CursorOverflow,
    InvalidRange {
        start: usize,
        end: usize,
    },
    InvalidUtf8Boundary {
        start: usize,
        end: usize,
    },
    InvalidDelimiterRange {
        marker: char,
        start: usize,
        end: usize,
    },
    InvalidListStart(u64),
    InvalidCodeFence,
    InvalidCustomAttributeName,
    InvalidCustomAttributeValue,
    DuplicateCustomAttribute,
    LimitExceeded {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    NumericOverflow(&'static str),
    ResourceOverflow,
    StackMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    UnexpectedEvent {
        event: &'static str,
        context: &'static str,
    },
    UnclosedContainer(&'static str),
    Unsupported(&'static str),
}

impl fmt::Display for MarkdownDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CursorOverflow => formatter.write_str("Markdown source cursor overflow"),
            Self::InvalidRange { start, end } => {
                write!(formatter, "invalid Markdown source range {start}..{end}")
            }
            Self::InvalidUtf8Boundary { start, end } => write!(
                formatter,
                "Markdown source range {start}..{end} is not on UTF-8 boundaries"
            ),
            Self::InvalidDelimiterRange { marker, start, end } => write!(
                formatter,
                "Markdown {marker} delimiters do not match source range {start}..{end}"
            ),
            Self::InvalidListStart(start) => {
                write!(
                    formatter,
                    "ordered-list start {start} exceeds the protocol range"
                )
            }
            Self::InvalidCodeFence => formatter.write_str("invalid fenced-code source range"),
            Self::InvalidCustomAttributeName => {
                formatter.write_str("invalid custom-block attribute name")
            }
            Self::InvalidCustomAttributeValue => {
                formatter.write_str("invalid custom-block attribute value")
            }
            Self::DuplicateCustomAttribute => {
                formatter.write_str("duplicate custom-block attribute")
            }
            Self::LimitExceeded {
                field,
                limit,
                actual,
            } => write!(
                formatter,
                "Markdown {field} {actual} exceeds the configured limit of {limit}"
            ),
            Self::NumericOverflow(field) => write!(formatter, "Markdown {field} overflow"),
            Self::ResourceOverflow => formatter.write_str("too many Markdown resources"),
            Self::StackMismatch { expected, actual } => {
                write!(
                    formatter,
                    "Markdown stack mismatch: expected {expected}, got {actual}"
                )
            }
            Self::UnexpectedEvent { event, context } => {
                write!(formatter, "unexpected Markdown event {event} in {context}")
            }
            Self::UnclosedContainer(kind) => {
                write!(formatter, "unclosed Markdown container {kind}")
            }
            Self::Unsupported(kind) => write!(formatter, "unsupported Markdown construct {kind}"),
        }
    }
}

impl std::error::Error for MarkdownDiagnostic {}

pub(crate) type MarkdownError = MarkdownDiagnostic;
