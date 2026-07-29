use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::PathBuf,
};

use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::{
    ChunkSchedule, ClaimScope, CompatibilityProfile, Dialect, FIXTURE_SCHEMA, Fixture,
    FixtureExpectation, NamedChunkSchedule, NormalizedSnapshot, OracleKind, ProtocolTrace,
    Provenance, RequiredCheckpoint, TraceInputEvent, assert_fixture_protocol, load_fixture,
    replay_protocol_trace,
};
use mdstream_processors::{
    ArtifactHost, CitationProcessor, CompletionOutcome, ConfigurationVersion, ContentProcessor,
    ProcessingPolicy, ProcessorArtifact, ProcessorCapabilities, ProcessorDescriptor,
    ProcessorFailure, ProcessorFailureCode, ProcessorLimits, run_catching,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeSet, ContentKind, DocumentLifecycle, NodeId, NodeStability, NodeVersion,
    Reducer, ReducerStatus,
};
use serde_json::Value;

const UPDATE_FIXTURE_ENV: &str = "MDSTREAM_UPDATE_ADOPTION_FIXTURE";

struct GoldenScenario {
    source: String,
    schedules: Vec<NamedChunkSchedule>,
    checkpoint_compatible: BTreeSet<String>,
    checkpoints: Vec<ScenarioCheckpoint>,
    final_observations: BTreeSet<String>,
    recovery_trace: String,
    recovery_actions: Vec<Value>,
    expected_node_kinds: BTreeSet<String>,
    expected_resource_kinds: BTreeSet<String>,
    expected_code_languages: BTreeSet<String>,
}

struct ScenarioCheckpoint {
    id: String,
    boundary_invariant: bool,
    source_cursor: usize,
    observations: BTreeSet<String>,
}

struct EchoProcessor {
    descriptor: ProcessorDescriptor,
}

impl EchoProcessor {
    fn new() -> Self {
        Self {
            descriptor: ProcessorDescriptor::new(
                "adoption.echo",
                "v1",
                ProcessorCapabilities::stable_only(),
            )
            .unwrap(),
        }
    }
}

impl ContentProcessor for EchoProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        &self.descriptor
    }

    fn process(
        &self,
        request: &mdstream_processors::ProcessorRequest,
    ) -> Result<ProcessorArtifact, ProcessorFailure> {
        ProcessorArtifact::text(
            "mdstream.adoption.echo/1",
            "text/plain",
            request.input().body(),
        )
        .map_err(|error| ProcessorFailure::new(ProcessorFailureCode::Processor, error.to_string()))
    }
}

#[test]
fn native_headless_adoption_matches_the_checked_in_golden() {
    let scenario = load_scenario();
    let expected = build_fixture(&scenario);
    let path = fixture_path();
    if env::var_os(UPDATE_FIXTURE_ENV).is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = serde_json::to_vec_pretty(&expected).unwrap();
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();
    }

    let fixture = load_fixture(&path).unwrap();
    assert_eq!(
        fixture, expected,
        "run with {UPDATE_FIXTURE_ENV}=1 to regenerate"
    );
    assert_fixture_protocol(&fixture).unwrap();
    exercise_recovery(&fixture, &scenario);
    exercise_native_state_and_processors(&fixture, &scenario);
}

