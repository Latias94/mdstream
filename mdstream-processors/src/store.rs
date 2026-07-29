use std::collections::{BTreeMap, BTreeSet};

use mdstream_protocol::{Epoch, NodeId, ProcessorInputVersion, RequestGeneration};

use crate::{
    ArtifactChange, ArtifactChangeKind, ArtifactReleaseReason, CancellationToken, HostError,
    ProcessorArtifact, ProcessorFailure, ProcessorId, ProcessorLimits, ProcessorMetrics,
    ProcessorRequestKey, ProcessorSlotKey, ProcessorSlotState, limits::check_limit,
};

pub(crate) struct InFlightLease {
    key: ProcessorRequestKey,
    input_bytes: usize,
    cancellation: CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ProcessorNodeKey {
    epoch: Epoch,
    node_id: NodeId,
}

impl ProcessorNodeKey {
    const fn new(epoch: Epoch, node_id: NodeId) -> Self {
        Self { epoch, node_id }
    }

    fn from_slot(slot: &ProcessorSlotKey) -> Self {
        Self::new(slot.epoch(), slot.node_id())
    }
}

#[derive(Default)]
struct NodeBucket {
    slots: BTreeMap<ProcessorId, SlotRecord>,
}

#[derive(Default)]
struct SlotRecord {
    state: Option<ProcessorSlotState>,
    in_flight: BTreeMap<RequestGeneration, InFlightLease>,
}

pub(crate) struct CompletionReservation {
    metrics: ProcessorMetrics,
    change: ArtifactChange,
}

pub(crate) struct PendingReservation {
    metrics: ProcessorMetrics,
    changes: Vec<ArtifactChange>,
}

#[derive(Clone, Copy)]
struct QueueReservation {
    pending_changes: usize,
    pending_change_bytes: usize,
}

pub(crate) struct ArtifactStore {
    nodes: BTreeMap<ProcessorNodeKey, NodeBucket>,
    metrics: ProcessorMetrics,
    changes: Vec<ArtifactChange>,
    max_pending_changes: usize,
    max_pending_change_bytes: usize,
}

impl ArtifactStore {
    pub fn new(limits: ProcessorLimits) -> Self {
        Self {
            nodes: BTreeMap::new(),
            metrics: ProcessorMetrics::default(),
            changes: Vec::new(),
            max_pending_changes: limits.max_pending_changes,
            max_pending_change_bytes: limits.max_pending_change_bytes,
        }
    }

    pub fn metrics(&self) -> ProcessorMetrics {
        self.metrics
    }

    pub fn state(&self, slot: &ProcessorSlotKey) -> Option<&ProcessorSlotState> {
        self.slot_record(slot)
            .and_then(|record| record.state.as_ref())
    }

    pub fn artifact(&self, slot: &ProcessorSlotKey) -> Option<&ProcessorArtifact> {
        self.state(slot).and_then(ProcessorSlotState::artifact)
    }

    pub fn contains_slot(&self, slot: &ProcessorSlotKey) -> bool {
        self.state(slot).is_some()
    }

    pub fn contains_node(&self, epoch: Epoch, node_id: NodeId) -> bool {
        self.nodes
            .contains_key(&ProcessorNodeKey::new(epoch, node_id))
    }

    #[cfg(test)]
    pub fn install_pending(
        &mut self,
        key: ProcessorRequestKey,
        input_bytes: usize,
        cancellation: CancellationToken,
    ) -> Result<(), HostError> {
        let reservation = self.preflight_pending(&key, input_bytes)?;
        self.commit_pending(reservation, key, input_bytes, cancellation);
        Ok(())
    }

