use std::collections::BTreeSet;

use mdstream_protocol::{
    ChangePayloadCost, ContentKind, Document, Epoch, NodeId, NodeStability, ProjectionOp,
    ProtocolLimits, ResourceId, SourceCursor,
};

use super::{
    checkpoints::CheckpointGate,
    custom::{CustomStartContext, PendingCustomState},
    frontier::{append_closes_structure, stable_root_prefix},
    identity::{IdentityCommit, IdentityLedger},
    markdown::{DraftUsage, compile_markdown_with_custom},
    metrics::{CompileObservation, add_metric_bytes, compile_metrics},
    operations::{OperationSink, collect_resources, incremental_operations},
    reconcile::{collect_frontier_nodes, reconcile_frontier},
    types::{CompilerError, CompilerMetrics, CustomBlockSpec},
};

#[derive(Debug, Default)]
pub(crate) struct ContentCompiler {
    custom_blocks: Vec<CustomBlockSpec>,
    limits: ProtocolLimits,
    identity: IdentityLedger,
    checkpoints: CheckpointGate,
    frontier: CompilerFrontier,
    stable_root_count: usize,
    frontier_resources: BTreeSet<ResourceId>,
    stable_resources: BTreeSet<ResourceId>,
    metrics: CompilerMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompilerFrontier {
    start: SourceCursor,
    custom_start_context: CustomStartContext,
    pending_custom: Option<PendingCustomState>,
}

#[derive(Debug, Clone, Copy)]
struct AppendObservation {
    structural_source_bytes: usize,
    pending_custom: Option<PendingCustomState>,
}

impl Default for CompilerFrontier {
    fn default() -> Self {
        Self {
            start: SourceCursor::new(0),
            custom_start_context: CustomStartContext::DocumentStart,
            pending_custom: None,
        }
    }
}

pub(crate) struct CompilerTransition {
    operations: Vec<ProjectionOp>,
    commit: CompilerCommit,
}

pub(crate) struct CompilerCommit {
    identity: IdentityCommit,
    checkpoints: CheckpointGate,
    frontier: CompilerFrontier,
    stable_root_count: usize,
    frontier_resources: Option<BTreeSet<ResourceId>>,
    promoted_resources: BTreeSet<ResourceId>,
    metrics: CompilerMetrics,
}

impl ContentCompiler {
    pub(crate) fn with_custom_blocks(
        custom_blocks: Vec<CustomBlockSpec>,
        limits: ProtocolLimits,
    ) -> Self {
        Self {
            custom_blocks,
            limits,
            ..Self::default()
        }
    }

    pub(crate) fn stage(
        &self,
        document: Option<&Document>,
        epoch: Epoch,
        suffix: &str,
        finishing: bool,
    ) -> Result<CompilerTransition, CompilerError> {
        let current_cursor = document.map_or(SourceCursor::new(0), |document| {
            document.coordinate().source_cursor
        });
        let suffix_len = u64::try_from(suffix.len()).map_err(|_| CompilerError::CursorOverflow)?;
        let revision = current_cursor
            .checked_add(suffix_len)
            .ok_or(CompilerError::CursorOverflow)?;
        let frontier_bytes = revision
            .get()
            .checked_sub(self.frontier.start.get())
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(CompilerError::InvalidSourceBoundary(self.frontier.start))?;
        let has_projection =
            document.is_some_and(|document| document.roots().len() > self.stable_root_count);
        let suffix_starts_content = suffix.chars().any(|character| !character.is_whitespace());
        let needs_initial_projection = !has_projection
            && frontier_bytes > 0
            && (self.frontier.start == current_cursor || suffix_starts_content);
        let (structural_boundary, structural_source_bytes, next_pending_custom) =
            append_closes_structure(
                document,
                self.stable_root_count,
                suffix,
                &self.custom_blocks,
                self.frontier.pending_custom,
            );
        let observation = AppendObservation {
            structural_source_bytes,
            pending_custom: next_pending_custom,
        };

        let transition = if self
            .checkpoints
            .reason(
                revision,
                frontier_bytes,
                needs_initial_projection,
                structural_boundary,
                finishing,
            )
            .is_some()
        {
            self.stage_compile(
                document,
                epoch,
                suffix,
                revision,
                finishing,
                observation.structural_source_bytes,
            )?
        } else if finishing {
            self.stage_stabilize(document, revision, observation.structural_source_bytes)?
        } else {
            let projection_cursor = document.map_or(SourceCursor::new(0), |document| {
                document.projection_cursor()
            });
            if projection_cursor != current_cursor {
                self.stage_deferred(suffix, frontier_bytes, observation)?
            } else {
                self.stage_incremental(
                    document,
                    suffix,
                    current_cursor,
                    revision,
                    frontier_bytes,
                    observation,
                )?
            }
        };
        Ok(transition)
    }

