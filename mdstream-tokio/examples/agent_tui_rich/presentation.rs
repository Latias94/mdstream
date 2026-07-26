use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mdstream_protocol::TransitionNodeKey;
use ratatui::text::Line;

const CATCH_UP_ENTER_DEPTH: usize = 8;
const CATCH_UP_EXIT_DEPTH: usize = 3;
const CATCH_UP_ENTER_AGE: Duration = Duration::from_millis(120);
const CATCH_UP_EXIT_AGE: Duration = Duration::from_millis(60);
const CATCH_UP_BATCH_LINES: usize = 64;

pub(super) type RootKey = TransitionNodeKey;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct RootProjection {
    key: RootKey,
    lines: Arc<[Line<'static>]>,
}

impl RootProjection {
    pub(super) fn new(key: RootKey, lines: Vec<Line<'static>>) -> Self {
        Self {
            key,
            lines: lines.into(),
        }
    }

    pub(super) const fn key(&self) -> RootKey {
        self.key
    }

    pub(super) fn lines(&self) -> &[Line<'static>] {
        &self.lines
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LineStage {
    Committed,
    Queued,
    Mutable,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PresentationLine {
    pub(super) owner: RootKey,
    pub(super) row: usize,
    pub(super) stage: LineStage,
    pub(super) line: Line<'static>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct TickResult {
    pub(super) changed: bool,
    pub(super) committed_lines: usize,
    pub(super) catch_up: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PresentationMetrics {
    pub(super) reconciliations: u64,
    pub(super) enqueued_lines: u64,
    pub(super) committed_lines: u64,
    pub(super) corrections: u64,
    pub(super) full_replacements: u64,
    pub(super) catch_up_entries: u64,
    pub(super) max_queue_depth: usize,
}

#[derive(Debug)]
struct PresentedRoot {
    projection: RootProjection,
    committed_lines: usize,
}

#[derive(Debug)]
struct QueuedLine {
    owner: RootKey,
    row: usize,
    enqueued_at: Instant,
    ordinal: u64,
}

#[derive(Debug, Default)]
pub(super) struct PresentationState {
    presented: Vec<PresentedRoot>,
    presented_index: HashMap<RootKey, usize>,
    mutable_tail: Vec<RootProjection>,
    mutable_index: HashMap<RootKey, usize>,
    queue: VecDeque<QueuedLine>,
    enqueue_times: BTreeMap<Instant, usize>,
    presented_history: HashSet<RootKey>,
    next_ordinal: u64,
    paused: bool,
    reduced_motion: bool,
    catch_up: bool,
    total_line_count: usize,
    committed_line_count: usize,
    metrics: PresentationMetrics,
}

impl PresentationState {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn reconcile(
        &mut self,
        stable_prefix: Vec<RootProjection>,
        mutable_tail: Vec<RootProjection>,
        now: Instant,
        full_replace: bool,
    ) -> bool {
        self.metrics.reconciliations = self.metrics.reconciliations.saturating_add(1);

        let stable_prefix = unique_projections(stable_prefix, &HashSet::new());
        let stable_keys = stable_prefix
            .iter()
            .map(RootProjection::key)
            .collect::<HashSet<_>>();
        let mutable_tail = unique_projections(mutable_tail, &stable_keys);
        let mutable_changed = self.mutable_tail != mutable_tail;

        if full_replace {
            self.metrics.full_replacements = self.metrics.full_replacements.saturating_add(1);
            self.reset_for_full_replacement();
        }

        let mut stable_changed = self.presented.len() != stable_prefix.len();
        let mut previous = std::mem::take(&mut self.presented)
            .into_iter()
            .enumerate()
            .map(|(index, root)| (root.projection.key, (index, root)))
            .collect::<HashMap<_, _>>();
        let queued_at = std::mem::take(&mut self.queue)
            .into_iter()
            .map(|queued| ((queued.owner, queued.row), queued.enqueued_at))
            .collect::<HashMap<_, _>>();
        let mut next_presented = Vec::with_capacity(stable_prefix.len());

        for (index, projection) in stable_prefix.into_iter().enumerate() {
            let key = projection.key;
            if let Some((previous_index, mut root)) = previous.remove(&key) {
                stable_changed |= previous_index != index;
                if root.projection.lines == projection.lines {
                    root.projection = projection;
                } else {
                    stable_changed = true;
                    self.metrics.corrections = self.metrics.corrections.saturating_add(1);
                    self.record_committed(projection.lines.len());
                    root = PresentedRoot {
                        committed_lines: projection.lines.len(),
                        projection,
                    };
                }
                next_presented.push(root);
                continue;
            }

            stable_changed = true;
            let is_first_presentation = self.presented_history.insert(key);
            let committed_lines = if is_first_presentation {
                0
            } else {
                projection.lines.len()
            };
            next_presented.push(PresentedRoot {
                projection,
                committed_lines,
            });
        }

        self.presented = next_presented;
        self.rebuild_presented_index();
        self.rebuild_queue(queued_at, now);
        self.mutable_tail = mutable_tail;
        self.rebuild_mutable_index();
        self.refresh_line_counts();
        self.reset_catch_up_if_below_exit(now);
        self.metrics.max_queue_depth = self.metrics.max_queue_depth.max(self.queue.len());

        let drain = if self.reduced_motion && !self.paused {
            self.drain_all_internal()
        } else {
            TickResult::default()
        };

        full_replace || stable_changed || mutable_changed || drain.changed
    }

    pub(super) fn tick(&mut self, now: Instant) -> TickResult {
        if self.paused {
            return TickResult::default();
        }
        if self.reduced_motion {
            return self.drain_all_internal();
        }
        if self.queue.is_empty() {
            self.catch_up = false;
            return TickResult::default();
        }

        if !self.catch_up
            && (self.queue.len() >= CATCH_UP_ENTER_DEPTH
                || self.oldest_age(now) >= CATCH_UP_ENTER_AGE)
        {
            self.catch_up = true;
            self.metrics.catch_up_entries = self.metrics.catch_up_entries.saturating_add(1);
        }

        let used_catch_up = self.catch_up;
        let limit = if used_catch_up {
            CATCH_UP_BATCH_LINES
        } else {
            1
        };
        let committed_lines = self.commit_up_to(limit);
        self.reset_catch_up_if_below_exit(now);

        TickResult {
            changed: committed_lines != 0,
            committed_lines,
            catch_up: used_catch_up,
        }
    }

    pub(super) fn set_paused(&mut self, paused: bool) -> TickResult {
        if self.paused == paused {
            return TickResult::default();
        }

        self.paused = paused;
        let mut result = TickResult {
            changed: true,
            ..TickResult::default()
        };
        if !paused && self.reduced_motion {
            let drain = self.drain_all_internal();
            result.committed_lines = drain.committed_lines;
        }
        result
    }

    pub(super) fn set_reduced_motion(&mut self, reduced_motion: bool) -> TickResult {
        if self.reduced_motion == reduced_motion {
            return TickResult::default();
        }

        self.reduced_motion = reduced_motion;
        let mut result = TickResult {
            changed: true,
            ..TickResult::default()
        };
        if reduced_motion && !self.paused {
            let drain = self.drain_all_internal();
            result.committed_lines = drain.committed_lines;
        }
        result
    }

    pub(super) fn drain_all(&mut self) -> TickResult {
        self.drain_all_internal()
    }

    pub(super) fn lines(&self) -> Vec<PresentationLine> {
        let stable_lines = self.presented.iter().flat_map(|root| {
            root.projection
                .lines
                .iter()
                .cloned()
                .enumerate()
                .map(move |(row, line)| PresentationLine {
                    owner: root.projection.key,
                    row,
                    stage: if row < root.committed_lines {
                        LineStage::Committed
                    } else {
                        LineStage::Queued
                    },
                    line,
                })
        });
        let mutable_lines = self.mutable_tail.iter().flat_map(|root| {
            root.lines
                .iter()
                .cloned()
                .enumerate()
                .map(move |(row, line)| PresentationLine {
                    owner: root.key,
                    row,
                    stage: LineStage::Mutable,
                    line,
                })
        });

        stable_lines.chain(mutable_lines).collect()
    }

    pub(super) fn line_stage(&self, owner: RootKey, row: usize) -> Option<LineStage> {
        if let Some(index) = self.presented_index.get(&owner).copied() {
            let root = &self.presented[index];
            return (row < root.projection.lines.len()).then_some(if row < root.committed_lines {
                LineStage::Committed
            } else {
                LineStage::Queued
            });
        }

        self.mutable_index
            .get(&owner)
            .and_then(|index| self.mutable_tail.get(*index))
            .and_then(|root| (row < root.lines.len()).then_some(LineStage::Mutable))
    }

    pub(super) fn queue_len(&self) -> usize {
        self.queue.len()
    }

    pub(super) fn line_count(&self) -> usize {
        self.total_line_count
    }

    pub(super) fn mutable_root_count(&self) -> usize {
        self.mutable_tail.len()
    }

    pub(super) fn presented_root_count(&self) -> usize {
        self.presented.len()
    }

    pub(super) fn committed_line_count(&self) -> usize {
        self.committed_line_count
    }

    pub(super) fn needs_tick(&self) -> bool {
        !self.queue.is_empty() && !self.paused && !self.reduced_motion
    }

    pub(super) fn is_idle(&self) -> bool {
        self.queue.is_empty()
    }

    pub(super) const fn is_paused(&self) -> bool {
        self.paused
    }

    pub(super) const fn is_reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    pub(super) const fn metrics(&self) -> PresentationMetrics {
        self.metrics
    }

    fn rebuild_queue(&mut self, queued_at: HashMap<(RootKey, usize), Instant>, now: Instant) {
        let mut queue = VecDeque::new();
        let mut enqueue_times = BTreeMap::new();
        let mut next_ordinal = self.next_ordinal;
        let mut enqueued_lines = 0;

        for root in &self.presented {
            for row in root.committed_lines..root.projection.lines.len() {
                let enqueued_at = queued_at
                    .get(&(root.projection.key, row))
                    .copied()
                    .unwrap_or_else(|| {
                        enqueued_lines += 1;
                        now
                    });
                *enqueue_times.entry(enqueued_at).or_insert(0) += 1;
                queue.push_back(QueuedLine {
                    owner: root.projection.key,
                    row,
                    enqueued_at,
                    ordinal: next_ordinal,
                });
                next_ordinal = next_ordinal.saturating_add(1);
            }
        }

        self.queue = queue;
        self.enqueue_times = enqueue_times;
        self.next_ordinal = next_ordinal;
        self.metrics.enqueued_lines = self
            .metrics
            .enqueued_lines
            .saturating_add(usize_as_u64(enqueued_lines));
    }

    fn commit_up_to(&mut self, limit: usize) -> usize {
        let mut committed_lines = 0;
        while committed_lines < limit {
            let Some(queued) = self.queue.pop_front() else {
                break;
            };
            if let Some(next) = self.queue.front() {
                debug_assert!(queued.ordinal <= next.ordinal);
            }

            let Some(index) = self.presented_index.get(&queued.owner).copied() else {
                self.remove_enqueue_time(queued.enqueued_at);
                continue;
            };
            let root = &self.presented[index];
            if queued.row < root.committed_lines || queued.row >= root.projection.lines.len() {
                self.remove_enqueue_time(queued.enqueued_at);
                continue;
            }
            if queued.row != root.committed_lines {
                self.queue.push_front(queued);
                break;
            }

            self.remove_enqueue_time(queued.enqueued_at);
            self.presented[index].committed_lines += 1;
            self.committed_line_count = self.committed_line_count.saturating_add(1);
            committed_lines += 1;
        }

        self.record_committed(committed_lines);
        committed_lines
    }

    fn drain_all_internal(&mut self) -> TickResult {
        let committed_lines = self.commit_up_to(usize::MAX);
        self.catch_up = false;
        TickResult {
            changed: committed_lines != 0,
            committed_lines,
            catch_up: false,
        }
    }

    fn oldest_age(&self, now: Instant) -> Duration {
        self.enqueue_times
            .first_key_value()
            .and_then(|(enqueued_at, _)| now.checked_duration_since(*enqueued_at))
            .unwrap_or_default()
    }

    fn remove_enqueue_time(&mut self, enqueued_at: Instant) {
        let Some(count) = self.enqueue_times.get_mut(&enqueued_at) else {
            debug_assert!(false, "queued line must retain its enqueue-time index");
            return;
        };
        *count -= 1;
        if *count == 0 {
            self.enqueue_times.remove(&enqueued_at);
        }
    }

    fn reset_catch_up_if_below_exit(&mut self, now: Instant) {
        if self.queue.is_empty()
            || (self.catch_up
                && self.queue.len() <= CATCH_UP_EXIT_DEPTH
                && self.oldest_age(now) < CATCH_UP_EXIT_AGE)
        {
            self.catch_up = false;
        }
    }

    fn record_committed(&mut self, committed_lines: usize) {
        self.metrics.committed_lines = self
            .metrics
            .committed_lines
            .saturating_add(usize_as_u64(committed_lines));
    }

    fn rebuild_presented_index(&mut self) {
        self.presented_index.clear();
        self.presented_index.extend(
            self.presented
                .iter()
                .enumerate()
                .map(|(index, root)| (root.projection.key, index)),
        );
    }

    fn rebuild_mutable_index(&mut self) {
        self.mutable_index.clear();
        self.mutable_index.extend(
            self.mutable_tail
                .iter()
                .enumerate()
                .map(|(index, root)| (root.key, index)),
        );
    }

    fn refresh_line_counts(&mut self) {
        self.total_line_count = self
            .presented
            .iter()
            .map(|root| root.projection.lines.len())
            .chain(self.mutable_tail.iter().map(|root| root.lines.len()))
            .sum();
        self.committed_line_count = self.presented.iter().map(|root| root.committed_lines).sum();
    }

    fn reset_for_full_replacement(&mut self) {
        self.presented.clear();
        self.presented_index.clear();
        self.mutable_tail.clear();
        self.mutable_index.clear();
        self.queue.clear();
        self.enqueue_times.clear();
        self.presented_history.clear();
        self.next_ordinal = 0;
        self.catch_up = false;
        self.total_line_count = 0;
        self.committed_line_count = 0;
    }
}

fn unique_projections(
    projections: Vec<RootProjection>,
    excluded: &HashSet<RootKey>,
) -> Vec<RootProjection> {
    let mut seen = excluded.clone();
    projections
        .into_iter()
        .filter(|projection| seen.insert(projection.key))
        .collect()
}

fn usize_as_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use mdstream_protocol::{ContinuityGeneration, Epoch, NodeId};

    use super::*;

    #[test]
    fn queued_lines_are_visible_once_and_do_not_remain_in_the_mutable_tail() {
        let now = Instant::now();
        let owner = key(1, 1);
        let stable = projection(owner, &["first", "second"]);
        let duplicate_tail = projection(owner, &["first", "second"]);
        let mut state = PresentationState::new();

        assert!(state.reconcile(vec![stable], vec![duplicate_tail], now, false));

        let lines = state.lines();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.owner == owner));
        assert_eq!(
            lines.iter().map(|line| line.row).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(lines.iter().all(|line| line.stage == LineStage::Queued));
        assert_eq!(state.queue_len(), 2);
        assert_eq!(state.presented_root_count(), 1);
        assert_eq!(state.mutable_root_count(), 0);
    }

    #[test]
    fn smooth_tick_commits_only_one_line_of_a_multi_line_root() {
        let now = Instant::now();
        let mut state = PresentationState::new();
        state.reconcile(
            vec![projection(key(1, 1), &["one", "two", "three"])],
            Vec::new(),
            now,
            false,
        );

        let tick = state.tick(now);

        assert_eq!(
            tick,
            TickResult {
                changed: true,
                committed_lines: 1,
                catch_up: false,
            }
        );
        assert_eq!(
            stages(&state),
            vec![LineStage::Committed, LineStage::Queued, LineStage::Queued]
        );
        assert_eq!(state.committed_line_count(), 1);
        assert_eq!(state.queue_len(), 2);
    }

    #[test]
    fn correction_replaces_the_projection_and_never_replays_it() {
        let now = Instant::now();
        let owner = key(1, 1);
        let mut state = PresentationState::new();
        state.reconcile(
            vec![projection(owner, &["old one", "old two", "old three"])],
            Vec::new(),
            now,
            false,
        );
        state.tick(now);

        assert!(state.reconcile(
            vec![projection(owner, &["new one", "new two"])],
            Vec::new(),
            now + Duration::from_millis(1),
            false,
        ));

        let lines = state.lines();
        assert_eq!(
            lines
                .iter()
                .map(|line| line.line.clone())
                .collect::<Vec<_>>(),
            vec![Line::from("new one"), Line::from("new two")]
        );
        assert!(lines.iter().all(|line| line.stage == LineStage::Committed));
        assert_eq!(state.queue_len(), 0);
        assert_eq!(state.committed_line_count(), 2);
        assert_eq!(state.metrics().corrections, 1);
        assert_eq!(state.metrics().committed_lines, 3);

        assert!(!state.reconcile(
            vec![projection(owner, &["new one", "new two"])],
            Vec::new(),
            now + Duration::from_millis(2),
            false,
        ));
        assert_eq!(state.metrics().corrections, 1);
        assert_eq!(state.tick(now + CATCH_UP_ENTER_AGE), TickResult::default());
    }

    #[test]
    fn unchanged_projection_preserves_frontier_and_original_enqueue_time() {
        let now = Instant::now();
        let owner = key(1, 1);
        let stable = projection(owner, &["one", "two", "three"]);
        let mut state = PresentationState::new();
        state.reconcile(vec![stable.clone()], Vec::new(), now, false);
        state.tick(now);

        assert!(!state.reconcile(
            vec![stable],
            Vec::new(),
            now + Duration::from_millis(119),
            false,
        ));
        let tick = state.tick(now + CATCH_UP_ENTER_AGE);

        assert_eq!(tick.committed_lines, 2);
        assert!(tick.catch_up);
        assert_eq!(state.committed_line_count(), 3);
    }

    #[test]
    fn full_replace_drops_old_generation_state_for_the_same_node_id() {
        let now = Instant::now();
        let old_owner = key(1, 7);
        let new_owner = key(2, 7);
        let mut state = PresentationState::new();
        state.reconcile(
            vec![projection(old_owner, &["old one", "old two"])],
            Vec::new(),
            now,
            false,
        );
        state.tick(now);

        assert!(state.reconcile(
            vec![projection(new_owner, &["replacement"])],
            Vec::new(),
            now + Duration::from_millis(1),
            true,
        ));

        let lines = state.lines();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].owner, new_owner);
        assert_eq!(lines[0].line, Line::from("replacement"));
        assert_eq!(lines[0].stage, LineStage::Queued);
        assert_eq!(state.queue_len(), 1);
        assert_eq!(state.committed_line_count(), 0);
        assert_eq!(state.metrics().full_replacements, 1);

        state.drain_all();
        state.reconcile(
            vec![projection(new_owner, &["replacement"])],
            Vec::new(),
            now + Duration::from_millis(2),
            true,
        );
        assert_eq!(state.queue_len(), 1);
        assert_eq!(state.metrics().corrections, 0);
    }

    #[test]
    fn pause_precedes_reduced_motion_but_not_canonical_reconciliation() {
        let now = Instant::now();
        let owner = key(1, 1);
        let replacement_owner = key(2, 1);
        let mut state = PresentationState::new();

        assert!(state.set_paused(true).changed);
        assert!(!state.set_paused(true).changed);
        assert!(state.set_reduced_motion(true).changed);
        state.reconcile(
            vec![projection(owner, &["one", "two"])],
            Vec::new(),
            now,
            false,
        );
        assert_eq!(state.queue_len(), 2);
        assert_eq!(state.tick(now), TickResult::default());

        state.reconcile(
            vec![projection(owner, &["corrected"])],
            Vec::new(),
            now + Duration::from_millis(1),
            false,
        );
        assert_eq!(state.queue_len(), 0);
        assert_eq!(stages(&state), vec![LineStage::Committed]);
        assert_eq!(state.metrics().corrections, 1);

        state.reconcile(
            vec![projection(replacement_owner, &["full replacement"])],
            Vec::new(),
            now + Duration::from_millis(2),
            true,
        );
        assert_eq!(state.queue_len(), 1);
        assert_eq!(state.lines()[0].owner, replacement_owner);
        assert_eq!(state.lines()[0].stage, LineStage::Queued);

        let resumed = state.set_paused(false);
        assert!(resumed.changed);
        assert_eq!(resumed.committed_lines, 1);
        assert_eq!(state.queue_len(), 0);
        assert_eq!(stages(&state), vec![LineStage::Committed]);
        assert!(!state.needs_tick());
        assert!(!state.is_paused());
        assert!(state.is_reduced_motion());
    }

    #[test]
    fn catch_up_is_bounded_and_preserves_fifo_order() {
        let now = Instant::now();
        let first_owner = key(1, 1);
        let second_owner = key(1, 2);
        let mut state = PresentationState::new();
        state.reconcile(
            vec![
                numbered_projection(first_owner, 64),
                numbered_projection(second_owner, 2),
            ],
            Vec::new(),
            now,
            false,
        );

        let catch_up = state.tick(now);

        assert_eq!(catch_up.committed_lines, CATCH_UP_BATCH_LINES);
        assert!(catch_up.catch_up);
        assert_eq!(state.queue_len(), 2);
        assert_eq!(state.committed_line_count(), 64);
        assert!(
            state
                .lines()
                .iter()
                .filter(|line| line.owner == second_owner)
                .all(|line| line.stage == LineStage::Queued)
        );

        let smooth = state.tick(now);
        assert_eq!(smooth.committed_lines, 1);
        assert!(!smooth.catch_up);
        let second = state
            .lines()
            .into_iter()
            .filter(|line| line.owner == second_owner)
            .collect::<Vec<_>>();
        assert_eq!(second[0].stage, LineStage::Committed);
        assert_eq!(second[1].stage, LineStage::Queued);
        assert_eq!(state.metrics().catch_up_entries, 1);
        assert_eq!(state.metrics().max_queue_depth, 66);
    }

    #[test]
    fn old_queue_age_enters_catch_up_below_the_depth_threshold() {
        let now = Instant::now();
        let mut state = PresentationState::new();
        state.reconcile(
            vec![projection(key(1, 1), &["one", "two"])],
            Vec::new(),
            now,
            false,
        );

        let tick = state.tick(now + CATCH_UP_ENTER_AGE);

        assert_eq!(tick.committed_lines, 2);
        assert!(tick.catch_up);
        assert!(state.is_idle());
    }

    #[test]
    fn a_previously_presented_key_returns_without_reanimation() {
        let now = Instant::now();
        let owner = key(1, 1);
        let mut state = PresentationState::new();
        state.reconcile(
            vec![projection(owner, &["one", "two"])],
            Vec::new(),
            now,
            false,
        );
        state.drain_all();
        state.reconcile(Vec::new(), Vec::new(), now, false);
        assert_eq!(state.presented_root_count(), 0);

        state.reconcile(
            vec![projection(owner, &["one", "two"])],
            Vec::new(),
            now + Duration::from_millis(1),
            false,
        );

        assert_eq!(state.queue_len(), 0);
        assert_eq!(
            stages(&state),
            vec![LineStage::Committed, LineStage::Committed]
        );
        assert_eq!(state.metrics().enqueued_lines, 2);
        assert_eq!(state.metrics().committed_lines, 2);
    }

    #[test]
    fn removal_cleans_queued_references_without_disturbing_survivors() {
        let now = Instant::now();
        let removed = key(1, 1);
        let survivor = key(1, 2);
        let survivor_projection = projection(survivor, &["survivor one", "survivor two"]);
        let mut state = PresentationState::new();
        state.reconcile(
            vec![
                projection(removed, &["removed one", "removed two"]),
                survivor_projection.clone(),
            ],
            Vec::new(),
            now,
            false,
        );
        state.tick(now);

        state.reconcile(
            vec![survivor_projection],
            Vec::new(),
            now + Duration::from_millis(1),
            false,
        );

        assert_eq!(state.queue_len(), 2);
        let tick = state.tick(now + Duration::from_millis(1));
        assert_eq!(tick.committed_lines, 1);
        assert_eq!(state.lines()[0].owner, survivor);
        assert_eq!(state.lines()[0].stage, LineStage::Committed);
    }

    #[test]
    fn inserted_stable_root_rebuilds_fifo_in_canonical_order() {
        let now = Instant::now();
        let successor = key(1, 2);
        let inserted = key(1, 1);
        let successor_projection = projection(successor, &["successor"]);
        let mut state = PresentationState::new();
        state.reconcile(vec![successor_projection.clone()], Vec::new(), now, false);

        state.reconcile(
            vec![projection(inserted, &["inserted"]), successor_projection],
            Vec::new(),
            now + Duration::from_millis(1),
            false,
        );
        let tick = state.tick(now + Duration::from_millis(1));

        assert_eq!(tick.committed_lines, 1);
        assert!(!tick.catch_up);
        let lines = state.lines();
        assert_eq!(lines[0].owner, inserted);
        assert_eq!(lines[0].stage, LineStage::Committed);
        assert_eq!(lines[1].owner, successor);
        assert_eq!(lines[1].stage, LineStage::Queued);
    }

    #[test]
    fn inserted_root_does_not_hide_an_aged_successor_from_catch_up() {
        let now = Instant::now();
        let successor = key(1, 2);
        let successor_projection = projection(successor, &["successor"]);
        let mut state = PresentationState::new();
        state.reconcile(vec![successor_projection.clone()], Vec::new(), now, false);

        let aged = now + CATCH_UP_ENTER_AGE;
        state.reconcile(
            vec![projection(key(1, 1), &["inserted"]), successor_projection],
            Vec::new(),
            aged,
            false,
        );
        let tick = state.tick(aged);

        assert!(tick.catch_up);
        assert_eq!(tick.committed_lines, 2);
        assert!(state.is_idle());
    }

    #[test]
    fn empty_line_is_work_while_zero_line_root_is_already_idle() {
        let now = Instant::now();
        let zero_line_owner = key(1, 1);
        let empty_line_owner = key(1, 2);
        let mut state = PresentationState::new();
        state.reconcile(
            vec![
                RootProjection::new(zero_line_owner, Vec::new()),
                RootProjection::new(empty_line_owner, vec![Line::default()]),
            ],
            Vec::new(),
            now,
            false,
        );

        assert_eq!(state.presented_root_count(), 2);
        assert_eq!(state.queue_len(), 1);
        assert_eq!(state.lines().len(), 1);
        assert_eq!(state.lines()[0].owner, empty_line_owner);
        assert_eq!(state.lines()[0].row, 0);
        state.tick(now);
        assert!(state.is_idle());

        state.reconcile(
            vec![RootProjection::new(zero_line_owner, Vec::new())],
            Vec::new(),
            now,
            false,
        );
        assert!(state.lines().is_empty());
        assert_eq!(state.committed_line_count(), 0);
        assert!(state.is_idle());
    }

    #[test]
    fn mutable_only_content_is_visible_but_presentation_is_idle() {
        let now = Instant::now();
        let owner = key(1, 1);
        let tail = projection(owner, &["mutable one", "mutable two"]);
        assert_eq!(tail.key(), owner);
        assert_eq!(tail.lines().len(), 2);
        let mut state = PresentationState::new();

        state.reconcile(Vec::new(), vec![tail], now, false);

        assert_eq!(state.presented_root_count(), 0);
        assert_eq!(state.mutable_root_count(), 1);
        assert_eq!(state.queue_len(), 0);
        assert!(!state.needs_tick());
        assert!(state.is_idle());
        assert!(
            state
                .lines()
                .iter()
                .all(|line| line.stage == LineStage::Mutable)
        );
    }

    #[test]
    fn unpaused_reduced_motion_drains_current_and_future_work_once() {
        let now = Instant::now();
        let first_owner = key(1, 1);
        let second_owner = key(1, 2);
        let third_owner = key(1, 3);
        let mut state = PresentationState::new();
        state.reconcile(
            vec![projection(first_owner, &["one", "two"])],
            Vec::new(),
            now,
            false,
        );

        let reduced = state.set_reduced_motion(true);
        assert_eq!(reduced.committed_lines, 2);
        assert!(state.is_idle());

        state.reconcile(
            vec![
                projection(first_owner, &["one", "two"]),
                projection(second_owner, &["three"]),
            ],
            Vec::new(),
            now,
            false,
        );
        assert!(state.is_idle());
        assert_eq!(state.committed_line_count(), 3);

        assert!(state.set_reduced_motion(false).changed);
        assert!(!state.set_reduced_motion(false).changed);
        state.reconcile(
            vec![
                projection(first_owner, &["one", "two"]),
                projection(second_owner, &["three"]),
                projection(third_owner, &["four", "five"]),
            ],
            Vec::new(),
            now + Duration::from_millis(1),
            false,
        );
        assert_eq!(
            stages(&state),
            vec![
                LineStage::Committed,
                LineStage::Committed,
                LineStage::Committed,
                LineStage::Queued,
                LineStage::Queued,
            ]
        );
        assert_eq!(state.queue_len(), 2);
        assert!(state.needs_tick());

        let paced = state.tick(now + Duration::from_millis(2));
        assert_eq!(paced.committed_lines, 1);
        assert_eq!(
            stages(&state),
            vec![
                LineStage::Committed,
                LineStage::Committed,
                LineStage::Committed,
                LineStage::Committed,
                LineStage::Queued,
            ]
        );
    }

    fn key(generation: u64, node_id: u64) -> RootKey {
        RootKey {
            continuity_generation: ContinuityGeneration::new(generation),
            epoch: Epoch::new(1),
            node_id: NodeId::from(node_id),
        }
    }

    fn projection(owner: RootKey, lines: &[&str]) -> RootProjection {
        RootProjection::new(
            owner,
            lines
                .iter()
                .map(|line| Line::from((*line).to_owned()))
                .collect(),
        )
    }

    fn numbered_projection(owner: RootKey, line_count: usize) -> RootProjection {
        RootProjection::new(
            owner,
            (0..line_count)
                .map(|row| Line::from(format!("line {row}")))
                .collect(),
        )
    }

    fn stages(state: &PresentationState) -> Vec<LineStage> {
        state.lines().into_iter().map(|line| line.stage).collect()
    }
}
