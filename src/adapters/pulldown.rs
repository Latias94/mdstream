use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};

use crate::reference;
use crate::types::{Block, BlockId, Update};

use pulldown_cmark::{Event, Options as PulldownOptions, Parser};

#[derive(Debug, Clone)]
pub struct PulldownAdapterOptions {
    pub pulldown: PulldownOptions,
    /// If true, pending blocks are parsed from `display` (terminated) when available.
    pub prefer_display_for_pending: bool,
}

impl Default for PulldownAdapterOptions {
    fn default() -> Self {
        Self {
            pulldown: PulldownOptions::empty(),
            prefer_display_for_pending: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct PulldownAdapter {
    opts: PulldownAdapterOptions,
    committed_raw: HashMap<BlockId, String>,
    committed_cache: HashMap<BlockId, Vec<Event<'static>>>,
    reference_definitions: BTreeMap<String, String>,
    reference_definitions_text: String,
    reference_definitions_dirty: bool,
}

impl PulldownAdapter {
    pub fn new(opts: PulldownAdapterOptions) -> Self {
        Self {
            opts,
            committed_raw: HashMap::new(),
            committed_cache: HashMap::new(),
            reference_definitions: BTreeMap::new(),
            reference_definitions_text: String::new(),
            reference_definitions_dirty: false,
        }
    }

    pub fn clear(&mut self) {
        self.committed_raw.clear();
        self.committed_cache.clear();
        self.reference_definitions.clear();
        self.reference_definitions_text.clear();
        self.reference_definitions_dirty = false;
    }

    pub fn apply_update(&mut self, update: &Update) {
        if update.reset {
            self.clear();
        }
        for block in &update.committed {
            self.committed_raw.insert(block.id, block.raw.clone());
            self.collect_reference_definitions(&block.raw);
            self.refresh_reference_definitions_text();
            let events = self.parse_with_definitions(&block.raw);
            self.committed_cache.insert(block.id, events);
        }

        // If definitions arrived late, selectively re-parse invalidated blocks.
        for id in &update.invalidated {
            let Some(raw) = self.committed_raw.get(id) else {
                continue;
            };
            let events = self.parse_with_definitions(raw);
            self.committed_cache.insert(*id, events);
        }
    }

    pub fn committed_events(&self, id: BlockId) -> Option<&[Event<'static>]> {
        self.committed_cache.get(&id).map(|v| v.as_slice())
    }

    pub fn parse_pending(&self, pending: &Block) -> Vec<Event<'static>> {
        let input = if self.opts.prefer_display_for_pending {
            pending.display.as_deref().unwrap_or(&pending.raw)
        } else {
            &pending.raw
        };
        // Pending should reflect the best-known definitions so far too.
        self.parse_with_definitions(input)
    }

    fn parse_with_definitions(&self, raw: &str) -> Vec<Event<'static>> {
        if self.reference_definitions_text.is_empty() {
            return parse_events_static(raw, self.opts.pulldown);
        }
        // Allocate fresh - simpler and Sync-compatible
        let combined = format!("{}\n\n{}", self.reference_definitions_text, raw);
        parse_events_static(&combined, self.opts.pulldown)
    }

    fn collect_reference_definitions(&mut self, raw: &str) {
        // Best-effort: extract single-line reference definitions and keep the latest per label.
        for line in raw.split('\n') {
            if let Some((label, def_line)) = reference::extract_reference_definition_line(line) {
                match self.reference_definitions.entry(label) {
                    Entry::Vacant(v) => {
                        v.insert(def_line);
                        self.reference_definitions_dirty = true;
                    }
                    Entry::Occupied(mut o) => {
                        if o.get() != &def_line {
                            o.insert(def_line);
                            self.reference_definitions_dirty = true;
                        }
                    }
                }
            }
        }
    }

    fn refresh_reference_definitions_text(&mut self) {
        if !self.reference_definitions_dirty {
            return;
        }
        self.reference_definitions_text = self
            .reference_definitions
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        self.reference_definitions_dirty = false;
    }
}

fn parse_events_static(input: &str, options: PulldownOptions) -> Vec<Event<'static>> {
    Parser::new_ext(input, options)
        .map(|e| e.into_static())
        .collect()
}