    pub(crate) fn commit(&mut self, commit: CompilerCommit) {
        self.identity.commit(commit.identity);
        self.checkpoints = commit.checkpoints;
        self.frontier = commit.frontier;
        self.stable_root_count = commit.stable_root_count;
        if let Some(resources) = commit.frontier_resources {
            self.frontier_resources = resources;
        }
        self.stable_resources.extend(commit.promoted_resources);
        self.metrics = commit.metrics;
    }

    pub(crate) fn metrics(&self) -> CompilerMetrics {
        self.metrics
    }

    pub(crate) fn reset(&mut self) {
        self.identity = IdentityLedger::default();
        self.checkpoints.reset();
        self.frontier = CompilerFrontier::default();
        self.stable_root_count = 0;
        self.frontier_resources.clear();
        self.stable_resources.clear();
        self.metrics = CompilerMetrics::default();
    }

    fn stage_compile(
        &self,
        document: Option<&Document>,
        epoch: Epoch,
        suffix: &str,
        revision: SourceCursor,
        finishing: bool,
        structural_source_bytes: usize,
    ) -> Result<CompilerTransition, CompilerError> {
        let projection_cursor = document.map_or(SourceCursor::new(0), |document| {
            document.projection_cursor()
        });
        let reserved_tail = usize::from(projection_cursor != revision)
            .checked_add(usize::from(finishing))
            .ok_or(CompilerError::CursorOverflow)?;
        let mut operations = OperationSink::new(self.limits, reserved_tail)?;
        let frontier_start = usize::try_from(self.frontier.start.get())
            .map_err(|_| CompilerError::InvalidSourceBoundary(self.frontier.start))?;
        let retained = document.map_or("", Document::source);
        let retained_frontier = retained
            .get(frontier_start..)
            .ok_or(CompilerError::InvalidSourceBoundary(self.frontier.start))?;
        let mut source = String::with_capacity(
            retained_frontier
                .len()
                .checked_add(suffix.len())
                .ok_or(CompilerError::CursorOverflow)?,
        );
        source.push_str(retained_frontier);
        source.push_str(suffix);

        let baseline = preserved_draft_usage(
            document,
            self.stable_root_count,
            &self.frontier_resources,
            &self.stable_resources,
            self.limits,
        )?;
        let compilation = compile_markdown_with_custom(
            &source,
            self.frontier.start,
            &self.custom_blocks,
            self.limits,
            baseline,
            self.frontier.custom_start_context,
            finishing,
        )?;
        let pending_custom = compilation.pending_custom;
        let draft = compilation.forest;
        let stable_draft_roots =
            stable_root_prefix(&draft, &source, self.frontier.start, finishing)?;
        let (candidate, identity) = self.identity.stage(epoch, &draft, stable_draft_roots)?;

        let stable_from_candidate = collect_resources(
            &candidate,
            candidate.roots.iter().take(stable_draft_roots).copied(),
        )?;
        let frontier_from_candidate = collect_resources(
            &candidate,
            candidate.roots.iter().skip(stable_draft_roots).copied(),
        )?;
        let reconciled = reconcile_frontier(
            document,
            self.stable_root_count,
            &self.frontier_resources,
            &self.stable_resources,
            &stable_from_candidate,
            &candidate,
            &mut operations,
        )?;
        if projection_cursor != revision {
            operations.push_tail_with(|| ProjectionOp::AdvanceProjection {
                expected_cursor: projection_cursor,
                new_cursor: revision,
            });
        }
        if finishing {
            operations.push_tail_with(|| ProjectionOp::FinishDocument);
        }

        let next_frontier_start = if let Some(root) = draft.roots.get(stable_draft_roots) {
            physical_line_start(&source, self.frontier.start, root.source.start)?
        } else if !finishing {
            unfinished_unclaimed_line_start(
                &source,
                self.frontier.start,
                draft.roots.last().map(|root| root.source.end),
            )?
            .unwrap_or(revision)
        } else {
            revision
        };
        let remaining_frontier = revision
            .get()
            .checked_sub(next_frontier_start.get())
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(CompilerError::InvalidSourceBoundary(next_frontier_start))?;
        let next_custom_start_context = if finishing {
            CustomStartContext::AfterNonBlankLine
        } else {
            custom_start_context_after(
                &source,
                self.frontier.start,
                self.frontier.custom_start_context,
                next_frontier_start,
            )?
        };
        let mut checkpoints = self.checkpoints;
        checkpoints.record_compile(revision, remaining_frontier);

        let metrics = compile_metrics(
            self.metrics,
            CompileObservation {
                parse_passes: compilation.parse_passes,
                parsed_source_bytes: compilation.parsed_source_bytes,
                custom_scan_source_bytes: compilation.custom_scan_source_bytes,
                structural_source_bytes,
                frontier_bytes: remaining_frontier,
                next_checkpoint: checkpoints.next_checkpoint(),
                reconciled: reconciled.metrics,
            },
        )?;
        let stable_root_count = self
            .stable_root_count
            .checked_add(stable_draft_roots)
            .ok_or(CompilerError::MetricsOverflow("stable roots"))?;

        Ok(CompilerTransition {
            operations: operations.into_operations(),
            commit: CompilerCommit {
                identity,
                checkpoints,
                frontier: CompilerFrontier {
                    start: next_frontier_start,
                    custom_start_context: next_custom_start_context,
                    pending_custom,
                },
                stable_root_count,
                frontier_resources: Some(frontier_from_candidate),
                promoted_resources: stable_from_candidate,
                metrics,
            },
        })
    }