    pub fn preflight_pending(
        &self,
        key: &ProcessorRequestKey,
        input_bytes: usize,
    ) -> Result<PendingReservation, HostError> {
        let previous = self.state(key.slot());
        let mut changes = Vec::with_capacity(2);
        if let Some(previous) = previous {
            changes.push(removed_change(previous, ArtifactReleaseReason::Replaced));
        }
        changes.push(ArtifactChange::new(
            key.clone(),
            ArtifactChangeKind::Pending,
        ));
        let queue = self.reserve_changes(&changes)?;
        if self
            .slot_record(key.slot())
            .is_some_and(|record| record.in_flight.contains_key(&key.generation()))
        {
            return Err(HostError::CounterOverflow("processor.request_generation"));
        }

        let mut metrics = self.metrics;
        if previous.is_none() {
            metrics.slots = checked_add(metrics.slots, 1, "processor.slots")?;
        }
        if let Some(artifact) = previous.and_then(ProcessorSlotState::artifact) {
            metrics.retained_artifacts = checked_sub(
                metrics.retained_artifacts,
                1,
                "processor.retained_artifacts",
            )?;
            metrics.retained_artifact_bytes = checked_sub(
                metrics.retained_artifact_bytes,
                artifact.byte_len(),
                "processor.retained_artifact_bytes",
            )?;
            metrics.released_artifacts = checked_add_u64(
                metrics.released_artifacts,
                1,
                "processor.released_artifacts",
            )?;
        }
        metrics.in_flight_jobs =
            checked_add(metrics.in_flight_jobs, 1, "processor.in_flight_jobs")?;
        metrics.in_flight_input_bytes = checked_add(
            metrics.in_flight_input_bytes,
            input_bytes,
            "processor.in_flight_input_bytes",
        )?;
        metrics.issued_requests =
            checked_add_u64(metrics.issued_requests, 1, "processor.issued_requests")?;
        metrics.input_materializations = checked_add_u64(
            metrics.input_materializations,
            1,
            "processor.input_materializations",
        )?;
        metrics.store_entry_visits = checked_add_u64(
            metrics.store_entry_visits,
            2,
            "processor.store_entry_visits",
        )?;
        apply_queue_reservation(&mut metrics, queue);

        Ok(PendingReservation { metrics, changes })
    }

    pub fn commit_pending(
        &mut self,
        reservation: PendingReservation,
        key: ProcessorRequestKey,
        input_bytes: usize,
        cancellation: CancellationToken,
    ) {
        let record = self.slot_record_mut_or_insert(key.slot());
        if let Some(previous) = record
            .state
            .replace(ProcessorSlotState::Pending { key: key.clone() })
        {
            if let Some(lease) = record.in_flight.get(&previous.key().generation()) {
                if lease.key == *previous.key() {
                    lease.cancellation.cancel();
                }
            }
        }
        record.in_flight.insert(
            key.generation(),
            InFlightLease {
                key,
                input_bytes,
                cancellation,
            },
        );
        self.commit(reservation.metrics, reservation.changes);
    }

    pub fn has_lease(&self, key: &ProcessorRequestKey) -> bool {
        self.slot_record(key.slot())
            .and_then(|record| record.in_flight.get(&key.generation()))
            .is_some_and(|lease| lease.key == *key)
    }

    pub fn settle_stale(&mut self, key: &ProcessorRequestKey) -> Result<bool, HostError> {
        let Some(lease) = self
            .slot_record(key.slot())
            .and_then(|record| record.in_flight.get(&key.generation()))
            .filter(|lease| lease.key == *key)
        else {
            return Ok(false);
        };
        let mut metrics = self.metrics;
        metrics.in_flight_jobs =
            checked_sub(metrics.in_flight_jobs, 1, "processor.in_flight_jobs")?;
        metrics.in_flight_input_bytes = checked_sub(
            metrics.in_flight_input_bytes,
            lease.input_bytes,
            "processor.in_flight_input_bytes",
        )?;
        metrics.stale_results =
            checked_add_u64(metrics.stale_results, 1, "processor.stale_results")?;
        metrics.store_entry_visits = checked_add_u64(
            metrics.store_entry_visits,
            1,
            "processor.store_entry_visits",
        )?;
        self.slot_record_mut(key.slot())
            .and_then(|record| record.in_flight.remove(&key.generation()));
        self.prune_slot(key.slot());
        self.metrics = metrics;
        #[cfg(test)]
        self.assert_usage();
        Ok(true)
    }