fn build_fixture(scenario: &GoldenScenario) -> Fixture {
    let schedules = scenario.schedules.clone();
    let traces = schedules
        .iter()
        .map(|named| capture_trace(&named.id, &named.schedule, &scenario.source))
        .collect::<Vec<_>>();
    let baseline_report = replay_protocol_trace(&traces[0]).unwrap();
    assert_final_contract(&baseline_report.final_snapshot, scenario);
    let normalized_snapshot = baseline_report.normalized_final_snapshot();
    assert!(traces.iter().all(|trace| {
        replay_protocol_trace(trace)
            .unwrap()
            .normalized_final_snapshot()
            == normalized_snapshot
    }));
    let required_checkpoints = build_required_checkpoints(scenario, &traces);

    let fixture = Fixture {
        schema: FIXTURE_SCHEMA.to_string(),
        id: "adoption.headless-rich-content".to_string(),
        description:
            "Golden AI Stream adoption across whole, stage-aligned, and adversarial chunks."
                .to_string(),
        source: scenario.source.clone(),
        dialect: Dialect {
            id: "mdstream.canonical/0.4".to_string(),
            extensions: vec!["mdstream.citation/1".to_string()],
        },
        profile: CompatibilityProfile {
            id: "mdstream.adoption/0.4".to_string(),
            claim_scope: vec![ClaimScope::ProtocolReplay],
            pipeline: vec![
                "StreamEngine".to_string(),
                "Reducer".to_string(),
                "ArtifactHost".to_string(),
            ],
        },
        provenance: Provenance::Synthetic {
            generator: "mdstream adoption fixture generator".to_string(),
            oracle_kind: OracleKind::CanonicalProtocol,
        },
        options: BTreeMap::new(),
        schedules,
        traces,
        expected: FixtureExpectation {
            normalized_snapshot: Some(normalized_snapshot),
            ..FixtureExpectation::default()
        },
        required_checkpoints,
    };
    fixture.validate().unwrap();
    fixture
}

fn capture_trace(id: &str, schedule: &ChunkSchedule, source: &str) -> ProtocolTrace {
    let mut engine = StreamEngine::new();
    let mut changes = Vec::new();
    let mut input_events = Vec::new();
    for chunk in schedule.slices(source).unwrap() {
        extend_changes(&mut changes, engine.append(chunk).unwrap());
        input_events.push(TraceInputEvent::Append {
            chunk: chunk.to_string(),
            change_end: changes.len(),
        });
    }
    extend_changes(&mut changes, engine.finish().unwrap());
    input_events.push(TraceInputEvent::Finish {
        change_end: changes.len(),
    });
    ProtocolTrace {
        id: id.to_string(),
        schedule: id.to_string(),
        setup_changes: 0,
        input_events,
        changes,
    }
}

fn extend_changes(changes: &mut Vec<ChangeSet>, output: EngineOutput) {
    changes.extend(output.into_changes());
}

fn load_scenario() -> GoldenScenario {
    let value: Value = serde_json::from_slice(&fs::read(scenario_path()).unwrap()).unwrap();
    assert_eq!(value["schema"], "mdstream.example-scenario/1");
    let mainline = &value["episodes"]["mainline"];
    let actions = mainline["actions"].as_array().unwrap();
    let mut source = String::new();
    let mut stage_boundaries = Vec::new();
    let mut checkpoints = Vec::new();
    let mut final_observations = BTreeSet::new();
    for action in actions {
        match action["kind"].as_str().unwrap() {
            "append" => {
                source.push_str(action["chunk"].as_str().unwrap());
                stage_boundaries.push(source.len());
            }
            "checkpoint" => checkpoints.push(ScenarioCheckpoint {
                id: action["id"].as_str().unwrap().to_string(),
                boundary_invariant: action["scope"] == "boundary_invariant",
                source_cursor: action["source_cursor"].as_u64().unwrap() as usize,
                observations: string_set(&action["observations"]),
            }),
            "finish" => final_observations = string_set(&action["observations"]),
            kind => panic!("unsupported Golden AI Stream action `{kind}`"),
        }
    }
    assert_eq!(value["expected"]["final_source"], source);

    let mut checkpoint_compatible = BTreeSet::new();
    let schedules = mainline["schedules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|schedule| {
            let id = schedule["id"].as_str().unwrap().to_string();
            if schedule["checkpoint_compatible"].as_bool().unwrap() {
                checkpoint_compatible.insert(id.clone());
            }
            let schedule = match schedule["kind"].as_str().unwrap() {
                "whole" => ChunkSchedule::Whole,
                "stage_aligned" => ChunkSchedule::ByteCuts {
                    cuts: stage_boundaries[..stage_boundaries.len() - 1].to_vec(),
                },
                "byte_cuts" => ChunkSchedule::ByteCuts {
                    cuts: schedule["cuts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|cut| cut.as_u64().unwrap() as usize)
                        .collect(),
                },
                kind => panic!("unsupported Golden AI Stream schedule `{kind}`"),
            };
            NamedChunkSchedule { id, schedule }
        })
        .collect();

    GoldenScenario {
        source,
        schedules,
        checkpoint_compatible,
        checkpoints,
        final_observations,
        recovery_trace: value["episodes"]["recovery"]["trace"]
            .as_str()
            .unwrap()
            .to_string(),
        recovery_actions: value["episodes"]["recovery"]["actions"]
            .as_array()
            .unwrap()
            .clone(),
        expected_node_kinds: string_set(&value["expected"]["node_kinds"]),
        expected_resource_kinds: string_set(&value["expected"]["resource_kinds"]),
        expected_code_languages: string_set(&value["expected"]["code_languages"]),
    }
}