    fn stage_stabilize(
        &self,
        document: Option<&Document>,
        revision: SourceCursor,
        structural_source_bytes: usize,
    ) -> Result<CompilerTransition, CompilerError> {
        let mut operations = OperationSink::new(self.limits, 1)?;
        let mut visits = 0_u64;
        if let Some(document) = document {
            let roots = document
                .roots()
                .iter()
                .skip(self.stable_root_count)
                .copied();
            let ids = collect_frontier_nodes(Some(document), roots)?;
            for id in ids {
                visits = visits
                    .checked_add(1)
                    .ok_or(CompilerError::MetricsOverflow("incremental projections"))?;
                let node = document.node(id).ok_or_else(|| {
                    CompilerError::InvalidReconciliation(format!("missing node {id}"))
                })?;
                if node.stability == NodeStability::Stable {
                    continue;
                }
                let permit = operations.reserve(ChangePayloadCost::ZERO)?;
                let mut stable = node.projection();
                stable.stability = NodeStability::Stable;
                stable.version = stable.derived_version();
                permit.commit(ProjectionOp::StabilizeNode {
                    node_id: id,
                    expected_version: node.version.clone(),
                    new_version: stable.version,
                });
            }
        }
        operations.push_tail_with(|| ProjectionOp::FinishDocument);

        let mut checkpoints = self.checkpoints;
        checkpoints.record_compile(revision, 0);
        let mut metrics = self.metrics;
        metrics.structural_source_bytes = add_metric_bytes(
            metrics.structural_source_bytes,
            structural_source_bytes,
            "structural source bytes",
        )?;
        metrics.incremental_projection_visits = metrics
            .incremental_projection_visits
            .checked_add(visits)
            .ok_or(CompilerError::MetricsOverflow("incremental projections"))?;
        metrics.frontier_bytes = 0;
        metrics.next_checkpoint = checkpoints.next_checkpoint();
        let stable_root_count = document.map_or(0, |document| document.roots().len());
        Ok(CompilerTransition {
            operations: operations.into_operations(),
            commit: CompilerCommit {
                identity: IdentityCommit::default(),
                checkpoints,
                frontier: CompilerFrontier {
                    start: revision,
                    custom_start_context: CustomStartContext::AfterNonBlankLine,
                    pending_custom: None,
                },
                stable_root_count,
                frontier_resources: Some(BTreeSet::new()),
                promoted_resources: self.frontier_resources.clone(),
                metrics,
            },
        })
    }