    pub fn current_pending(&self, key: &ProcessorRequestKey) -> bool {
        matches!(
            self.state(key.slot()),
            Some(ProcessorSlotState::Pending { key: current }) if current == key
        )
    }

    pub fn record_stale_result(&mut self) -> Result<(), HostError> {
        self.metrics.stale_results =
            checked_add_u64(self.metrics.stale_results, 1, "processor.stale_results")?;
        Ok(())
    }

    pub fn preflight_completion(
        &self,
        key: &ProcessorRequestKey,
        kind: ArtifactChangeKind,
        retained_artifact_bytes: Option<usize>,
    ) -> Result<CompletionReservation, HostError> {
        let lease = self
            .slot_record(key.slot())
            .and_then(|record| record.in_flight.get(&key.generation()))
            .filter(|lease| lease.key == *key)
            .ok_or(HostError::CounterOverflow("processor.in_flight_jobs"))?;
        let change = ArtifactChange::new(key.clone(), kind);
        let queue = self.reserve_changes(std::slice::from_ref(&change))?;
        let mut metrics = self.metrics;
        metrics.in_flight_jobs =
            checked_sub(metrics.in_flight_jobs, 1, "processor.in_flight_jobs")?;
        metrics.in_flight_input_bytes = checked_sub(
            metrics.in_flight_input_bytes,
            lease.input_bytes,
            "processor.in_flight_input_bytes",
        )?;
        if let Some(bytes) = retained_artifact_bytes {
            metrics.retained_artifacts = checked_add(
                metrics.retained_artifacts,
                1,
                "processor.retained_artifacts",
            )?;
            metrics.retained_artifact_bytes = checked_add(
                metrics.retained_artifact_bytes,
                bytes,
                "processor.retained_artifact_bytes",
            )?;
        }
        metrics.accepted_results =
            checked_add_u64(metrics.accepted_results, 1, "processor.accepted_results")?;
        metrics.store_entry_visits = checked_add_u64(
            metrics.store_entry_visits,
            2,
            "processor.store_entry_visits",
        )?;
        apply_queue_reservation(&mut metrics, queue);
        Ok(CompletionReservation { metrics, change })
    }

    pub fn commit_artifact(
        &mut self,
        reservation: CompletionReservation,
        key: ProcessorRequestKey,
        artifact: ProcessorArtifact,
    ) {
        let record = self
            .slot_record_mut(key.slot())
            .expect("a preflighted completion retains its slot");
        record.in_flight.remove(&key.generation());
        record.state = Some(ProcessorSlotState::Ready {
            key: key.clone(),
            artifact,
        });
        self.commit(reservation.metrics, vec![reservation.change]);
    }

    pub fn commit_failure(
        &mut self,
        reservation: CompletionReservation,
        key: ProcessorRequestKey,
        failure: ProcessorFailure,
    ) {
        let record = self
            .slot_record_mut(key.slot())
            .expect("a preflighted completion retains its slot");
        record.in_flight.remove(&key.generation());
        record.state = Some(ProcessorSlotState::Failed {
            key: key.clone(),
            failure,
        });
        self.commit(reservation.metrics, vec![reservation.change]);
    }

