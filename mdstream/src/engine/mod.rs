mod builder;
mod effects;
mod input;
mod lifecycle;
mod limits;
mod storage;
mod work;

pub use crate::compiler::{CompilerError, CompilerMetrics, CustomBlockSpec, MarkdownDiagnostic};
pub use builder::StreamEngineBuilder;
pub use effects::EngineOutput;
pub use lifecycle::EngineError;
pub use limits::EngineLimits;
pub use storage::EngineStorageMetrics;
pub use work::EngineWorkMetrics;

use mdstream_protocol::{
    ApplyOutcome, Coordinate, DocumentLifecycle, Epoch, ProtocolError, ProtocolLimits, Reducer,
    ReducerMetrics, Snapshot,
};

use self::input::NewlineNormalizer;
use self::lifecycle::{append_change, reset_change};
use crate::compiler::ContentCompiler;

#[derive(Debug)]
pub struct StreamEngine {
    normalizer: NewlineNormalizer,
    producer: Reducer,
    initial_epoch: Epoch,
    compiler: ContentCompiler,
    limits: ProtocolLimits,
    engine_limits: EngineLimits,
    work: EngineWorkMetrics,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
/// Retained canonical frontier memory and absolute projection progress.
pub struct EngineMetrics {
    pub retained_input_bytes: usize,
    pub retained_source_base: u64,
    pub compiler: CompilerMetrics,
    pub reducer: ReducerMetrics,
    pub storage: EngineStorageMetrics,
    pub work: EngineWorkMetrics,
}

impl StreamEngine {
    pub fn new() -> Self {
        Self::with_custom_blocks(
            Vec::new(),
            ProtocolLimits::default(),
            EngineLimits::default(),
        )
    }

    #[cfg(test)]
    fn with_limits(limits: ProtocolLimits) -> Self {
        Self::with_custom_blocks(Vec::new(), limits, EngineLimits::default())
    }

    fn with_custom_blocks(
        custom_blocks: Vec<CustomBlockSpec>,
        limits: ProtocolLimits,
        engine_limits: EngineLimits,
    ) -> Self {
        Self {
            normalizer: NewlineNormalizer::default(),
            producer: Reducer::with_limits(limits),
            initial_epoch: Epoch::new(1),
            compiler: ContentCompiler::with_custom_blocks(custom_blocks, limits),
            limits,
            engine_limits,
            work: EngineWorkMetrics::default(),
        }
    }

    pub fn builder() -> StreamEngineBuilder {
        StreamEngineBuilder::default()
    }

    pub fn append(&mut self, chunk: &str) -> Result<EngineOutput, EngineError> {
        self.append_transition(chunk)
    }

    pub fn finish(&mut self) -> Result<EngineOutput, EngineError> {
        self.finish_transition()
    }

    pub fn reset(&mut self) -> Result<EngineOutput, EngineError> {
        let change = reset_change(&self.producer, self.initial_epoch)?;
        let work = EngineWorkMetrics::default().stage(
            &change,
            mdstream_protocol::ChangePayloadCost::ZERO,
            0,
            self.engine_limits,
        )?;
        apply_canonical(&mut self.producer, &change)?;

        self.normalizer = NewlineNormalizer::default();
        self.compiler.reset();
        self.work = work;
        Ok(EngineOutput::one(change))
    }

    pub fn lifecycle(&self) -> DocumentLifecycle {
        self.producer
            .document()
            .map_or(DocumentLifecycle::Open, |document| document.lifecycle())
    }

    pub fn coordinate(&self) -> Option<&Coordinate> {
        self.producer
            .document()
            .map(mdstream_protocol::Document::coordinate)
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        self.producer.document().map(|document| document.snapshot())
    }

    pub fn metrics(&self) -> EngineMetrics {
        let compiler = self.compiler.metrics();
        let normalized_input_debt_bytes = self.normalizer.pending_bytes();
        let source_cursor = self
            .producer
            .document()
            .map_or(0, |document| document.coordinate().source_cursor.get());
        let frontier_bytes = u64::try_from(compiler.frontier_bytes)
            .expect("compiler frontier lengths fit the protocol cursor domain");
        let reducer = self.producer.metrics();
        let storage = EngineStorageMetrics::measure(
            self.producer.document(),
            compiler.frontier_bytes,
            normalized_input_debt_bytes,
            reducer,
        );
        EngineMetrics {
            retained_input_bytes: compiler
                .frontier_bytes
                .saturating_add(normalized_input_debt_bytes),
            retained_source_base: source_cursor.saturating_sub(frontier_bytes),
            compiler,
            reducer,
            storage,
            work: self.work,
        }
    }

