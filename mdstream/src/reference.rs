use std::collections::{BTreeMap, HashMap, HashSet};

use crate::options::ReferenceDefinitionsMode;
use crate::syntax::facts::strip_up_to_three_leading_spaces;
use crate::types::{Block, BlockId, BlockKind};

#[derive(Debug, Default)]
pub(crate) struct ReferenceIndex {
    usage_index: HashMap<String, HashSet<BlockId>>,
}

impl ReferenceIndex {
    pub(crate) fn observe_committed_block(
        &mut self,
        block: &Block,
        reference_mode: ReferenceDefinitionsMode,
    ) -> Vec<BlockId> {
        self.index_usages(block);
        self.invalidated_by_definitions(block, reference_mode)
    }

    pub(crate) fn clear(&mut self) {
        self.usage_index.clear();
    }

    fn index_usages(&mut self, block: &Block) {
        if block.kind == BlockKind::CodeFence || !block.raw.contains('[') {
            return;
        }
        let used = extract_reference_usages(&block.raw);
        for label in used {
            self.usage_index.entry(label).or_default().insert(block.id);
        }
    }

    fn invalidated_by_definitions(
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
            if let Some(ids) = self.usage_index.get(&label) {
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

#[derive(Debug, Default)]
pub(crate) struct ReferenceDefinitions {
    definitions: BTreeMap<String, String>,
    prelude_text: String,
    dirty: bool,
}

impl ReferenceDefinitions {
    pub(crate) fn clear(&mut self) {
        self.definitions.clear();
        self.prelude_text.clear();
        self.dirty = false;
    }

    pub(crate) fn observe_text(&mut self, raw: &str) {
        for line in raw.split('\n') {
            let Some((label, def_line)) = extract_reference_definition_line(line) else {
                continue;
            };
            match self.definitions.entry(label) {
                std::collections::btree_map::Entry::Vacant(v) => {
                    v.insert(def_line);
                    self.dirty = true;
                }
                std::collections::btree_map::Entry::Occupied(mut o) => {
                    if o.get() != &def_line {
                        o.insert(def_line);
                        self.dirty = true;
                    }
                }
            }
        }
    }

    pub(crate) fn refresh(&mut self) {
        if !self.dirty {
            return;
        }
        self.prelude_text = self
            .definitions
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        self.dirty = false;
    }

    pub(crate) fn prelude_text(&self) -> &str {
        &self.prelude_text
    }
}

pub(crate) fn normalize_reference_label(label: &str) -> Option<String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() > 200 {
        return None;
    }
    let mut out = String::with_capacity(trimmed.len());
    let mut last_was_ws = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            last_was_ws = true;
            continue;
        }
        if last_was_ws && !out.is_empty() {
            out.push(' ');
        }
        last_was_ws = false;
        for lc in ch.to_lowercase() {
            out.push(lc);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

pub(crate) fn extract_reference_definition_label(line: &str) -> Option<String> {
    // CommonMark-ish reference definition, single line only:
    // up to 3 leading spaces, then "[label]:"
    //
    // We purposely keep this lightweight and streaming-friendly; multi-line definitions
    // can be supported later via a dedicated block mode.
    let s = strip_up_to_three_leading_spaces(line);
    let bytes = s.as_bytes();
    if bytes.len() < 4 || bytes[0] != b'[' {
        return None;
    }
    let close = s.find(']')?;
    if close == 1 {
        return None;
    }
    if s.as_bytes().get(close + 1) != Some(&b':') {
        return None;
    }
    let label = &s[1..close];
    // Exclude footnote definitions like "[^1]:"
    if label.starts_with('^') {
        return None;
    }
    normalize_reference_label(label)
}

pub(crate) fn extract_reference_definition_line(line: &str) -> Option<(String, String)> {
    let label = extract_reference_definition_label(line)?;
    Some((label, line.trim_end().to_string()))
}

fn extract_reference_usages(text: &str) -> HashSet<String> {
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