    pub fn clear(&mut self, reason: ArtifactReleaseReason) -> Result<(), HostError> {
        let changes = self
            .nodes
            .values()
            .flat_map(|node| node.slots.values())
            .filter_map(|record| record.state.as_ref())
            .map(|state| removed_change(state, reason))
            .collect::<Vec<_>>();
        let queue = self.reserve_changes(&changes)?;
        let slot_visits = u64::try_from(
            self.nodes
                .values()
                .map(|node| node.slots.len())
                .sum::<usize>(),
        )
        .map_err(|_| HostError::CounterOverflow("processor.store_entry_visits"))?;
        let lease_visits = u64::try_from(self.metrics.in_flight_jobs)
            .map_err(|_| HostError::CounterOverflow("processor.store_entry_visits"))?;
        let mut metrics = self.metrics;
        metrics.slots = 0;
        metrics.in_flight_jobs = 0;
        metrics.in_flight_input_bytes = 0;
        metrics.retained_artifacts = 0;
        metrics.retained_artifact_bytes = 0;
        metrics.released_artifacts = checked_add_u64(
            metrics.released_artifacts,
            u64::try_from(self.metrics.retained_artifacts)
                .map_err(|_| HostError::CounterOverflow("processor.released_artifacts"))?,
            "processor.released_artifacts",
        )?;
        metrics.store_entry_visits = checked_add_u64(
            metrics.store_entry_visits,
            slot_visits
                .checked_add(lease_visits)
                .ok_or(HostError::CounterOverflow("processor.store_entry_visits"))?,
            "processor.store_entry_visits",
        )?;
        apply_queue_reservation(&mut metrics, queue);

        let nodes = std::mem::take(&mut self.nodes);
        for lease in nodes
            .into_values()
            .flat_map(|node| node.slots.into_values())
            .flat_map(|record| record.in_flight.into_values())
        {
            lease.cancellation.cancel();
        }
        self.commit(metrics, changes);
        Ok(())
    }

    pub fn remove_node(
        &mut self,
        epoch: Epoch,
        node_id: NodeId,
        reason: ArtifactReleaseReason,
    ) -> Result<(), HostError> {
        self.remove_node_batch(&[(ProcessorNodeKey::new(epoch, node_id), reason)])
    }

    pub fn remove_slot_for_stale_result(
        &mut self,
        slot: &ProcessorSlotKey,
        reason: ArtifactReleaseReason,
    ) -> Result<(), HostError> {
        let Some(record) = self.slot_record(slot) else {
            return Ok(());
        };
        let changes = record
            .state
            .as_ref()
            .map(|state| vec![removed_change(state, reason)])
            .unwrap_or_default();
        let queue = self.reserve_changes(&changes)?;
        let mut metrics = self.metrics;
        apply_record_removal(&mut metrics, record)?;
        metrics.store_entry_visits = checked_add_u64(
            metrics.store_entry_visits,
            u64::try_from(record.in_flight.len().saturating_add(1))
                .map_err(|_| HostError::CounterOverflow("processor.store_entry_visits"))?,
            "processor.store_entry_visits",
        )?;
        metrics.stale_results =
            checked_add_u64(metrics.stale_results, 1, "processor.stale_results")?;
        apply_queue_reservation(&mut metrics, queue);

        if let Some(record) = self.remove_slot_record(slot) {
            for lease in record.in_flight.into_values() {
                lease.cancellation.cancel();
            }
        }
        self.commit(metrics, changes);
        Ok(())
    }