    fn stage_incremental(
        &self,
        document: Option<&Document>,
        suffix: &str,
        current_cursor: SourceCursor,
        revision: SourceCursor,
        frontier_bytes: usize,
        observation: AppendObservation,
    ) -> Result<CompilerTransition, CompilerError> {
        let mut operations = OperationSink::new(self.limits, 1)?;
        let Some(visits) = incremental_operations(
            document,
            self.stable_root_count,
            suffix,
            current_cursor,
            revision,
            &mut operations,
        )?
        else {
            return self.stage_deferred(suffix, frontier_bytes, observation);
        };
        operations.push_tail_with(|| ProjectionOp::AdvanceProjection {
            expected_cursor: current_cursor,
            new_cursor: revision,
        });
        let mut metrics = self.metrics;
        metrics.structural_source_bytes = add_metric_bytes(
            metrics.structural_source_bytes,
            observation.structural_source_bytes,
            "structural source bytes",
        )?;
        metrics.incremental_projection_visits = metrics
            .incremental_projection_visits
            .checked_add(visits)
            .ok_or(CompilerError::MetricsOverflow("incremental projections"))?;
        metrics.frontier_bytes = frontier_bytes;
        metrics.next_checkpoint = self.checkpoints.next_checkpoint();

        Ok(CompilerTransition {
            operations: operations.into_operations(),
            commit: CompilerCommit {
                identity: IdentityCommit::default(),
                checkpoints: self.checkpoints,
                frontier: CompilerFrontier {
                    pending_custom: observation.pending_custom,
                    ..self.frontier
                },
                stable_root_count: self.stable_root_count,
                frontier_resources: None,
                promoted_resources: BTreeSet::new(),
                metrics,
            },
        })
    }