fn build_required_checkpoints(
    scenario: &GoldenScenario,
    traces: &[ProtocolTrace],
) -> Vec<RequiredCheckpoint> {
    let mut required = Vec::new();
    for checkpoint in &scenario.checkpoints {
        let mut normalized_baseline = None;
        for trace in traces
            .iter()
            .filter(|trace| scenario.checkpoint_compatible.contains(&trace.id))
        {
            let after_change = change_after_source_cursor(trace, checkpoint.source_cursor);
            let report = replay_protocol_trace(trace).unwrap();
            let snapshot = report.snapshot_after(after_change).unwrap();
            let normalized = NormalizedSnapshot::from(snapshot);
            if checkpoint.boundary_invariant {
                if let Some(baseline) = &normalized_baseline {
                    assert_eq!(
                        &normalized, baseline,
                        "checkpoint `{}` diverged for schedule `{}`",
                        checkpoint.id, trace.id
                    );
                } else {
                    normalized_baseline = Some(normalized);
                }
            }
            required.push(RequiredCheckpoint {
                id: format!("{}.{}", trace.id, checkpoint.id),
                trace: trace.id.clone(),
                after_change,
                coordinate: None,
                lifecycle: Some(DocumentLifecycle::Open),
                source: Some(scenario.source[..checkpoint.source_cursor].to_string()),
                normalized_snapshot: None,
            });
        }
    }
    for trace in traces {
        required.push(RequiredCheckpoint {
            id: format!("{}.finalized", trace.id),
            trace: trace.id.clone(),
            after_change: trace.changes.len() - 1,
            coordinate: None,
            lifecycle: Some(DocumentLifecycle::Finalized),
            source: Some(scenario.source.clone()),
            normalized_snapshot: None,
        });
    }
    assert_scenario_observations(scenario, traces);
    required
}

fn change_after_source_cursor(trace: &ProtocolTrace, expected_cursor: usize) -> usize {
    let mut source_cursor = 0;
    for event in &trace.input_events {
        if let TraceInputEvent::Append { chunk, change_end } = event {
            source_cursor += chunk.len();
            if source_cursor == expected_cursor {
                return change_end.checked_sub(1).unwrap();
            }
            assert!(source_cursor < expected_cursor);
        }
    }
    panic!(
        "trace `{}` has no append boundary at source cursor {expected_cursor}",
        trace.id
    );
}

fn string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_string())
        .collect()
}