    pub fn reconcile_nodes(
        &mut self,
        epoch: Epoch,
        removed_nodes: &[NodeId],
        changed_inputs: &[(NodeId, ProcessorInputVersion)],
    ) -> Result<(), HostError> {
        let removed = removed_nodes.iter().copied().collect::<BTreeSet<_>>();
        let changed = changed_inputs
            .iter()
            .filter(|(node_id, _)| !removed.contains(node_id))
            .cloned()
            .collect::<BTreeMap<_, _>>();
        let removed_targets = removed
            .into_iter()
            .map(|node_id| ProcessorNodeKey::new(epoch, node_id))
            .collect::<Vec<_>>();
        let changed_targets = changed
            .into_iter()
            .map(|(node_id, version)| (ProcessorNodeKey::new(epoch, node_id), version))
            .collect::<Vec<_>>();

        let changes = removed_targets
            .iter()
            .filter_map(|key| self.nodes.get(key))
            .flat_map(|node| node.slots.values())
            .filter_map(|record| record.state.as_ref())
            .map(|state| removed_change(state, ArtifactReleaseReason::NodeRemoved))
            .chain(changed_targets.iter().flat_map(|(key, current_version)| {
                self.nodes
                    .get(key)
                    .into_iter()
                    .flat_map(|node| node.slots.values())
                    .filter_map(|record| record.state.as_ref())
                    .filter(move |state| state.key().input_version() != current_version)
                    .map(|state| removed_change(state, ArtifactReleaseReason::NodeChanged))
            }))
            .collect::<Vec<_>>();
        let queue = self.reserve_changes(&changes)?;
        let mut metrics = self.metrics;
        let mut visits = 0_u64;

        for key in &removed_targets {
            let Some(node) = self.nodes.get(key) else {
                continue;
            };
            for record in node.slots.values() {
                apply_record_removal(&mut metrics, record)?;
                visits = checked_record_visits(visits, record)?;
            }
        }
        for (key, current_version) in &changed_targets {
            let Some(node) = self.nodes.get(key) else {
                continue;
            };
            for record in node.slots.values() {
                if let Some(state) = record
                    .state
                    .as_ref()
                    .filter(|state| state.key().input_version() != current_version)
                {
                    apply_state_removal(&mut metrics, state)?;
                }
                for lease in record
                    .in_flight
                    .values()
                    .filter(|lease| lease.key.input_version() != current_version)
                {
                    apply_lease_removal(&mut metrics, lease)?;
                }
                visits = checked_record_visits(visits, record)?;
            }
        }
        metrics.store_entry_visits = checked_add_u64(
            metrics.store_entry_visits,
            visits,
            "processor.store_entry_visits",
        )?;
        apply_queue_reservation(&mut metrics, queue);

        for key in &removed_targets {
            if let Some(node) = self.nodes.remove(key) {
                cancel_node_leases(node);
            }
        }
        for (key, current_version) in &changed_targets {
            let remove_node = if let Some(node) = self.nodes.get_mut(key) {
                node.slots.retain(|_, record| {
                    if record
                        .state
                        .as_ref()
                        .is_some_and(|state| state.key().input_version() != current_version)
                    {
                        record.state = None;
                    }
                    record.in_flight.retain(|_, lease| {
                        let keep = lease.key.input_version() == current_version;
                        if !keep {
                            lease.cancellation.cancel();
                        }
                        keep
                    });
                    record.state.is_some() || !record.in_flight.is_empty()
                });
                node.slots.is_empty()
            } else {
                false
            };
            if remove_node {
                self.nodes.remove(key);
            }
        }
        self.commit(metrics, changes);
        Ok(())
    }

