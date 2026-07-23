use std::fmt;

use crate::SplitSafety;

/// Error returned by [`crate::StreamEngine::append_bytes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendBytesError {
    RawInputTooLarge { limit: usize, actual: usize },
    InvalidUtf8(std::str::Utf8Error),
    Engine(super::EngineError),
}

impl AppendBytesError {
    pub const fn split_safety(&self) -> SplitSafety {
        match self {
            Self::Engine(error) => error.split_safety(),
            Self::RawInputTooLarge { .. } | Self::InvalidUtf8(_) => SplitSafety::NotSafe,
        }
    }
}

impl fmt::Display for AppendBytesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RawInputTooLarge { limit, actual } => write!(
                formatter,
                "raw append input uses {actual} bytes, conservative limit is {limit}"
            ),
            Self::InvalidUtf8(error) => write!(formatter, "append input is not UTF-8: {error}"),
            Self::Engine(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AppendBytesError {}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct NewlineNormalizer {
    pending_cr: bool,
}

impl NewlineNormalizer {
    /// Counts canonical bytes after appending `chunk` without materializing them.
    /// Returns `None` only when the count cannot fit in `usize`.
    pub(super) fn projected_canonical_bytes(self, chunk: &str) -> Option<usize> {
        let mut bytes = 0_usize;
        let mut pending_cr = self.pending_cr;

        for byte in chunk.bytes() {
            if pending_cr {
                bytes = bytes.checked_add(1)?;
                pending_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }

            if byte == b'\r' {
                pending_cr = true;
            } else {
                bytes = bytes.checked_add(1)?;
            }
        }

        if pending_cr {
            bytes.checked_add(1)
        } else {
            Some(bytes)
        }
    }

    pub(super) fn pending_bytes(self) -> usize {
        usize::from(self.pending_cr)
    }

    pub(super) fn raw_append_byte_ceiling(self, remaining_source_bytes: usize) -> usize {
        let ceiling = remaining_source_bytes.saturating_mul(2);
        if self.pending_cr {
            ceiling.saturating_sub(1)
        } else {
            ceiling
        }
    }

    pub(super) fn append(self, chunk: &str) -> (Self, String) {
        if chunk.is_empty() {
            return (self, String::new());
        }

        let mut next = self;
        let mut output = String::with_capacity(chunk.len());
        let mut characters = chunk.chars().peekable();

        if next.pending_cr {
            if characters.peek() == Some(&'\n') {
                characters.next();
            }
            output.push('\n');
            next.pending_cr = false;
        }

        while let Some(character) = characters.next() {
            if character != '\r' {
                output.push(character);
                continue;
            }
            if characters.peek() == Some(&'\n') {
                characters.next();
                output.push('\n');
            } else if characters.peek().is_none() {
                next.pending_cr = true;
            } else {
                output.push('\n');
            }
        }

        (next, output)
    }

    pub(super) fn finish(self) -> (Self, String) {
        if self.pending_cr {
            (Self::default(), "\n".to_string())
        } else {
            (self, String::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NewlineNormalizer;
    use proptest::{collection::vec, prelude::*};

    #[test]
    fn projected_bytes_match_pending_and_emitted_bytes_across_chunks() {
        let normalizer = NewlineNormalizer::default();
        assert_eq!(normalizer.projected_canonical_bytes("é\r"), Some(3));

        let (normalizer, suffix) = normalizer.append("é\r");
        assert_eq!(suffix, "é");
        assert_eq!(normalizer.pending_bytes(), 1);
        assert_eq!(normalizer.projected_canonical_bytes("\n中"), Some(4));

        let (next, suffix) = normalizer.append("\n中");
        assert_eq!(suffix, "\n中");
        assert_eq!(next.pending_bytes(), 0);
        assert_eq!(
            normalizer.projected_canonical_bytes("\n中"),
            Some(suffix.len() + next.pending_bytes())
        );
    }

    #[test]
    fn projected_bytes_count_cr_and_crlf_as_one_byte() {
        let normalizer = NewlineNormalizer::default();
        assert_eq!(normalizer.projected_canonical_bytes("\r\n\rX\r"), Some(4));

        let (next, suffix) = normalizer.append("\r\n\rX\r");
        assert_eq!(suffix, "\n\nX");
        assert_eq!(suffix.len() + next.pending_bytes(), 4);
    }

    #[test]
    fn raw_ceiling_preserves_every_possible_crlf_normalization() {
        let normalizer = NewlineNormalizer::default();
        assert_eq!(normalizer.raw_append_byte_ceiling(0), 0);
        assert_eq!(normalizer.raw_append_byte_ceiling(4), 8);

        let (pending, _) = normalizer.append("a\r");
        assert_eq!(pending.raw_append_byte_ceiling(0), 0);
        assert_eq!(pending.raw_append_byte_ceiling(3), 5);
    }

    proptest! {
        #[test]
        fn projected_bytes_match_arbitrary_unicode_chunk_sequences(
            chunks in vec(any::<String>(), 0..32),
        ) {
            let mut normalizer = NewlineNormalizer::default();
            for chunk in chunks {
                let projected = normalizer.projected_canonical_bytes(&chunk);
                let (next, suffix) = normalizer.append(&chunk);
                prop_assert_eq!(
                    projected,
                    suffix.len().checked_add(next.pending_bytes()),
                );
                normalizer = next;
            }
        }
    }
}
