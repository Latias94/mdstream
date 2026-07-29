use std::collections::{BTreeMap, BTreeSet};

use crate::compiler::CompilerLimits;
use crate::compiler::draft::{DraftForest, DraftResourceIndex};
use crate::compiler::identity::MaterializedForest;
use crate::compiler::types::CompilerError;
use mdstream_protocol::{
    ChangePayloadCost, Document, NodeId, NodeStability, ProjectionOp, ProtocolLimits, ResourceId,
    SourceCursor,
};

use super::model::{DefinitionFact, DefinitionKey, DefinitionValue};
use super::projection::{
    SemanticCorrection, collect_footnote_definitions, corrected_projection, definition_target,
    dependency_key, enrich_forest,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticState {
    stable_definitions: BTreeMap<DefinitionKey, DefinitionFact>,
    frontier_definitions: BTreeMap<DefinitionKey, DefinitionFact>,
    stable_dependencies: BTreeMap<DefinitionKey, BTreeSet<NodeId>>,
    frontier_dependencies: BTreeMap<DefinitionKey, BTreeSet<NodeId>>,
    stable_definition_metadata_bytes: usize,
    frontier_definition_metadata_bytes: usize,
    stable_dependency_count: usize,
    frontier_dependency_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticCommit {
    promote_frontier: bool,
    stable_definition_inserts: BTreeMap<DefinitionKey, DefinitionFact>,
    frontier_definitions: BTreeMap<DefinitionKey, DefinitionFact>,
    stable_dependency_inserts: BTreeMap<DefinitionKey, BTreeSet<NodeId>>,
    frontier_dependencies: BTreeMap<DefinitionKey, BTreeSet<NodeId>>,
    stable_definition_metadata_bytes: usize,
    frontier_definition_metadata_bytes: usize,
    stable_dependency_count: usize,
    frontier_dependency_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SemanticPlan<'state> {
    state: &'state SemanticState,
    commit: SemanticCommit,
    changed_definition_keys: BTreeSet<DefinitionKey>,
    resource_indices: BTreeMap<DefinitionKey, DraftResourceIndex>,
    definition_visits: u64,
    state_key_visits: u64,
    retained_definition_count: usize,
    retained_definition_metadata_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SemanticWork {
    pub(crate) definition_visits: u64,
    pub(crate) state_key_visits: u64,
    pub(crate) state_edge_visits: u64,
    pub(crate) candidate_node_visits: u64,
    pub(crate) candidate_dependency_visits: u64,
    pub(crate) dependent_visits: u64,
    pub(crate) corrections_emitted: u64,
    pub(crate) retained_definitions: usize,
    pub(crate) retained_dependencies: usize,
    pub(crate) retained_metadata_bytes: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SemanticOutcome {
    pub(crate) commit: SemanticCommit,
    pub(crate) corrections: Vec<SemanticCorrection>,
    pub(crate) stable_resources: BTreeSet<ResourceId>,
    pub(crate) frontier_resources: BTreeSet<ResourceId>,
    pub(crate) work: SemanticWork,
}

pub(super) struct DefinitionView<'state, 'commit> {
    state: &'state SemanticState,
    commit: &'commit SemanticCommit,
}

struct DefinitionStage {
    commit: SemanticCommit,
    changed_keys: BTreeSet<DefinitionKey>,
    state_key_visits: u64,
    retained_definition_count: usize,
    retained_definition_metadata_bytes: usize,
}

impl SemanticState {
    fn definition(&self, key: &DefinitionKey) -> Option<&DefinitionFact> {
        self.stable_definitions
            .get(key)
            .or_else(|| self.frontier_definitions.get(key))
    }

    fn stage_definitions(
        &self,
        mut facts: Vec<DefinitionFact>,
        stable_before: SourceCursor,
        limits: CompilerLimits,
    ) -> Result<DefinitionStage, CompilerError> {
        facts.sort_by_key(|fact| (fact.source.start, fact.source.end));
        let mut stable_inserts = BTreeMap::new();
        let mut frontier = BTreeMap::new();
        let mut stable_metadata_bytes = self.stable_definition_metadata_bytes;
        let mut frontier_metadata_bytes = 0usize;
        for fact in facts {
            if self.stable_definitions.contains_key(&fact.key)
                || stable_inserts.contains_key(&fact.key)
                || frontier.contains_key(&fact.key)
            {
                continue;
            }
            let metadata_bytes = definition_metadata_bytes(&fact)?;
            if fact.source.end.get() <= stable_before.get() {
                stable_metadata_bytes = stable_metadata_bytes
                    .checked_add(metadata_bytes)
                    .ok_or(CompilerError::MetricsOverflow("definition metadata"))?;
                stable_inserts.insert(fact.key.clone(), fact);
            } else {
                frontier_metadata_bytes = frontier_metadata_bytes
                    .checked_add(metadata_bytes)
                    .ok_or(CompilerError::MetricsOverflow("definition metadata"))?;
                frontier.insert(fact.key.clone(), fact);
            }
        }

        let retained_definition_count = self
            .stable_definitions
            .len()
            .checked_add(stable_inserts.len())
            .and_then(|count| count.checked_add(frontier.len()))
            .ok_or(CompilerError::MetricsOverflow(
                "retained semantic definitions",
            ))?;
        let retained_definition_metadata_bytes = stable_metadata_bytes
            .checked_add(frontier_metadata_bytes)
            .ok_or(CompilerError::MetricsOverflow("definition metadata"))?;
        validate_definition_registry(
            retained_definition_count,
            retained_definition_metadata_bytes,
            limits,
        )?;

        let commit = SemanticCommit {
            promote_frontier: false,
            stable_definition_inserts: stable_inserts,
            frontier_definitions: frontier,
            stable_dependency_inserts: BTreeMap::new(),
            frontier_dependencies: BTreeMap::new(),
            stable_definition_metadata_bytes: stable_metadata_bytes,
            frontier_definition_metadata_bytes: frontier_metadata_bytes,
            stable_dependency_count: self.stable_dependency_count,
            frontier_dependency_count: 0,
        };
        let candidate_keys = self
            .frontier_definitions
            .keys()
            .chain(commit.stable_definition_inserts.keys())
            .chain(commit.frontier_definitions.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let state_key_visits = u64::try_from(candidate_keys.len())
            .map_err(|_| CompilerError::MetricsOverflow("semantic state key visits"))?;
        let view = DefinitionView {
            state: self,
            commit: &commit,
        };
        let changed_keys = candidate_keys
            .into_iter()
            .filter(|key| self.definition(key) != view.get(key))
            .collect();
        Ok(DefinitionStage {
            commit,
            changed_keys,
            state_key_visits,
            retained_definition_count,
            retained_definition_metadata_bytes,
        })
    }

    pub(crate) fn prepare<'state>(
        &'state self,
        forest: &mut DraftForest,
        mut facts: Vec<DefinitionFact>,
        stable_before: SourceCursor,
        protocol_limits: ProtocolLimits,
        compiler_limits: CompilerLimits,
    ) -> Result<SemanticPlan<'state>, CompilerError> {
        collect_footnote_definitions(&forest.roots, &mut facts);
        let definition_visits = u64::try_from(facts.len())
            .map_err(|_| CompilerError::MetricsOverflow("semantic definitions"))?;
        validate_definition_facts(&facts, protocol_limits)?;
        let DefinitionStage {
            commit,
            changed_keys: changed_definition_keys,
            mut state_key_visits,
            retained_definition_count,
            retained_definition_metadata_bytes,
        } = self.stage_definitions(facts, stable_before, compiler_limits)?;
        let view = DefinitionView {
            state: self,
            commit: &commit,
        };
        let mut resource_indices = enrich_forest(forest, &view, protocol_limits)?;
        let required_resource_keys = changed_definition_keys
            .iter()
            .chain(commit.stable_definition_inserts.keys())
            .chain(commit.frontier_definitions.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        state_key_visits = state_key_visits
            .checked_add(
                u64::try_from(required_resource_keys.len())
                    .map_err(|_| CompilerError::MetricsOverflow("semantic state key visits"))?,
            )
            .ok_or(CompilerError::MetricsOverflow("semantic state key visits"))?;
        for key in required_resource_keys {
            if self
                .stable_dependencies
                .get(&key)
                .is_none_or(BTreeSet::is_empty)
            {
                continue;
            }
            definition_target(
                &key,
                &view,
                &mut forest.resources,
                &mut resource_indices,
                protocol_limits,
            )?;
        }
        Ok(SemanticPlan {
            state: self,
            commit,
            changed_definition_keys,
            resource_indices,
            definition_visits,
            state_key_visits,
            retained_definition_count,
            retained_definition_metadata_bytes,
        })
    }

    pub(crate) fn commit(&mut self, commit: SemanticCommit) {
        if commit.promote_frontier {
            self.stable_definitions
                .append(&mut self.frontier_definitions);
            for (key, dependencies) in std::mem::take(&mut self.frontier_dependencies) {
                self.stable_dependencies
                    .entry(key)
                    .or_default()
                    .extend(dependencies);
            }
        }
        for (key, definition) in commit.stable_definition_inserts {
            let previous = self.stable_definitions.insert(key, definition);
            debug_assert!(previous.is_none());
        }
        self.frontier_definitions = commit.frontier_definitions;
        for (key, dependencies) in commit.stable_dependency_inserts {
            self.stable_dependencies
                .entry(key)
                .or_default()
                .extend(dependencies);
        }
        self.frontier_dependencies = commit.frontier_dependencies;
        self.stable_definition_metadata_bytes = commit.stable_definition_metadata_bytes;
        self.frontier_definition_metadata_bytes = commit.frontier_definition_metadata_bytes;
        self.stable_dependency_count = commit.stable_dependency_count;
        self.frontier_dependency_count = commit.frontier_dependency_count;
    }

    pub(crate) fn stage_stabilize(&self) -> Result<(SemanticCommit, SemanticWork), CompilerError> {
        let stable_definition_metadata_bytes = self
            .stable_definition_metadata_bytes
            .checked_add(self.frontier_definition_metadata_bytes)
            .ok_or(CompilerError::MetricsOverflow("definition metadata"))?;
        let stable_dependency_count = self
            .stable_dependency_count
            .checked_add(self.frontier_dependency_count)
            .ok_or(CompilerError::MetricsOverflow("definition dependencies"))?;
        let retained_definition_count = self
            .stable_definitions
            .len()
            .checked_add(self.frontier_definitions.len())
            .ok_or(CompilerError::MetricsOverflow(
                "retained semantic definitions",
            ))?;
        let state_key_visits = u64::try_from(self.frontier_definitions.len())
            .map_err(|_| CompilerError::MetricsOverflow("semantic state key visits"))?;
        let state_edge_visits = u64::try_from(self.frontier_dependency_count)
            .map_err(|_| CompilerError::MetricsOverflow("semantic state edge visits"))?;
        let commit = SemanticCommit {
            promote_frontier: true,
            stable_definition_inserts: BTreeMap::new(),
            frontier_definitions: BTreeMap::new(),
            stable_dependency_inserts: BTreeMap::new(),
            frontier_dependencies: BTreeMap::new(),
            stable_definition_metadata_bytes,
            frontier_definition_metadata_bytes: 0,
            stable_dependency_count,
            frontier_dependency_count: 0,
        };
        let work = SemanticWork {
            state_key_visits,
            state_edge_visits,
            retained_definitions: retained_definition_count,
            retained_dependencies: stable_dependency_count,
            retained_metadata_bytes: stable_definition_metadata_bytes,
            ..SemanticWork::default()
        };
        Ok((commit, work))
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

impl DefinitionView<'_, '_> {
    pub(super) fn get(&self, key: &DefinitionKey) -> Option<&DefinitionFact> {
        self.state
            .stable_definitions
            .get(key)
            .or_else(|| self.commit.stable_definition_inserts.get(key))
            .or_else(|| self.commit.frontier_definitions.get(key))
    }
}

impl SemanticPlan<'_> {
    pub(crate) fn finalize(
        mut self,
        document: Option<&Document>,
        candidate: &MaterializedForest,
        protocol_limits: ProtocolLimits,
        compiler_limits: CompilerLimits,
    ) -> Result<SemanticOutcome, CompilerError> {
        let mut candidate_node_visits = 0_u64;
        let mut candidate_dependency_visits = 0_u64;
        let mut candidate_stable_resource_keys = BTreeSet::new();
        for (id, node) in &candidate.nodes {
            candidate_node_visits = candidate_node_visits
                .checked_add(1)
                .ok_or(CompilerError::MetricsOverflow("semantic candidate nodes"))?;
            let Some(key) = dependency_key(&node.projection.content) else {
                continue;
            };
            candidate_dependency_visits = candidate_dependency_visits.checked_add(1).ok_or(
                CompilerError::MetricsOverflow("semantic candidate dependencies"),
            )?;
            let dependencies = if node.projection.stability == NodeStability::Stable {
                candidate_stable_resource_keys.insert(key.clone());
                &mut self.commit.stable_dependency_inserts
            } else {
                &mut self.commit.frontier_dependencies
            };
            dependencies.entry(key).or_default().insert(*id);
        }
        let stable_dependency_inserts = self
            .commit
            .stable_dependency_inserts
            .values()
            .try_fold(0usize, |count, ids| count.checked_add(ids.len()))
            .ok_or(CompilerError::MetricsOverflow("definition dependencies"))?;
        self.commit.stable_dependency_count = self
            .commit
            .stable_dependency_count
            .checked_add(stable_dependency_inserts)
            .ok_or(CompilerError::MetricsOverflow("definition dependencies"))?;
        self.commit.frontier_dependency_count = self
            .commit
            .frontier_dependencies
            .values()
            .try_fold(0usize, |count, ids| count.checked_add(ids.len()))
            .ok_or(CompilerError::MetricsOverflow("definition dependencies"))?;
        let dependency_count = self
            .commit
            .stable_dependency_count
            .checked_add(self.commit.frontier_dependency_count)
            .ok_or(CompilerError::MetricsOverflow("definition dependencies"))?;
        if dependency_count > compiler_limits.max_definition_edges {
            return Err(CompilerError::LimitExceeded {
                field: "definition.dependencies",
                limit: compiler_limits.max_definition_edges,
                actual: dependency_count,
            });
        }

        let mut corrections = Vec::new();
        let mut dependent_visits = 0_u64;
        if let Some(document) = document {
            let view = DefinitionView {
                state: self.state,
                commit: &self.commit,
            };
            for key in &self.changed_definition_keys {
                let Some(dependents) = self.state.stable_dependencies.get(key) else {
                    continue;
                };
                let definition = view.get(key);
                let target = self
                    .resource_indices
                    .get(key)
                    .and_then(|index| candidate.resource_refs.get(index.get()))
                    .cloned();
                for node_id in dependents {
                    dependent_visits = dependent_visits
                        .checked_add(1)
                        .ok_or(CompilerError::MetricsOverflow("semantic dependent visits"))?;
                    let current = document.node(*node_id).ok_or_else(|| {
                        CompilerError::InvalidReconciliation(format!(
                            "semantic dependency references missing node {node_id}"
                        ))
                    })?;
                    let Some(projection) = corrected_projection(
                        current.projection(),
                        key,
                        definition,
                        target.as_ref(),
                    )?
                    else {
                        continue;
                    };
                    let cost = ChangePayloadCost::for_projection(
                        *node_id,
                        &projection,
                        protocol_limits,
                    )
                    .map_err(|error| CompilerError::InvalidReconciliation(error.to_string()))?;
                    corrections.push(SemanticCorrection {
                        cost,
                        operation: ProjectionOp::ReplaceNode {
                            node_id: *node_id,
                            expected_version: current.version.clone(),
                            projection,
                        },
                    });
                }
            }
        }
        let corrections_emitted = u64::try_from(corrections.len())
            .map_err(|_| CompilerError::MetricsOverflow("semantic corrections"))?;

        let mut stable_resources = BTreeSet::new();
        let mut frontier_resources = BTreeSet::new();
        for (key, index) in &self.resource_indices {
            let reference = candidate.resource_refs.get(index.get()).ok_or_else(|| {
                CompilerError::InvalidIdentity(
                    "semantic resource index was not materialized".to_string(),
                )
            })?;
            // Resource retention requires both stable semantic content and a
            // stable IR owner. Either provisional side stays with the frontier.
            let definition_is_stable = self.state.stable_definitions.contains_key(key)
                || self.commit.stable_definition_inserts.contains_key(key);
            let has_stable_owner = candidate_stable_resource_keys.contains(key)
                || self
                    .state
                    .stable_dependencies
                    .get(key)
                    .is_some_and(|dependencies| !dependencies.is_empty());
            if definition_is_stable && has_stable_owner {
                stable_resources.insert(reference.id);
            } else {
                frontier_resources.insert(reference.id);
            }
        }

        Ok(SemanticOutcome {
            commit: self.commit,
            corrections,
            stable_resources,
            frontier_resources,
            work: SemanticWork {
                definition_visits: self.definition_visits,
                state_key_visits: self.state_key_visits,
                state_edge_visits: 0,
                candidate_node_visits,
                candidate_dependency_visits,
                dependent_visits,
                corrections_emitted,
                retained_definitions: self.retained_definition_count,
                retained_dependencies: dependency_count,
                retained_metadata_bytes: self.retained_definition_metadata_bytes,
            },
        })
    }
}

fn validate_definition_facts(
    facts: &[DefinitionFact],
    limits: ProtocolLimits,
) -> Result<(), CompilerError> {
    for fact in facts {
        for (field, value) in [
            (
                "definition.normalized_label",
                Some(fact.key.folded_label.as_str()),
            ),
            ("definition.label", Some(fact.label.as_str())),
            (
                "definition.destination",
                match &fact.value {
                    DefinitionValue::Reference { destination, .. } => Some(destination.as_str()),
                    DefinitionValue::Footnote => None,
                },
            ),
            (
                "definition.title",
                match &fact.value {
                    DefinitionValue::Reference { title, .. } => title.as_deref(),
                    DefinitionValue::Footnote => None,
                },
            ),
        ] {
            let Some(value) = value else {
                continue;
            };
            if value.len() > limits.max_metadata_value_bytes {
                return Err(CompilerError::LimitExceeded {
                    field,
                    limit: limits.max_metadata_value_bytes,
                    actual: value.len(),
                });
            }
        }
    }
    Ok(())
}

fn validate_definition_registry(
    definition_count: usize,
    metadata_bytes: usize,
    limits: CompilerLimits,
) -> Result<(), CompilerError> {
    if definition_count > limits.max_definitions {
        return Err(CompilerError::LimitExceeded {
            field: "definitions",
            limit: limits.max_definitions,
            actual: definition_count,
        });
    }
    if metadata_bytes > limits.max_definition_metadata_bytes {
        return Err(CompilerError::LimitExceeded {
            field: "definition.metadata",
            limit: limits.max_definition_metadata_bytes,
            actual: metadata_bytes,
        });
    }
    Ok(())
}

fn definition_metadata_bytes(definition: &DefinitionFact) -> Result<usize, CompilerError> {
    let key_bytes = definition
        .key
        .folded_label
        .len()
        .checked_mul(2)
        .ok_or(CompilerError::MetricsOverflow("definition metadata"))?;
    let value_bytes = match &definition.value {
        DefinitionValue::Reference { destination, title } => definition
            .label
            .len()
            .checked_add(destination.len())
            .and_then(|bytes| {
                title
                    .as_ref()
                    .map_or(Some(bytes), |title| bytes.checked_add(title.len()))
            }),
        DefinitionValue::Footnote => Some(definition.label.len()),
    }
    .ok_or(CompilerError::MetricsOverflow("definition metadata"))?;
    key_bytes
        .checked_add(value_bytes)
        .ok_or(CompilerError::MetricsOverflow("definition metadata"))
}
