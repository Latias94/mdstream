use std::collections::HashMap;

use crate::reference::ReferenceDefinitions;
use crate::types::{Block, BlockId, Update};

use pulldown_cmark::{Event, Options as PulldownOptions, Parser};

#[cfg(not(feature = "sync"))]
use std::cell::RefCell;
#[cfg(feature = "sync")]
use std::sync::{Mutex, MutexGuard};

#[cfg(not(feature = "sync"))]
type ParseScratch = RefCell<String>;
#[cfg(feature = "sync")]
type ParseScratch = Mutex<String>;

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
    reference_definitions: ReferenceDefinitions,
    parse_scratch: ParseScratch,
}

impl PulldownAdapter {
    pub fn new(opts: PulldownAdapterOptions) -> Self {
        Self {
            opts,
            committed_raw: HashMap::new(),
            committed_cache: HashMap::new(),
            reference_definitions: ReferenceDefinitions::default(),
            parse_scratch: Default::default(),
        }
    }

    pub fn clear(&mut self) {
        self.committed_raw.clear();
        self.committed_cache.clear();
        self.reference_definitions.clear();
        #[cfg(not(feature = "sync"))]
        {
            self.parse_scratch.borrow_mut().clear();
        }
        #[cfg(feature = "sync")]
        {
            self.lock_parse_scratch().clear();
        }
    }

    pub fn apply_update(&mut self, update: &Update) {
        if update.reset {
            self.clear();
        }
        for block in &update.committed {
            self.committed_raw.insert(block.id, block.raw.clone());
            self.reference_definitions.observe_text(&block.raw);
            self.reference_definitions.refresh();
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
        let prelude = self.reference_definitions.prelude_text();
        if prelude.is_empty() {
            return parse_events_static(raw, self.opts.pulldown);
        }
        #[cfg(not(feature = "sync"))]
        {
            let mut scratch = self.parse_scratch.borrow_mut();
            scratch.clear();
            scratch.reserve(prelude.len() + 2 + raw.len());
            scratch.push_str(prelude);
            scratch.push_str("\n\n");
            scratch.push_str(raw);
            parse_events_static(&scratch, self.opts.pulldown)
        }
        #[cfg(feature = "sync")]
        {
            let mut scratch = self.lock_parse_scratch();
            scratch.clear();
            scratch.reserve(prelude.len() + 2 + raw.len());
            scratch.push_str(prelude);
            scratch.push_str("\n\n");
            scratch.push_str(raw);
            parse_events_static(&scratch, self.opts.pulldown)
        }
    }

    #[cfg(feature = "sync")]
    fn lock_parse_scratch(&self) -> MutexGuard<'_, String> {
        match self.parse_scratch.lock() {
            Ok(scratch) => scratch,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn parse_events_static(input: &str, options: PulldownOptions) -> Vec<Event<'static>> {
    Parser::new_ext(input, options)
        .map(|e| e.into_static())
        .collect()
}

#[cfg(all(test, feature = "sync"))]
mod tests {
    use std::panic::{self, AssertUnwindSafe};

    use pulldown_cmark::{Event, Tag};

    use super::*;

    fn contains_link(events: &[Event<'static>]) -> bool {
        events
            .iter()
            .any(|event| matches!(event, Event::Start(Tag::Link { .. })))
    }

    #[test]
    fn parse_scratch_recovers_from_poisoned_mutex() {
        let mut adapter = PulldownAdapter::new(PulldownAdapterOptions::default());
        adapter
            .reference_definitions
            .observe_text("[ref]: https://example.com");
        adapter.reference_definitions.refresh();

        let poison_result = panic::catch_unwind(AssertUnwindSafe(|| {
            let mut scratch = adapter.lock_parse_scratch();
            scratch.push_str("stale partial parse");
            panic!("poison scratch mutex");
        }));
        assert!(poison_result.is_err());

        let events = adapter.parse_with_definitions("See [ref].");
        assert!(contains_link(&events));

        let scratch = adapter.lock_parse_scratch();
        assert!(!scratch.starts_with("stale partial parse"));
    }
}