fn assert_scenario_observations(scenario: &GoldenScenario, traces: &[ProtocolTrace]) {
    let trace = traces
        .iter()
        .find(|trace| trace.id == "stage-aligned")
        .unwrap();
    let report = replay_protocol_trace(trace).unwrap();
    let mut snapshots = BTreeMap::new();
    for checkpoint in &scenario.checkpoints {
        let after_change = change_after_source_cursor(trace, checkpoint.source_cursor);
        let snapshot = report.snapshot_after(after_change).unwrap();
        snapshots.insert(checkpoint.id.as_str(), snapshot);
        for observation in &checkpoint.observations {
            match observation.as_str() {
                "pending_source" => assert!(
                    !snapshot.pending_source().unwrap().is_empty(),
                    "checkpoint `{}` promised pending source",
                    checkpoint.id
                ),
                observation if observation.starts_with("provisional_") => assert!(
                    snapshot
                        .nodes()
                        .iter()
                        .any(|node| node.stability == NodeStability::Provisional),
                    "checkpoint `{}` promised `{observation}`",
                    checkpoint.id
                ),
                "stable_code_block" => {
                    assert!(
                        has_stable_code_block(snapshot, "rust"),
                        "checkpoint `{}` promised a stable Rust block",
                        checkpoint.id
                    );
                }
                "stable_mermaid_block" => {
                    assert!(
                        has_stable_code_block(snapshot, "mermaid"),
                        "checkpoint `{}` promised a stable Mermaid block",
                        checkpoint.id
                    );
                }
                "unresolved_citation" => assert!(snapshot.nodes().iter().any(|node| matches!(
                    &node.content,
                    ContentKind::CitationReference {
                        key,
                        target: None,
                    } if key == "engine"
                ))),
                "resolved_citation" | "semantic_correction" => {
                    assert!(
                        snapshot.nodes().iter().any(|node| matches!(
                            &node.content,
                            ContentKind::CitationReference {
                                key,
                                target: Some(_),
                            } if key == "engine"
                        )),
                        "checkpoint `{}` promised `{observation}`",
                        checkpoint.id
                    );
                }
                observation => panic!("unsupported scenario observation `{observation}`"),
            }
        }
    }

    let unresolved = snapshots["rust-fence-pending"]
        .nodes()
        .iter()
        .find(|node| matches!(&node.content, ContentKind::CitationReference { key, .. } if key == "engine"))
        .unwrap();
    assert_eq!(
        scenario.final_observations,
        BTreeSet::from([
            "finalized".to_string(),
            "resolved_citation".to_string(),
            "semantic_correction".to_string(),
            "stable_mermaid_block".to_string(),
        ])
    );
    assert_eq!(
        report.final_snapshot.lifecycle(),
        DocumentLifecycle::Finalized
    );
    assert!(has_stable_code_block(&report.final_snapshot, "mermaid"));
    let corrected = report
        .final_snapshot
        .nodes()
        .iter()
        .find(|node| matches!(&node.content, ContentKind::CitationReference { key, .. } if key == "engine"))
        .unwrap();
    assert_eq!(unresolved.id, corrected.id);
    assert_ne!(unresolved.version, corrected.version);
}

fn has_stable_code_block(snapshot: &mdstream_protocol::Snapshot, language: &str) -> bool {
    snapshot.nodes().iter().any(|node| {
        node.stability == NodeStability::Stable
            && node
                .content
                .code_language()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(language))
    })
}