    fn append_transition(&mut self, chunk: &str) -> Result<EngineOutput, EngineError> {
        if self.lifecycle() == DocumentLifecycle::Finalized {
            return Err(EngineError::Finished);
        }

        let (normalizer, suffix) = self.normalizer.append(chunk);
        self.preflight_source(&normalizer, &suffix)?;
        if suffix.is_empty() {
            self.normalizer = normalizer;
            return Ok(EngineOutput::default());
        }

        self.apply_compiler_transition(normalizer, suffix, false)
    }

    fn finish_transition(&mut self) -> Result<EngineOutput, EngineError> {
        if self.lifecycle() == DocumentLifecycle::Finalized {
            return Ok(EngineOutput::default());
        }

        let (normalizer, suffix) = self.normalizer.finish();
        self.preflight_source(&normalizer, &suffix)?;
        self.apply_compiler_transition(normalizer, suffix, true)
    }

    fn preflight_source(
        &self,
        normalizer: &NewlineNormalizer,
        suffix: &str,
    ) -> Result<(), EngineError> {
        let retained_source_bytes = self
            .producer
            .document()
            .map_or(0, |document| document.source().len());
        let source_bytes = retained_source_bytes
            .checked_add(suffix.len())
            .and_then(|bytes| bytes.checked_add(normalizer.pending_bytes()))
            .ok_or(EngineError::CursorOverflow)?;
        if source_bytes > self.limits.max_source_bytes {
            return Err(EngineError::Protocol(ProtocolError::SourceTooLarge {
                limit: self.limits.max_source_bytes,
                actual: source_bytes,
            }));
        }
        Ok(())
    }

    fn apply_compiler_transition(
        &mut self,
        normalizer: NewlineNormalizer,
        suffix: String,
        finishing: bool,
    ) -> Result<EngineOutput, EngineError> {
        let epoch = self
            .producer
            .document()
            .map_or(self.initial_epoch, |document| document.coordinate().epoch);
        let staging_frontier_bytes = self
            .compiler
            .metrics()
            .frontier_bytes
            .checked_add(suffix.len())
            .ok_or(EngineError::MetricsOverflow("staging frontier bytes"))?;
        EngineWorkMetrics::check_transaction_lower_bound(
            suffix.len(),
            staging_frontier_bytes,
            self.engine_limits,
        )?;
        let transition =
            self.compiler
                .stage(self.producer.document(), epoch, &suffix, finishing)?;
        debug_assert_eq!(transition.staging_frontier_bytes(), staging_frontier_bytes);
        let frontier_bytes = transition.staging_frontier_bytes();
        let (operations, payload_cost, commit) = transition.into_parts();
        let change = append_change(&self.producer, self.initial_epoch, suffix, operations)?;
        let work = self
            .work
            .stage(&change, payload_cost, frontier_bytes, self.engine_limits)?;
        apply_canonical(&mut self.producer, &change)?;
        self.compiler.commit(commit);
        self.normalizer = normalizer;
        self.work = work;
        Ok(EngineOutput::one(change))
    }
}

impl Default for StreamEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_canonical(
    reducer: &mut Reducer,
    change: &mdstream_protocol::ChangeSet,
) -> Result<(), EngineError> {
    let outcome = reducer
        .apply_producer_ref(change)
        .map_err(classify_producer_error)?;
    if !matches!(
        outcome,
        ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
    ) {
        return Err(EngineError::InternalInvariant(
            ProtocolError::InvalidChange(format!("producer reducer returned {outcome:?}")),
        ));
    }
    Ok(())
}

fn classify_producer_error(error: ProtocolError) -> EngineError {
    match error {
        error @ (ProtocolError::CursorOverflow
        | ProtocolError::MetadataOverflow
        | ProtocolError::SourceTooLarge { .. }
        | ProtocolError::TooManyNodes { .. }
        | ProtocolError::TooManyOperations { .. }
        | ProtocolError::ValueTooLarge { .. }) => EngineError::Protocol(error),
        error => EngineError::InternalInvariant(error),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn rejected_compiler_transition_does_not_commit_state() {
        let limits = ProtocolLimits {
            max_operations: 0,
            ..ProtocolLimits::default()
        };
        let mut engine = StreamEngine::with_limits(limits);
        let before = engine.metrics().compiler;

        assert!(matches!(
            engine.append("hello"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "change.operations",
                limit: 0,
                ..
            }))
        ));
        assert_eq!(engine.metrics().compiler, before);
        assert!(engine.snapshot().is_none());
        assert!(engine.coordinate().is_none());
    }

