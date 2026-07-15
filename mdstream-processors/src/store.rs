use std::collections::BTreeMap;

use mdstream_protocol::{Epoch, NodeId};

use crate::{
    ArtifactChange, ArtifactChangeKind, ArtifactReleaseReason, CancellationToken,
    ProcessorArtifact, ProcessorFailure, ProcessorMetrics, ProcessorRequestKey, ProcessorSlotKey,
    ProcessorSlotState,
};

pub(crate) struct InFlightLease {
    pub input_bytes: usize,
    pub cancellation: CancellationToken,
}

#[derive(Default)]
pub(crate) struct ArtifactStore {
    slots: BTreeMap<ProcessorSlotKey, ProcessorSlotState>,
    in_flight: BTreeMap<ProcessorRequestKey, InFlightLease>,
    metrics: ProcessorMetrics,
    changes: Vec<ArtifactChange>,
}

impl ArtifactStore {
    pub fn metrics(&self) -> ProcessorMetrics {
        self.metrics
    }

    pub fn state(&self, slot: &ProcessorSlotKey) -> Option<&ProcessorSlotState> {
        self.slots.get(slot)
    }

    pub fn artifact(&self, slot: &ProcessorSlotKey) -> Option<&ProcessorArtifact> {
        self.state(slot).and_then(ProcessorSlotState::artifact)
    }

    pub fn contains_slot(&self, slot: &ProcessorSlotKey) -> bool {
        self.slots.contains_key(slot)
    }

    pub fn install_pending(
        &mut self,
        key: ProcessorRequestKey,
        input_bytes: usize,
        cancellation: CancellationToken,
    ) {
        if let Some(previous) = self.slots.insert(
            key.slot().clone(),
            ProcessorSlotState::Pending { key: key.clone() },
        ) {
            if let Some(lease) = self.in_flight.get(previous.key()) {
                lease.cancellation.cancel();
            }
            if previous.artifact().is_some() {
                self.metrics.released_artifacts = self.metrics.released_artifacts.saturating_add(1);
            }
            self.push_removed(previous, ArtifactReleaseReason::Replaced);
        }
        self.in_flight.insert(
            key.clone(),
            InFlightLease {
                input_bytes,
                cancellation,
            },
        );
        self.refresh_usage();
        self.metrics.issued_requests = self.metrics.issued_requests.saturating_add(1);
        self.changes
            .push(ArtifactChange::new(key, ArtifactChangeKind::Pending));
    }

    pub fn has_lease(&self, key: &ProcessorRequestKey) -> bool {
        self.in_flight.contains_key(key)
    }

    pub fn settle_lease(&mut self, key: &ProcessorRequestKey) -> bool {
        let removed = self.in_flight.remove(key).is_some();
        if removed {
            self.refresh_usage();
        }
        removed
    }

    pub fn current_pending(&self, key: &ProcessorRequestKey) -> bool {
        matches!(
            self.slots.get(key.slot()),
            Some(ProcessorSlotState::Pending { key: current }) if current == key
        )
    }

    pub fn record_stale_result(&mut self) {
        self.metrics.stale_results = self.metrics.stale_results.saturating_add(1);
    }

    pub fn install_artifact(&mut self, key: ProcessorRequestKey, artifact: ProcessorArtifact) {
        let artifact_bytes = artifact.byte_len();
        self.slots.insert(
            key.slot().clone(),
            ProcessorSlotState::Ready {
                key: key.clone(),
                artifact,
            },
        );
        self.refresh_usage();
        self.metrics.accepted_results = self.metrics.accepted_results.saturating_add(1);
        self.changes.push(ArtifactChange::new(
            key,
            ArtifactChangeKind::Ready { artifact_bytes },
        ));
    }

    pub fn install_failure(&mut self, key: ProcessorRequestKey, failure: ProcessorFailure) {
        let code = failure.code();
        self.slots.insert(
            key.slot().clone(),
            ProcessorSlotState::Failed {
                key: key.clone(),
                failure,
            },
        );
        self.refresh_usage();
        self.metrics.accepted_results = self.metrics.accepted_results.saturating_add(1);
        self.changes.push(ArtifactChange::new(
            key,
            ArtifactChangeKind::Failed { code },
        ));
    }

