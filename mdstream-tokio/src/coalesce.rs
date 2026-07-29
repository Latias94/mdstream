use std::collections::VecDeque;

use tokio::time::Instant;

use crate::CoalesceOptions;
use crate::stats::{CoalesceWork, FlushReason};

/// Original non-empty chunks accepted locally but not committed downstream.
#[derive(Debug, Default)]
pub struct PendingInput {
    chunks: VecDeque<String>,
    bytes: usize,
}

impl PendingInput {
    pub(crate) fn from_pending(pending: PendingChunks) -> Self {
        let bytes = pending.bytes();
        Self {
            chunks: pending.into_texts(),
            bytes,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn chunks(&self) -> impl ExactSizeIterator<Item = &str> + DoubleEndedIterator {
        self.chunks.iter().map(String::as_str)
    }

    pub fn into_chunks(self) -> VecDeque<String> {
        self.chunks
    }
}

#[derive(Debug)]
pub(crate) struct ScannedChunk {
    text: String,
    has_newline: bool,
}

impl ScannedChunk {
    pub(crate) fn scan(text: String, work: &mut CoalesceWork) -> Self {
        let (has_newline, scanned_bytes) = scan_newline(text.as_bytes());
        work.record_input(scanned_bytes);
        Self { text, has_newline }
    }

    pub(crate) fn scan_without_recording(text: String, has_newline: bool) -> Self {
        Self { text, has_newline }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.text.len()
    }

    pub(crate) fn into_text(self) -> String {
        self.text
    }

    pub(crate) const fn boundary_metadata_bytes() -> usize {
        std::mem::size_of::<Self>()
    }
}

pub(crate) fn scan_newline(text: &[u8]) -> (bool, usize) {
    match text.iter().position(|byte| *byte == b'\n') {
        Some(index) => (true, index.saturating_add(1)),
        None => (false, text.len()),
    }
}

#[derive(Debug, Default)]
pub(crate) struct PendingChunks {
    chunks: VecDeque<ScannedChunk>,
    bytes: usize,
    messages: usize,
    newline_chunks: usize,
    started_at: Option<Instant>,
}

impl PendingChunks {
    pub(crate) fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn constituents(&self) -> usize {
        self.chunks.len()
    }

    /// Logical bytes occupied by live boundary records. Allocator spare
    /// capacity is deliberately excluded so the counter is deterministic.
    pub(crate) fn boundary_metadata_bytes(&self) -> usize {
        self.constituents()
            .saturating_mul(ScannedChunk::boundary_metadata_bytes())
    }

    pub(crate) fn deadline(&self, options: CoalesceOptions) -> Option<Instant> {
        self.started_at
            .and_then(|start| start.checked_add(options.max_delay()))
    }

    pub(crate) fn overflow_reason(
        &self,
        chunk: &ScannedChunk,
        options: CoalesceOptions,
    ) -> Option<FlushReason> {
        if self.is_empty() || chunk.is_empty() {
            return None;
        }
        if self.bytes.saturating_add(chunk.len()) > options.max_bytes() {
            return Some(FlushReason::MaxBytes);
        }
        if self.constituents().saturating_add(1) > options.max_pending_chunks() {
            return Some(FlushReason::MaxPendingChunks);
        }
        None
    }

    pub(crate) fn accept(&mut self, chunk: ScannedChunk, now: Instant) {
        self.messages = self.messages.saturating_add(1);
        if chunk.is_empty() {
            return;
        }
        if self.chunks.is_empty() {
            self.started_at = Some(now);
        }
        self.bytes = self.bytes.saturating_add(chunk.len());
        self.newline_chunks = self
            .newline_chunks
            .saturating_add(usize::from(chunk.has_newline));
        self.chunks.push_back(chunk);
    }

    pub(crate) fn flush_reason(&self, options: CoalesceOptions) -> Option<FlushReason> {
        if self.is_empty() {
            return None;
        }
        if self.bytes >= options.max_bytes() {
            return Some(FlushReason::MaxBytes);
        }
        if self.constituents() >= options.max_pending_chunks() {
            return Some(FlushReason::MaxPendingChunks);
        }
        if options.flush_on_newline() && self.newline_chunks != 0 {
            return Some(FlushReason::Newline);
        }
        None
    }

    pub(crate) fn take_text(&mut self, work: &mut CoalesceWork) -> (String, usize) {
        let messages = self.messages;
        let text = if self.chunks.len() == 1 {
            self.chunks
                .pop_front()
                .expect("one pending chunk must exist")
                .into_text()
        } else {
            let mut joined = String::with_capacity(self.bytes);
            for chunk in self.chunks.drain(..) {
                joined.push_str(&chunk.text);
            }
            work.record_join_copy(self.bytes);
            joined
        };
        self.clear_facts();
        (text, messages)
    }

    pub(crate) fn front(&self) -> Option<&str> {
        self.chunks.front().map(|chunk| chunk.text.as_str())
    }

    pub(crate) fn commit_front(&mut self) -> String {
        let chunk = self
            .chunks
            .pop_front()
            .expect("a committed pending chunk must exist");
        self.bytes = self.bytes.saturating_sub(chunk.len());
        self.newline_chunks = self
            .newline_chunks
            .saturating_sub(usize::from(chunk.has_newline));
        if self.chunks.is_empty() {
            self.clear_facts();
        }
        chunk.into_text()
    }

    pub(crate) fn into_texts(self) -> VecDeque<String> {
        self.chunks
            .into_iter()
            .map(ScannedChunk::into_text)
            .collect()
    }

    pub(crate) fn clear_empty_messages(&mut self) {
        if self.is_empty() {
            self.messages = 0;
        }
    }

    fn clear_facts(&mut self) {
        self.bytes = 0;
        self.messages = 0;
        self.newline_chunks = 0;
        self.started_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_owned_chunk_keeps_its_allocation() {
        let mut work = CoalesceWork::default();
        let text = String::from("owned");
        let pointer = text.as_ptr();
        let mut pending = PendingChunks::default();
        pending.accept(ScannedChunk::scan(text, &mut work), Instant::now());

        let (joined, messages) = pending.take_text(&mut work);

        assert_eq!(joined.as_ptr(), pointer);
        assert_eq!(messages, 1);
        assert_eq!(work.join_copy_bytes, 0);
    }

    #[test]
    fn newline_scan_stops_after_the_first_match() {
        assert_eq!(scan_newline(b"\nrest"), (true, 1));
        assert_eq!(scan_newline(b"ab\nrest"), (true, 3));
        assert_eq!(scan_newline(b"rest"), (false, 4));
    }

    #[test]
    fn scanning_one_byte_chunks_is_linear() {
        let mut previous_scan_bytes = 0;
        for size in [64, 128, 256, 512, 1024, 2048, 4096] {
            let mut work = CoalesceWork::default();
            let mut pending = PendingChunks::default();
            let now = Instant::now();
            for _ in 0..size {
                pending.accept(ScannedChunk::scan("x".to_string(), &mut work), now);
            }

            assert_eq!(work.scan_bytes, size);
            assert_eq!(pending.bytes(), usize::try_from(size).unwrap());
            assert_eq!(pending.constituents(), usize::try_from(size).unwrap());
            assert_eq!(
                pending.boundary_metadata_bytes(),
                usize::try_from(size)
                    .unwrap()
                    .saturating_mul(ScannedChunk::boundary_metadata_bytes())
            );
            if previous_scan_bytes != 0 {
                assert_eq!(work.scan_bytes, previous_scan_bytes * 2);
            }
            previous_scan_bytes = work.scan_bytes;
        }
    }
}
