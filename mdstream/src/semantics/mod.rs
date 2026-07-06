use std::collections::{HashMap, HashSet};

use crate::options::ReferenceDefinitionsMode;
use crate::reference::{extract_reference_definition_label, normalize_reference_label};
use crate::types::{Block, BlockId, BlockKind};

#[derive(Debug, Default)]
pub(crate) struct DocumentSemantics {
    footnotes_detected: bool,
    footnote_scan_tail: String,
    reference_usage_index: HashMap<String, HashSet<BlockId>>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CommitEffects {
    pub(crate) invalidated: Vec<BlockId>,
}

impl DocumentSemantics {
    pub(crate) fn footnotes_detected(&self) -> bool {
        self.footnotes_detected
    }

    pub(crate) fn observe_chunk_for_footnotes(&mut self, chunk: &str) {
        if self.footnotes_detected {
            return;
        }
        if detect_footnotes(chunk) {
            self.footnotes_detected = true;
            return;
        }

        // Keep a small tail window to detect patterns across chunk boundaries.
        const MAX_TAIL: usize = 256;
        let chunk_prefix = take_prefix_at_char_boundary(chunk, MAX_TAIL);
        if !self.footnote_scan_tail.is_empty() && !chunk_prefix.is_empty() {
            let mut combined =
                String::with_capacity(self.footnote_scan_tail.len() + chunk_prefix.len());
            combined.push_str(&self.footnote_scan_tail);
            combined.push_str(chunk_prefix);
            if detect_footnotes(&combined) {
                self.footnotes_detected = true;
            }
        }
        if !self.footnotes_detected {
            update_tail(&mut self.footnote_scan_tail, chunk, MAX_TAIL);
        }
    }

    pub(crate) fn observe_committed_block(
        &mut self,
        block: &Block,
        reference_mode: ReferenceDefinitionsMode,
    ) -> CommitEffects {
        self.index_reference_usages(block);
        let invalidated = self.invalidated_by_reference_definitions(block, reference_mode);
        CommitEffects { invalidated }
    }

    pub(crate) fn clear_references(&mut self) {
        self.reference_usage_index.clear();
    }

    pub(crate) fn reset(&mut self) {
        self.footnotes_detected = false;
        self.footnote_scan_tail.clear();
        self.reference_usage_index.clear();
    }

    fn index_reference_usages(&mut self, block: &Block) {
        if block.kind == BlockKind::CodeFence || !block.raw.contains('[') {
            return;
        }
        let used = extract_reference_usages(&block.raw);
        for label in used {
            self.reference_usage_index
                .entry(label)
                .or_default()
                .insert(block.id);
        }
    }

    fn invalidated_by_reference_definitions(
        &self,
        block: &Block,
        reference_mode: ReferenceDefinitionsMode,
    ) -> Vec<BlockId> {
        if reference_mode != ReferenceDefinitionsMode::Invalidate
            || block.kind == BlockKind::CodeFence
            || !block.raw.contains("]:")
        {
            return Vec::new();
        }

        let mut invalidated = HashSet::new();
        for line in block.raw.split('\n') {
            let Some(label) = extract_reference_definition_label(line) else {
                continue;
            };
            if let Some(ids) = self.reference_usage_index.get(&label) {
                for id in ids {
                    if *id != block.id {
                        invalidated.insert(*id);
                    }
                }
            }
        }
        let mut ids: Vec<BlockId> = invalidated.into_iter().collect();
        ids.sort_by_key(|id| id.0);
        ids
    }
}

pub(crate) fn extract_reference_usages(text: &str) -> HashSet<String> {
    // Best-effort extractor for reference-style link labels:
    // - [text][label]
    // - [label][]
    // - [label] (shortcut)
    //
    // We intentionally over-approximate: false positives only cause extra invalidations.
    let bytes = text.as_bytes();
    let mut out = HashSet::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let mut close1 = i + 1;
        while close1 < bytes.len() && bytes[close1] != b']' {
            close1 += 1;
        }
        if close1 >= bytes.len() {
            break;
        }
        let label1 = &text[i + 1..close1];
        // Skip footnote-ish labels.
        if label1.as_bytes().first() == Some(&b'^') {
            i = close1 + 1;
            continue;
        }

        // Inline links/images: [text](...) / ![alt](...)
        if bytes.get(close1 + 1) == Some(&b'(') {
            i = close1 + 1;
            continue;
        }
        // Definition: [label]: ...
        if bytes.get(close1 + 1) == Some(&b':') {
            i = close1 + 1;
            continue;
        }

        // Reference form: [text][label] or [label][]
        if bytes.get(close1 + 1) == Some(&b'[') {
            let start2 = close1 + 2;
            if start2 >= bytes.len() {
                break;
            }
            let mut close2 = start2;
            while close2 < bytes.len() && bytes[close2] != b']' {
                close2 += 1;
            }
            if close2 >= bytes.len() {
                break;
            }
            let label2 = &text[start2..close2];
            let chosen = if label2.trim().is_empty() {
                label1
            } else {
                label2
            };
            if let Some(norm) = normalize_reference_label(chosen) {
                out.insert(norm);
            }
            i = close2 + 1;
            continue;
        }

        // Shortcut reference: [label]
        if let Some(norm) = normalize_reference_label(label1) {
            out.insert(norm);
        }
        i = close1 + 1;
    }
    out
}

pub(crate) fn detect_footnotes(text: &str) -> bool {
    // Very small, streaming-friendly detector:
    // - references: [^id] (not followed by :)
    // - definitions: [^id]:
    //
    // Compatibility notes:
    // - Align with Streamdown/Incremark: identifiers must not contain whitespace, and must be non-empty.
    // - Keep a conservative identifier length cap to avoid pathological scans.
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'^' {
            const MAX_ID_LEN: usize = 200;
            // Find closing `]` while validating identifier rules.
            let mut j = i + 2;
            let mut id_len = 0usize;
            while j < bytes.len() {
                let b = bytes[j];
                if b == b']' {
                    break;
                }
                if b == b'\n' || b == b'\r' || b == b' ' || b == b'\t' {
                    // Invalid footnote identifier; do not treat as footnote.
                    id_len = 0;
                    break;
                }
                id_len += 1;
                if id_len > MAX_ID_LEN {
                    id_len = 0;
                    break;
                }
                j += 1;
            }
            if id_len > 0 && j < bytes.len() && bytes[j] == b']' {
                // Either a reference (`[^id]`) or a definition (`[^id]:`) should trigger single-block mode.
                return true;
            }
        }
        i += 1;
    }
    false
}

fn take_prefix_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn take_suffix_at_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn update_tail(tail: &mut String, chunk: &str, max_bytes: usize) {
    if chunk.is_empty() {
        return;
    }
    if chunk.len() >= max_bytes {
        *tail = take_suffix_at_char_boundary(chunk, max_bytes).to_string();
        return;
    }
    if tail.len() + chunk.len() <= max_bytes {
        tail.push_str(chunk);
        return;
    }
    let mut combined = String::with_capacity(max_bytes + 4);
    combined.push_str(tail);
    combined.push_str(chunk);
    *tail = take_suffix_at_char_boundary(&combined, max_bytes).to_string();
}