    fn stage_deferred(
        &self,
        suffix: &str,
        frontier_bytes: usize,
        observation: AppendObservation,
    ) -> Result<CompilerTransition, CompilerError> {
        let mut metrics = self.metrics;
        metrics.structural_source_bytes = add_metric_bytes(
            metrics.structural_source_bytes,
            observation.structural_source_bytes,
            "structural source bytes",
        )?;
        metrics.deferred_source_bytes = add_metric_bytes(
            metrics.deferred_source_bytes,
            suffix.len(),
            "deferred source bytes",
        )?;
        metrics.frontier_bytes = frontier_bytes;
        metrics.next_checkpoint = self.checkpoints.next_checkpoint();

        Ok(CompilerTransition {
            operations: Vec::new(),
            commit: CompilerCommit {
                identity: IdentityCommit::default(),
                checkpoints: self.checkpoints,
                frontier: CompilerFrontier {
                    pending_custom: observation.pending_custom,
                    ..self.frontier
                },
                stable_root_count: self.stable_root_count,
                frontier_resources: None,
                promoted_resources: BTreeSet::new(),
                metrics,
            },
        })
    }
}

impl CompilerTransition {
    pub(crate) fn into_parts(self) -> (Vec<ProjectionOp>, CompilerCommit) {
        (self.operations, self.commit)
    }
}

fn preserved_draft_usage(
    document: Option<&Document>,
    stable_root_count: usize,
    frontier_resources: &BTreeSet<ResourceId>,
    stable_resources: &BTreeSet<ResourceId>,
    limits: ProtocolLimits,
) -> Result<DraftUsage, CompilerError> {
    let Some(document) = document else {
        return Ok(DraftUsage::default());
    };

    let mut replaced = BTreeSet::<NodeId>::new();
    let mut pending = document
        .roots()
        .iter()
        .skip(stable_root_count)
        .copied()
        .collect::<Vec<_>>();
    if stable_root_count > document.roots().len() {
        return Err(CompilerError::InvalidReconciliation(
            "stable root inventory exceeds the document".to_string(),
        ));
    }
    let mut replaced_structural_items = 0usize;
    let mut replaced_metadata_bytes = 0usize;
    while let Some(id) = pending.pop() {
        if !replaced.insert(id) {
            continue;
        }
        let node = document.node(id).ok_or_else(|| {
            CompilerError::InvalidReconciliation(format!(
                "frontier inventory references missing node {id}"
            ))
        })?;
        let content_items = match &node.content {
            ContentKind::Table { alignments } => alignments.len(),
            _ => 0,
        };
        replaced_structural_items = replaced_structural_items
            .checked_add(1)
            .and_then(|items| items.checked_add(content_items))
            .ok_or_else(|| {
                CompilerError::InvalidReconciliation(
                    "frontier structural inventory overflowed".to_string(),
                )
            })?;
        let metadata_bytes = ChangePayloadCost::for_content(&node.content, limits)
            .map_err(|error| CompilerError::InvalidReconciliation(error.to_string()))?
            .metadata_bytes;
        replaced_metadata_bytes = replaced_metadata_bytes
            .checked_add(metadata_bytes)
            .ok_or_else(|| {
                CompilerError::InvalidReconciliation(
                    "frontier metadata inventory overflowed".to_string(),
                )
            })?;
        pending.extend(node.children.iter().copied());
    }

    let replaceable_resources = frontier_resources
        .difference(stable_resources)
        .filter_map(|id| document.resource(*id))
        .collect::<Vec<_>>();
    let mut replaced_resource_metadata = 0usize;
    for resource in &replaceable_resources {
        let metadata_bytes = ChangePayloadCost::for_resource(resource, limits)
            .map_err(|error| CompilerError::InvalidReconciliation(error.to_string()))?
            .metadata_bytes;
        replaced_resource_metadata = replaced_resource_metadata
            .checked_add(metadata_bytes)
            .ok_or_else(|| {
                CompilerError::InvalidReconciliation(
                    "frontier resource metadata inventory overflowed".to_string(),
                )
            })?;
    }

    let nodes = document
        .nodes()
        .count()
        .checked_sub(replaced.len())
        .ok_or_else(|| {
            CompilerError::InvalidReconciliation(
                "frontier node inventory exceeds the document".to_string(),
            )
        })?;
    let structural_items = document
        .structural_items()
        .checked_sub(replaced_structural_items)
        .ok_or_else(|| {
            CompilerError::InvalidReconciliation(
                "frontier structural inventory exceeds the document".to_string(),
            )
        })?;
    let resources = document
        .resources()
        .len()
        .checked_sub(replaceable_resources.len())
        .ok_or_else(|| {
            CompilerError::InvalidReconciliation(
                "frontier resource inventory exceeds the document".to_string(),
            )
        })?;
    let metadata_bytes = document
        .metadata_bytes()
        .checked_sub(replaced_metadata_bytes)
        .and_then(|bytes| bytes.checked_sub(replaced_resource_metadata))
        .ok_or_else(|| {
            CompilerError::InvalidReconciliation(
                "frontier metadata inventory exceeds the document".to_string(),
            )
        })?;
    Ok(DraftUsage {
        roots: stable_root_count,
        nodes,
        resources,
        structural_items,
        metadata_bytes,
    })
}

fn custom_start_context_after(
    frontier_source: &str,
    absolute_base: SourceCursor,
    current: CustomStartContext,
    next_start: SourceCursor,
) -> Result<CustomStartContext, CompilerError> {
    let relative_start = next_start
        .get()
        .checked_sub(absolute_base.get())
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or(CompilerError::InvalidSourceBoundary(next_start))?;
    if relative_start == 0 {
        return Ok(current);
    }
    let prefix = frontier_source
        .get(..relative_start)
        .ok_or(CompilerError::InvalidSourceBoundary(next_start))?;
    let Some(before_line_ending) = prefix.strip_suffix('\n') else {
        return Err(CompilerError::InvalidSourceBoundary(next_start));
    };
    let previous_line = before_line_ending
        .rsplit_once('\n')
        .map_or(before_line_ending, |(_, line)| line)
        .strip_suffix('\r')
        .unwrap_or_else(|| {
            before_line_ending
                .rsplit_once('\n')
                .map_or(before_line_ending, |(_, line)| line)
        });
    if previous_line
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
    {
        Ok(CustomStartContext::AfterBlankLine)
    } else {
        Ok(CustomStartContext::AfterNonBlankLine)
    }
}

fn physical_line_start(
    frontier_source: &str,
    absolute_base: SourceCursor,
    node_start: SourceCursor,
) -> Result<SourceCursor, CompilerError> {
    let relative_start = node_start
        .get()
        .checked_sub(absolute_base.get())
        .and_then(|offset| usize::try_from(offset).ok())
        .ok_or(CompilerError::InvalidSourceBoundary(node_start))?;
    let prefix = frontier_source
        .get(..relative_start)
        .ok_or(CompilerError::InvalidSourceBoundary(node_start))?;
    let line_start = prefix
        .rfind('\n')
        .map_or(0, |index| index.saturating_add(1));
    absolute_base
        .checked_add(u64::try_from(line_start).map_err(|_| CompilerError::CursorOverflow)?)
        .ok_or(CompilerError::CursorOverflow)
}

fn unfinished_unclaimed_line_start(
    frontier_source: &str,
    absolute_base: SourceCursor,
    last_root_end: Option<SourceCursor>,
) -> Result<Option<SourceCursor>, CompilerError> {
    if frontier_source.ends_with('\n') {
        return Ok(None);
    }
    let line_start = frontier_source
        .rfind('\n')
        .map_or(0, |index| index.saturating_add(1));
    let start = absolute_base
        .checked_add(u64::try_from(line_start).map_err(|_| CompilerError::CursorOverflow)?)
        .ok_or(CompilerError::CursorOverflow)?;
    if last_root_end.is_some_and(|root_end| root_end.get() > start.get()) {
        return Ok(None);
    }
    Ok(Some(start))
}