    pub fn cancel(&mut self, key: &ProcessorRequestKey) -> Result<bool, HostError> {
        let Some(record) = self.slot_record(key.slot()) else {
            return Ok(false);
        };
        let has_lease = record
            .in_flight
            .get(&key.generation())
            .is_some_and(|lease| lease.key == *key);
        let is_current = record
            .state
            .as_ref()
            .is_some_and(|state| state.key() == key);
        if !has_lease && !is_current {
            return Ok(false);
        }
        let changes = if is_current {
            record
                .state
                .as_ref()
                .map(|state| vec![removed_change(state, ArtifactReleaseReason::Cancelled)])
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let queue = self.reserve_changes(&changes)?;
        let mut metrics = self.metrics;
        if let Some(lease) = record
            .in_flight
            .get(&key.generation())
            .filter(|lease| lease.key == *key)
        {
            metrics.in_flight_jobs =
                checked_sub(metrics.in_flight_jobs, 1, "processor.in_flight_jobs")?;
            metrics.in_flight_input_bytes = checked_sub(
                metrics.in_flight_input_bytes,
                lease.input_bytes,
                "processor.in_flight_input_bytes",
            )?;
        }
        if is_current {
            apply_state_removal(
                &mut metrics,
                record.state.as_ref().expect("checked current state"),
            )?;
        }
        metrics.store_entry_visits = checked_add_u64(
            metrics.store_entry_visits,
            u64::from(has_lease) + u64::from(is_current),
            "processor.store_entry_visits",
        )?;
        apply_queue_reservation(&mut metrics, queue);

        if let Some(record) = self.slot_record_mut(key.slot()) {
            if has_lease {
                if let Some(lease) = record.in_flight.remove(&key.generation()) {
                    lease.cancellation.cancel();
                }
            }
            if is_current {
                record.state = None;
            }
        }
        self.prune_slot(key.slot());
        self.commit(metrics, changes);
        Ok(true)
    }

    pub fn take_changes(&mut self) -> Vec<ArtifactChange> {
        self.metrics.pending_changes = 0;
        self.metrics.pending_change_bytes = 0;
        let changes = std::mem::take(&mut self.changes);
        #[cfg(test)]
        self.assert_usage();
        changes
    }

    fn remove_node_batch(
        &mut self,
        targets: &[(ProcessorNodeKey, ArtifactReleaseReason)],
    ) -> Result<(), HostError> {
        let changes = targets
            .iter()
            .filter_map(|(key, reason)| self.nodes.get(key).map(|node| (node, *reason)))
            .flat_map(|(node, reason)| {
                node.slots
                    .values()
                    .filter_map(move |record| record.state.as_ref().map(|state| (state, reason)))
            })
            .map(|(state, reason)| removed_change(state, reason))
            .collect::<Vec<_>>();
        let queue = self.reserve_changes(&changes)?;
        let mut metrics = self.metrics;
        let mut visits = 0_u64;
        for (key, _) in targets {
            let Some(node) = self.nodes.get(key) else {
                continue;
            };
            for record in node.slots.values() {
                apply_record_removal(&mut metrics, record)?;
                visits = visits
                    .checked_add(
                        u64::try_from(record.in_flight.len().saturating_add(1)).map_err(|_| {
                            HostError::CounterOverflow("processor.store_entry_visits")
                        })?,
                    )
                    .ok_or(HostError::CounterOverflow("processor.store_entry_visits"))?;
            }
        }
        metrics.store_entry_visits = checked_add_u64(
            metrics.store_entry_visits,
            visits,
            "processor.store_entry_visits",
        )?;
        apply_queue_reservation(&mut metrics, queue);

        for (key, _) in targets {
            if let Some(node) = self.nodes.remove(key) {
                for lease in node
                    .slots
                    .into_values()
                    .flat_map(|record| record.in_flight.into_values())
                {
                    lease.cancellation.cancel();
                }
            }
        }
        self.commit(metrics, changes);
        Ok(())
    }

    fn reserve_changes(&self, changes: &[ArtifactChange]) -> Result<QueueReservation, HostError> {
        let pending_changes = checked_add(
            self.metrics.pending_changes,
            changes.len(),
            "processor.pending_changes",
        )?;
        check_limit(
            "processor.pending_changes",
            self.max_pending_changes,
            pending_changes,
        )?;
        let added_bytes = checked_change_bytes(changes)?;
        let pending_change_bytes = checked_add(
            self.metrics.pending_change_bytes,
            added_bytes,
            "processor.pending_change_bytes",
        )?;
        check_limit(
            "processor.pending_change_bytes",
            self.max_pending_change_bytes,
            pending_change_bytes,
        )?;
        Ok(QueueReservation {
            pending_changes,
            pending_change_bytes,
        })
    }

    fn commit(&mut self, metrics: ProcessorMetrics, changes: Vec<ArtifactChange>) {
        self.metrics = metrics;
        self.changes.extend(changes);
        #[cfg(test)]
        self.assert_usage();
    }

    fn slot_record(&self, slot: &ProcessorSlotKey) -> Option<&SlotRecord> {
        self.nodes
            .get(&ProcessorNodeKey::from_slot(slot))?
            .slots
            .get(slot.processor_id())
    }

    fn slot_record_mut(&mut self, slot: &ProcessorSlotKey) -> Option<&mut SlotRecord> {
        self.nodes
            .get_mut(&ProcessorNodeKey::from_slot(slot))?
            .slots
            .get_mut(slot.processor_id())
    }

    fn slot_record_mut_or_insert(&mut self, slot: &ProcessorSlotKey) -> &mut SlotRecord {
        self.nodes
            .entry(ProcessorNodeKey::from_slot(slot))
            .or_default()
            .slots
            .entry(slot.processor_id().clone())
            .or_default()
    }

    fn remove_slot_record(&mut self, slot: &ProcessorSlotKey) -> Option<SlotRecord> {
        let node_key = ProcessorNodeKey::from_slot(slot);
        let record = self
            .nodes
            .get_mut(&node_key)?
            .slots
            .remove(slot.processor_id());
        if self
            .nodes
            .get(&node_key)
            .is_some_and(|node| node.slots.is_empty())
        {
            self.nodes.remove(&node_key);
        }
        record
    }

    fn prune_slot(&mut self, slot: &ProcessorSlotKey) {
        let should_remove = self
            .slot_record(slot)
            .is_some_and(|record| record.state.is_none() && record.in_flight.is_empty());
        if should_remove {
            self.remove_slot_record(slot);
        }
    }

    #[cfg(test)]
    fn recompute_usage(&self) -> (usize, usize, usize, usize, usize) {
        let records = self
            .nodes
            .values()
            .flat_map(|node| node.slots.values())
            .collect::<Vec<_>>();
        let slots = records
            .iter()
            .filter(|record| record.state.is_some())
            .count();
        let in_flight_jobs = records.iter().map(|record| record.in_flight.len()).sum();
        let in_flight_input_bytes = records
            .iter()
            .flat_map(|record| record.in_flight.values())
            .map(|lease| lease.input_bytes)
            .sum();
        let retained = records
            .iter()
            .filter_map(|record| record.state.as_ref()?.artifact())
            .collect::<Vec<_>>();
        let retained_artifacts = retained.len();
        let retained_artifact_bytes = retained.iter().map(|artifact| artifact.byte_len()).sum();
        (
            slots,
            in_flight_jobs,
            in_flight_input_bytes,
            retained_artifacts,
            retained_artifact_bytes,
        )
    }

    #[cfg(test)]
    fn assert_usage(&self) {
        let (slots, jobs, input_bytes, artifacts, artifact_bytes) = self.recompute_usage();
        assert_eq!(self.metrics.slots, slots);
        assert_eq!(self.metrics.in_flight_jobs, jobs);
        assert_eq!(self.metrics.in_flight_input_bytes, input_bytes);
        assert_eq!(self.metrics.retained_artifacts, artifacts);
        assert_eq!(self.metrics.retained_artifact_bytes, artifact_bytes);
        assert_eq!(self.metrics.pending_changes, self.changes.len());
        assert_eq!(
            self.metrics.pending_change_bytes,
            checked_change_bytes(&self.changes).unwrap()
        );
    }
}

fn apply_record_removal(
    metrics: &mut ProcessorMetrics,
    record: &SlotRecord,
) -> Result<(), HostError> {
    let lease_bytes = checked_sum(record.in_flight.values().map(|lease| lease.input_bytes)).ok_or(
        HostError::CounterOverflow("processor.in_flight_input_bytes"),
    )?;
    metrics.in_flight_jobs = checked_sub(
        metrics.in_flight_jobs,
        record.in_flight.len(),
        "processor.in_flight_jobs",
    )?;
    metrics.in_flight_input_bytes = checked_sub(
        metrics.in_flight_input_bytes,
        lease_bytes,
        "processor.in_flight_input_bytes",
    )?;
    if let Some(state) = &record.state {
        apply_state_removal(metrics, state)?;
    }
    Ok(())
}

fn apply_lease_removal(
    metrics: &mut ProcessorMetrics,
    lease: &InFlightLease,
) -> Result<(), HostError> {
    metrics.in_flight_jobs = checked_sub(metrics.in_flight_jobs, 1, "processor.in_flight_jobs")?;
    metrics.in_flight_input_bytes = checked_sub(
        metrics.in_flight_input_bytes,
        lease.input_bytes,
        "processor.in_flight_input_bytes",
    )?;
    Ok(())
}

fn checked_record_visits(visits: u64, record: &SlotRecord) -> Result<u64, HostError> {
    visits
        .checked_add(
            u64::try_from(record.in_flight.len().saturating_add(1))
                .map_err(|_| HostError::CounterOverflow("processor.store_entry_visits"))?,
        )
        .ok_or(HostError::CounterOverflow("processor.store_entry_visits"))
}

fn cancel_node_leases(node: NodeBucket) {
    for lease in node
        .slots
        .into_values()
        .flat_map(|record| record.in_flight.into_values())
    {
        lease.cancellation.cancel();
    }
}

fn apply_state_removal(
    metrics: &mut ProcessorMetrics,
    state: &ProcessorSlotState,
) -> Result<(), HostError> {
    metrics.slots = checked_sub(metrics.slots, 1, "processor.slots")?;
    if let Some(artifact) = state.artifact() {
        metrics.retained_artifacts = checked_sub(
            metrics.retained_artifacts,
            1,
            "processor.retained_artifacts",
        )?;
        metrics.retained_artifact_bytes = checked_sub(
            metrics.retained_artifact_bytes,
            artifact.byte_len(),
            "processor.retained_artifact_bytes",
        )?;
        metrics.released_artifacts = checked_add_u64(
            metrics.released_artifacts,
            1,
            "processor.released_artifacts",
        )?;
    }
    Ok(())
}

fn apply_queue_reservation(metrics: &mut ProcessorMetrics, queue: QueueReservation) {
    metrics.pending_changes = queue.pending_changes;
    metrics.pending_change_bytes = queue.pending_change_bytes;
}

fn removed_change(state: &ProcessorSlotState, reason: ArtifactReleaseReason) -> ArtifactChange {
    ArtifactChange::new(
        state.key().clone(),
        ArtifactChangeKind::Removed {
            reason,
            released_artifact_bytes: state.artifact().map_or(0, ProcessorArtifact::byte_len),
        },
    )
}

fn checked_change_bytes(changes: &[ArtifactChange]) -> Result<usize, HostError> {
    checked_sum(
        changes
            .iter()
            .map(|change| change.checked_byte_len().unwrap_or(usize::MAX)),
    )
    .ok_or(HostError::CounterOverflow("processor.pending_change_bytes"))
}

fn checked_sum(mut values: impl Iterator<Item = usize>) -> Option<usize> {
    values.try_fold(0_usize, |total, value| total.checked_add(value))
}

fn checked_add(value: usize, amount: usize, field: &'static str) -> Result<usize, HostError> {
    value
        .checked_add(amount)
        .ok_or(HostError::CounterOverflow(field))
}

fn checked_sub(value: usize, amount: usize, field: &'static str) -> Result<usize, HostError> {
    value
        .checked_sub(amount)
        .ok_or(HostError::CounterOverflow(field))
}

fn checked_add_u64(value: u64, amount: u64, field: &'static str) -> Result<u64, HostError> {
    value
        .checked_add(amount)
        .ok_or(HostError::CounterOverflow(field))
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