    #[test]
    fn rejected_operation_batch_preserves_an_existing_document_and_allows_retry() {
        let limits = ProtocolLimits {
            max_operations: 10,
            ..ProtocolLimits::default()
        };
        let mut engine = StreamEngine::with_limits(limits);
        engine.append("seed\n\n").unwrap();
        let before = engine.snapshot().unwrap();
        let before_metrics = engine.metrics();
        let mut oversized = String::new();
        for index in 0..20 {
            write!(oversized, "paragraph {index}\n\n").unwrap();
        }

        assert!(matches!(
            engine.append(&oversized),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "change.operations",
                limit: 10,
                actual: 11,
            }))
        ));
        assert_eq!(engine.snapshot().unwrap(), before);
        assert_eq!(engine.metrics(), before_metrics);

        engine
            .append("tail")
            .expect("a smaller retry after rejection must succeed");
        assert!(engine.snapshot().unwrap().source().ends_with("tail"));
    }

    #[test]
    fn source_limit_is_rejected_before_compiler_work() {
        let limits = ProtocolLimits {
            max_source_bytes: 4,
            ..ProtocolLimits::default()
        };
        let mut engine = StreamEngine::with_limits(limits);
        let before = engine.metrics().compiler;

        assert!(matches!(
            engine.append("hello"),
            Err(EngineError::Protocol(ProtocolError::SourceTooLarge {
                limit: 4,
                actual: 5,
            }))
        ));
        assert_eq!(engine.metrics().compiler, before);
        assert!(engine.snapshot().is_none());

        engine
            .append("okay")
            .expect("a retry within the limit works");
        assert_eq!(engine.coordinate().unwrap().source_cursor.get(), 4);
    }

    #[test]
    fn ordinary_markdown_depth_is_rejected_before_identity_materialization() {
        let limits = ProtocolLimits {
            max_tree_depth: 2,
            ..ProtocolLimits::default()
        };
        let mut engine = StreamEngine::with_limits(limits);

        assert!(matches!(
            engine.append("> > > deep"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "tree.depth",
                limit: 2,
                actual: 3,
            }))
        ));
        assert!(engine.snapshot().is_none());
    }

    #[test]
    fn draft_node_budget_rejects_at_the_first_unadmitted_node() {
        let exact_limits = ProtocolLimits {
            max_nodes: 2,
            ..ProtocolLimits::default()
        };
        let mut exact = StreamEngine::with_limits(exact_limits);
        exact
            .append("hello")
            .expect("a paragraph and its text leaf fit exactly");

        let exceeded_limits = ProtocolLimits {
            max_nodes: 1,
            ..ProtocolLimits::default()
        };
        let mut exceeded = StreamEngine::with_limits(exceeded_limits);
        assert!(matches!(
            exceeded.append("hello"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "nodes",
                limit: 1,
                actual: 2,
            }))
        ));
        assert!(exceeded.snapshot().is_none());
    }

    #[test]
    fn draft_budget_includes_stable_nodes_but_excludes_the_replaced_frontier() {
        let limits = ProtocolLimits {
            max_nodes: 3,
            ..ProtocolLimits::default()
        };
        let mut engine = StreamEngine::with_limits(limits);
        engine.append("one\n\n").unwrap();
        let before = engine.snapshot().unwrap();
        let before_metrics = engine.metrics();

        assert!(matches!(
            engine.append("two"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "nodes",
                limit: 3,
                actual: 4,
            }))
        ));
        assert_eq!(engine.snapshot().unwrap(), before);
        assert_eq!(engine.metrics(), before_metrics);
        engine
            .append("---\n")
            .expect("one stable node must still fit after the rejected append");

        let mut frontier = StreamEngine::with_limits(ProtocolLimits {
            max_nodes: 2,
            ..ProtocolLimits::default()
        });
        frontier.append("hel").unwrap();
        frontier
            .append("lo\n\n")
            .expect("recompiling a frontier must not count its old projection twice");
    }

    #[test]
    fn draft_budget_preflights_document_wide_roots_resources_and_metadata() {
        let mut roots = StreamEngine::with_limits(ProtocolLimits {
            max_children_per_list: 1,
            ..ProtocolLimits::default()
        });
        roots.append("one\n\n").unwrap();
        let roots_before = roots.snapshot().unwrap();
        assert!(matches!(
            roots.append("two"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "roots",
                limit: 1,
                actual: 2,
            }))
        ));
        assert_eq!(roots.snapshot().unwrap(), roots_before);

        let mut resources = StreamEngine::with_limits(ProtocolLimits {
            max_resources: 1,
            ..ProtocolLimits::default()
        });
        resources.append("[a](https://a)\n\n").unwrap();
        let resources_before = resources.snapshot().unwrap();
        assert!(matches!(
            resources.append("[b](https://b)"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "resources",
                limit: 1,
                actual: 2,
            }))
        ));
        assert_eq!(resources.snapshot().unwrap(), resources_before);

        let mut metadata = StreamEngine::with_limits(ProtocolLimits {
            max_document_metadata_bytes: 1,
            ..ProtocolLimits::default()
        });
        metadata.append("&amp;\n\n").unwrap();
        let metadata_before = metadata.snapshot().unwrap();
        assert!(matches!(
            metadata.append("&amp;"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "document.metadata",
                limit: 1,
                actual: 2,
            }))
        ));
        assert_eq!(metadata.snapshot().unwrap(), metadata_before);
    }

    #[test]
    fn document_wide_draft_budget_excludes_the_replaced_frontier() {
        let mut roots = StreamEngine::with_limits(ProtocolLimits {
            max_children_per_list: 1,
            ..ProtocolLimits::default()
        });
        roots.append("hel").unwrap();
        roots
            .append("lo\n\n")
            .expect("recompiling one frontier root must not count it twice");

        let mut resources = StreamEngine::with_limits(ProtocolLimits {
            max_resources: 1,
            ..ProtocolLimits::default()
        });
        resources.append("[a](https://a)").unwrap();
        resources
            .append(" tail\n\n")
            .expect("recompiling one frontier resource must not count it twice");
    }

    #[test]
    fn repeated_reference_uses_share_one_canonical_resource_budget() {
        let source = "[a][shared] [b][shared]\n\n[shared]: https://example.test\n";
        let mut engine = StreamEngine::with_limits(ProtocolLimits {
            max_resources: 1,
            ..ProtocolLimits::default()
        });

        engine
            .append(source)
            .expect("one canonical reference target must consume one resource slot");
        let document = engine.producer.document().unwrap();
        assert_eq!(document.resources().len(), 1);

        let exact_metadata_bytes = "shared".len() * 2 + "https://example.test".len();
        let mut metadata = StreamEngine::with_limits(ProtocolLimits {
            max_document_metadata_bytes: exact_metadata_bytes,
            ..ProtocolLimits::default()
        });
        metadata
            .append(source)
            .expect("duplicate uses must not duplicate canonical resource metadata");
    }

    #[test]
    fn reference_resources_use_the_first_definition_across_custom_regions() {
        let source = concat!(
            "[a][shared]\n\n[shared]: /one\n\n",
            "<thinking>\nbody\n</thinking>\n\n",
            "[b][shared]\n\n[shared]: /two\n",
        );
        let mut engine = StreamEngine::builder()
            .custom_block(CustomBlockSpec::try_new("x", "thinking").unwrap())
            .build()
            .unwrap();

        engine.append(source).unwrap();
        let document = engine.producer.document().unwrap();
        let resources = document.resources().collect::<Vec<_>>();
        assert_eq!(resources.len(), 1);
        assert!(matches!(
            &resources[0].content,
            mdstream_protocol::SemanticResourceKind::Link { destination, .. }
                if destination == "/one"
        ));
        let targets = document
            .nodes()
            .filter_map(|node| match &node.content {
                mdstream_protocol::ContentKind::Link {
                    target: Some(target),
                    ..
                } => Some(target.id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(targets, vec![resources[0].id; 2]);
    }

    #[test]
    fn synthetic_nodes_use_the_same_draft_admission_budget() {
        let mut tight_list = StreamEngine::with_limits(ProtocolLimits {
            max_nodes: 3,
            ..ProtocolLimits::default()
        });
        assert!(matches!(
            tight_list.append("- item"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "nodes",
                limit: 3,
                actual: 4,
            }))
        ));

        let table = "| a |\n| --- |\n| b |\n";
        for (limit, actual) in [(4, 5), (8, 9)] {
            let mut engine = StreamEngine::with_limits(ProtocolLimits {
                max_nodes: limit,
                ..ProtocolLimits::default()
            });
            assert!(matches!(
                engine.append(table),
                Err(EngineError::Compiler(CompilerError::LimitExceeded {
                    field: "nodes",
                    limit: rejected_limit,
                    actual: rejected_actual,
                })) if rejected_limit == limit && rejected_actual == actual
            ));
        }

        let mut structural = StreamEngine::with_limits(ProtocolLimits {
            max_document_structural_items: 9,
            ..ProtocolLimits::default()
        });
        assert!(matches!(
            structural.append(table),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "document.structural_items",
                limit: 9,
                actual: 10,
            }))
        ));
    }

    #[test]
    fn custom_nodes_use_the_same_draft_admission_budget() {
        let custom = || CustomBlockSpec::try_new("x", "thinking").unwrap();
        let mut opaque = StreamEngine::builder()
            .protocol_limits(ProtocolLimits {
                max_nodes: 0,
                ..ProtocolLimits::default()
            })
            .custom_block(custom())
            .build()
            .unwrap();
        assert!(matches!(
            opaque.append("<thinking>\nbody\n</thinking>"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "nodes",
                limit: 0,
                actual: 1,
            }))
        ));

        let mut nonopaque = StreamEngine::builder()
            .protocol_limits(ProtocolLimits {
                max_nodes: 2,
                ..ProtocolLimits::default()
            })
            .custom_block(custom().opaque(false))
            .build()
            .unwrap();
        assert!(matches!(
            nonopaque.append("<thinking>\nbody\n</thinking>"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "nodes",
                limit: 2,
                actual: 3,
            }))
        ));
    }

    #[test]
    fn compiler_preflights_aggregate_change_payload_budgets() {
        let mut structural = StreamEngine::with_limits(ProtocolLimits {
            max_change_structural_items: 1,
            ..ProtocolLimits::default()
        });
        assert!(matches!(
            structural.append("hello"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "change.structural_items",
                limit: 1,
                actual: 2,
            }))
        ));
        assert!(structural.snapshot().is_none());

        let custom = || CustomBlockSpec::try_new("x", "thinking").unwrap();
        let mut metadata = StreamEngine::builder()
            .protocol_limits(ProtocolLimits {
                max_change_metadata_bytes: 10,
                ..ProtocolLimits::default()
            })
            .custom_block(custom())
            .build()
            .unwrap();
        assert!(matches!(
            metadata.append("<thinking a=1>\nbody\n</thinking>"),
            Err(EngineError::Compiler(CompilerError::LimitExceeded {
                field: "change.metadata",
                limit: 10,
                actual: 11,
            }))
        ));
        assert!(metadata.snapshot().is_none());

        StreamEngine::builder()
            .protocol_limits(ProtocolLimits {
                max_change_metadata_bytes: 11,
                ..ProtocolLimits::default()
            })
            .custom_block(custom())
            .build()
            .unwrap()
            .append("<thinking a=1>\nbody\n</thinking>")
            .expect("the exact aggregate metadata budget must be accepted");
    }

    #[test]
    fn compiler_preserves_machine_classifiable_markdown_diagnostics() {
        let error = CompilerError::from(MarkdownDiagnostic::InvalidRange { start: 7, end: 3 });

        assert!(matches!(
            error,
            CompilerError::Markdown(MarkdownDiagnostic::InvalidRange { start: 7, end: 3 })
        ));
    }
}