fn assert_final_contract(snapshot: &mdstream_protocol::Snapshot, scenario: &GoldenScenario) {
    assert_eq!(snapshot.source(), scenario.source);
    assert_eq!(snapshot.lifecycle(), DocumentLifecycle::Finalized);
    assert!(
        snapshot
            .nodes()
            .iter()
            .all(|node| node.stability == NodeStability::Stable)
    );
    let node_kinds = snapshot
        .nodes()
        .iter()
        .map(|node| {
            serde_json::to_value(&node.content).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    let resource_kinds = snapshot
        .resources()
        .iter()
        .map(|resource| {
            serde_json::to_value(&resource.content).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    let code_languages = snapshot
        .nodes()
        .iter()
        .filter_map(|node| node.content.code_language().map(str::to_string))
        .collect::<BTreeSet<_>>();
    assert_eq!(node_kinds, scenario.expected_node_kinds);
    assert_eq!(resource_kinds, scenario.expected_resource_kinds);
    assert_eq!(code_languages, scenario.expected_code_languages);
}

fn exercise_recovery(fixture: &Fixture, scenario: &GoldenScenario) {
    let trace = fixture
        .traces
        .iter()
        .find(|trace| trace.id == scenario.recovery_trace)
        .unwrap();
    assert!(trace.changes.len() > 4);
    let actions = &scenario.recovery_actions;

    assert_recovery_action(
        &actions[0],
        "apply_change",
        "same-floor-replica",
        "retained",
    );
    assert_recovery_action(
        &actions[1],
        "apply_change",
        "same-floor-replica",
        "awaiting_snapshot",
    );
    assert_recovery_action(
        &actions[2],
        "recover_snapshot",
        "same-floor-replica",
        "retained_same_floor",
    );
    let first = actions[0]["change_ordinal"].as_u64().unwrap() as usize;
    let skipped = actions[1]["change_ordinal"].as_u64().unwrap() as usize;
    let same_floor = named_snapshot(trace, scenario, actions[2]["snapshot"].as_str().unwrap());
    let mut replica = Reducer::new();
    assert!(matches!(
        replica.apply(trace.changes[first].clone()).unwrap(),
        ApplyOutcome::Applied { .. }
    ));
    assert!(matches!(
        replica.apply(trace.changes[skipped].clone()).unwrap(),
        ApplyOutcome::RecoveryRequired { .. }
    ));
    assert!(matches!(
        replica.status(),
        ReducerStatus::NeedsSnapshot { .. }
    ));
    match replica.recover_snapshot(same_floor).unwrap() {
        ApplyOutcome::Recovered { impact, .. } => assert!(impact.is_empty()),
        other => panic!("same-floor snapshot was not recovered: {other:?}"),
    }
    for change in trace.changes.iter().skip(first + 1) {
        assert!(matches!(
            replica.apply(change.clone()).unwrap(),
            ApplyOutcome::Applied { .. }
        ));
    }
    assert_eq!(
        NormalizedSnapshot::from(replica.document().unwrap().snapshot()),
        fixture.expected.normalized_snapshot.clone().unwrap()
    );

    assert_recovery_action(&actions[3], "apply_change", "advanced-replica", "retained");
    assert_recovery_action(
        &actions[4],
        "apply_change",
        "advanced-replica",
        "awaiting_snapshot",
    );
    assert_recovery_action(
        &actions[5],
        "recover_snapshot",
        "advanced-replica",
        "replaced_advanced",
    );
    let mut advanced = Reducer::new();
    advanced
        .apply(trace.changes[actions[3]["change_ordinal"].as_u64().unwrap() as usize].clone())
        .unwrap();
    assert!(matches!(
        advanced
            .apply(trace.changes[actions[4]["change_ordinal"].as_u64().unwrap() as usize].clone())
            .unwrap(),
        ApplyOutcome::RecoveryRequired { .. }
    ));
    let replacement = named_snapshot(trace, scenario, actions[5]["snapshot"].as_str().unwrap());
    let replacement_sequence = replacement.coordinate().sequence;
    match advanced.recover_snapshot(replacement).unwrap() {
        ApplyOutcome::Recovered { impact, .. } => assert!(impact.full_replace),
        other => panic!("advanced snapshot was not recovered: {other:?}"),
    }
    for change in trace
        .changes
        .iter()
        .filter(|change| change.sequence() > replacement_sequence)
    {
        assert!(matches!(
            advanced.apply(change.clone()).unwrap(),
            ApplyOutcome::Applied { .. }
        ));
    }
    assert_eq!(
        NormalizedSnapshot::from(advanced.document().unwrap().snapshot()),
        fixture.expected.normalized_snapshot.clone().unwrap()
    );

    assert_recovery_action(&actions[6], "reset", "producer", "new_epoch");
    let mut engine = StreamEngine::new();
    let mut producer = Reducer::new();
    for chunk in fixture
        .schedule("stage-aligned")
        .unwrap()
        .slices(&scenario.source)
        .unwrap()
    {
        apply_direct(&mut producer, engine.append(chunk).unwrap());
    }
    apply_direct(&mut producer, engine.finish().unwrap());
    for change in engine.reset().unwrap().into_changes() {
        match producer.apply(change).unwrap() {
            ApplyOutcome::Recovered { coordinate, impact } => {
                assert_eq!(
                    coordinate.epoch.get(),
                    actions[6]["expect_epoch"].as_u64().unwrap()
                );
                assert!(impact.full_replace);
            }
            other => panic!("reset did not start a new epoch: {other:?}"),
        }
    }
}

fn assert_recovery_action(action: &Value, kind: &str, target: &str, continuity: &str) {
    assert_eq!(action["kind"], kind);
    assert_eq!(action["target"], target);
    assert_eq!(action["continuity"], continuity);
}

fn named_snapshot(
    trace: &ProtocolTrace,
    scenario: &GoldenScenario,
    checkpoint_id: &str,
) -> mdstream_protocol::Snapshot {
    let checkpoint = scenario
        .checkpoints
        .iter()
        .find(|checkpoint| checkpoint.id == checkpoint_id)
        .unwrap();
    replay_protocol_trace(trace)
        .unwrap()
        .snapshot_after(change_after_source_cursor(trace, checkpoint.source_cursor))
        .unwrap()
        .clone()
}

fn apply_direct(reducer: &mut Reducer, output: EngineOutput) {
    for change in output.into_changes() {
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
    }
}

fn exercise_native_state_and_processors(fixture: &Fixture, scenario: &GoldenScenario) {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    let mut rendered = BTreeMap::<NodeId, NodeVersion>::new();
    for chunk in fixture
        .schedule("adversarial")
        .unwrap()
        .slices(&scenario.source)
        .unwrap()
    {
        apply_output(
            &mut reducer,
            &mut host,
            &mut rendered,
            engine.append(chunk).unwrap(),
        );
    }
    apply_output(
        &mut reducer,
        &mut host,
        &mut rendered,
        engine.finish().unwrap(),
    );

    let document = reducer.document().unwrap();
    assert_eq!(
        NormalizedSnapshot::from(document.snapshot()),
        fixture.expected.normalized_snapshot.clone().unwrap()
    );
    assert_eq!(
        rendered,
        document
            .nodes()
            .map(|node| (node.id, node.version.clone()))
            .collect::<BTreeMap<_, _>>()
    );
    let citation_id = document
        .nodes()
        .find(|node| {
            matches!(
                node.content,
                ContentKind::CitationReference {
                    target: Some(_),
                    ..
                }
            )
        })
        .unwrap()
        .id;
    let mermaid_id = document
        .nodes()
        .find(|node| {
            matches!(
                &node.content,
                ContentKind::CodeBlock { info: Some(info), .. }
                    if info.eq_ignore_ascii_case("mermaid")
            )
        })
        .unwrap()
        .id;
    let canonical_before = document.snapshot();

    let citation = CitationProcessor::new();
    let citation_request = host
        .begin(
            document,
            citation.descriptor().clone(),
            citation_id,
            ConfigurationVersion::new("adoption.citation.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(
        host.complete(document, run_catching(&citation, &citation_request))
            .unwrap(),
        CompletionOutcome::Applied
    );
    assert!(
        host.artifact(citation_request.key().slot())
            .and_then(ProcessorArtifact::as_citation)
            .is_some()
    );

    let echo = EchoProcessor::new();
    let echo_configuration = ConfigurationVersion::new("adoption.echo.v1").unwrap();
    let echo_request = host
        .begin(
            document,
            echo.descriptor().clone(),
            mermaid_id,
            echo_configuration.clone(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_eq!(
        host.complete(document, run_catching(&echo, &echo_request))
            .unwrap(),
        CompletionOutcome::Applied
    );
    assert!(host.artifact(echo_request.key().slot()).is_some());
    assert_eq!(reducer.document().unwrap().snapshot(), canonical_before);

    let late = host
        .begin(
            reducer.document().unwrap(),
            echo.descriptor().clone(),
            mermaid_id,
            echo_configuration,
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let late_result = run_catching(&echo, &late);
    apply_output(
        &mut reducer,
        &mut host,
        &mut rendered,
        engine.reset().unwrap(),
    );
    assert_eq!(
        host.complete(reducer.document().unwrap(), late_result)
            .unwrap(),
        CompletionOutcome::Stale
    );
    assert!(host.artifact(echo_request.key().slot()).is_none());
}

fn apply_output(
    reducer: &mut Reducer,
    host: &mut ArtifactHost,
    rendered: &mut BTreeMap<NodeId, NodeVersion>,
    output: EngineOutput,
) {
    for change in output.into_changes() {
        let outcome = reducer.apply(change).unwrap();
        let impact = match outcome {
            ApplyOutcome::Applied { impact, .. } | ApplyOutcome::Recovered { impact, .. } => impact,
            other => panic!("producer emitted a non-continuous change: {other:?}"),
        };
        let document = reducer.document().unwrap();
        host.reconcile(document, &impact).unwrap();
        for id in &impact.removed_nodes {
            assert!(impact.changed_nodes.contains(id));
            assert!(document.node(*id).is_none());
        }
        for id in impact.changed_nodes {
            match document.node(id) {
                Some(node) => {
                    rendered.insert(id, node.version.clone());
                }
                None => {
                    rendered.remove(&id);
                }
            }
        }
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("conformance/fixtures/adoption/headless-rich-content.json")
}

fn scenario_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples/fixtures/golden-ai-stream.json")
}