    pub fn clear(&mut self, reason: ArtifactReleaseReason) {
        let released = self.metrics.retained_artifacts as u64;
        for lease in self.in_flight.values() {
            lease.cancellation.cancel();
        }
        let states = std::mem::take(&mut self.slots);
        for state in states.into_values() {
            self.push_removed(state, reason);
        }
        self.in_flight.clear();
        self.refresh_usage();
        self.metrics.released_artifacts = self.metrics.released_artifacts.saturating_add(released);
    }

    pub fn remove_node(&mut self, epoch: Epoch, node_id: NodeId, reason: ArtifactReleaseReason) {
        self.remove_matching(
            |slot| slot.epoch() == epoch && slot.node_id() == node_id,
            reason,
        );
    }

    pub fn remove_slot(&mut self, slot: &ProcessorSlotKey, reason: ArtifactReleaseReason) {
        self.remove_matching(|candidate| candidate == slot, reason);
    }

    fn remove_matching(
        &mut self,
        matches: impl Fn(&ProcessorSlotKey) -> bool,
        reason: ArtifactReleaseReason,
    ) {
        let lease_keys = self
            .in_flight
            .keys()
            .filter(|key| matches(key.slot()))
            .cloned()
            .collect::<Vec<_>>();
        for key in lease_keys {
            if let Some(lease) = self.in_flight.remove(&key) {
                lease.cancellation.cancel();
            }
        }

        let slot_keys = self
            .slots
            .keys()
            .filter(|slot| matches(slot))
            .cloned()
            .collect::<Vec<_>>();
        for slot in slot_keys {
            if let Some(state) = self.slots.remove(&slot) {
                if state.artifact().is_some() {
                    self.metrics.released_artifacts =
                        self.metrics.released_artifacts.saturating_add(1);
                }
                self.push_removed(state, reason);
            }
        }
        self.refresh_usage();
    }

    pub fn cancel(&mut self, key: &ProcessorRequestKey) -> bool {
        let mut changed = false;
        if let Some(lease) = self.in_flight.remove(key) {
            lease.cancellation.cancel();
            changed = true;
        }
        let is_current = self
            .slots
            .get(key.slot())
            .is_some_and(|state| state.key() == key);
        if is_current {
            if let Some(state) = self.slots.remove(key.slot()) {
                if state.artifact().is_some() {
                    self.metrics.released_artifacts =
                        self.metrics.released_artifacts.saturating_add(1);
                }
                self.push_removed(state, ArtifactReleaseReason::Cancelled);
            }
            changed = true;
        }
        if changed {
            self.refresh_usage();
        }
        changed
    }

    pub fn take_changes(&mut self) -> Vec<ArtifactChange> {
        std::mem::take(&mut self.changes)
    }

    fn push_removed(&mut self, state: ProcessorSlotState, reason: ArtifactReleaseReason) {
        let released_artifact_bytes = state.artifact().map_or(0, ProcessorArtifact::byte_len);
        self.changes.push(ArtifactChange::new(
            state.key().clone(),
            ArtifactChangeKind::Removed {
                reason,
                released_artifact_bytes,
            },
        ));
    }

    fn refresh_usage(&mut self) {
        self.metrics.slots = self.slots.len();
        self.metrics.in_flight_jobs = self.in_flight.len();
        self.metrics.in_flight_input_bytes =
            checked_sum(self.in_flight.values().map(|lease| lease.input_bytes))
                .unwrap_or(usize::MAX);
        self.metrics.retained_artifacts = self
            .slots
            .values()
            .filter(|state| state.artifact().is_some())
            .count();
        self.metrics.retained_artifact_bytes = checked_sum(
            self.slots
                .values()
                .filter_map(ProcessorSlotState::artifact)
                .map(ProcessorArtifact::byte_len),
        )
        .unwrap_or(usize::MAX);
    }
}

fn checked_sum(mut values: impl Iterator<Item = usize>) -> Option<usize> {
    values.try_fold(0_usize, |total, value| total.checked_add(value))
}

#[cfg(test)]
mod tests {
    use super::checked_sum;

    #[test]
    fn aggregate_cost_reports_integer_overflow() {
        assert_eq!(checked_sum([usize::MAX, 1].into_iter()), None);
        assert_eq!(checked_sum([1, 2, 3].into_iter()), Some(6));
    }
}
