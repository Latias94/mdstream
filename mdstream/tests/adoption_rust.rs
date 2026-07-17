use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::PathBuf,
};

use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::{
    ChunkSchedule, ClaimScope, CompatibilityProfile, Dialect, FIXTURE_SCHEMA, Fixture,
    FixtureExpectation, NamedChunkSchedule, NormalizedSnapshot, OracleKind, ProtocolTrace,
    Provenance, TraceInputEvent, assert_fixture_protocol, load_fixture, replay_protocol_trace,
};
use mdstream_processors::{
    ArtifactHost, CitationProcessor, CompletionOutcome, ConfigurationVersion, ContentProcessor,
    ProcessingPolicy, ProcessorArtifact, ProcessorCapabilities, ProcessorDescriptor,
    ProcessorFailure, ProcessorFailureCode, ProcessorLimits, run_catching,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeSet, ContentKind, NodeId, NodeVersion, Reducer, ReducerStatus,
};

const SOURCE: &str = concat!(
    "# Adoption\n\n",
    "See [@Engine] while this diagram streams.\n\n",
    "```mermaid\n",
    "flowchart LR\n",
    "  Token --> IR\n",
    "```\n\n",
    "[@engine]: https://mdstream.dev/engine \"mdstream\"\n",
);
const UPDATE_FIXTURE_ENV: &str = "MDSTREAM_UPDATE_ADOPTION_FIXTURE";

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
    let expected = build_fixture();
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
    exercise_recovery(&fixture);
    exercise_native_state_and_processors(&fixture);
}

fn build_fixture() -> Fixture {
    let schedules = vec![
        NamedChunkSchedule {
            id: "whole".to_string(),
            schedule: ChunkSchedule::Whole,
        },
        NamedChunkSchedule {
            id: "adversarial".to_string(),
            schedule: adversarial_schedule(),
        },
    ];
    let traces = schedules
        .iter()
        .map(|named| capture_trace(&named.id, &named.schedule))
        .collect::<Vec<_>>();
    let normalized_snapshot = replay_protocol_trace(&traces[0])
        .unwrap()
        .normalized_final_snapshot();
    assert_eq!(
        replay_protocol_trace(&traces[1])
            .unwrap()
            .normalized_final_snapshot(),
        normalized_snapshot
    );

    let fixture = Fixture {
        schema: FIXTURE_SCHEMA.to_string(),
        id: "adoption.headless-rich-content".to_string(),
        description: "Production-shaped headless adoption across adversarial token chunks."
            .to_string(),
        source: SOURCE.to_string(),
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
        required_checkpoints: Vec::new(),
    };
    fixture.validate().unwrap();
    fixture
}

fn capture_trace(id: &str, schedule: &ChunkSchedule) -> ProtocolTrace {
    let mut engine = StreamEngine::new();
    let mut changes = Vec::new();
    let mut input_events = Vec::new();
    for chunk in schedule.slices(SOURCE).unwrap() {
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

fn adversarial_schedule() -> ChunkSchedule {
    let mut cuts = BTreeSet::new();
    for (needle, offsets) in [
        ("[@Engine]", &[1, 3, 7][..]),
        ("```mermaid", &[1, 2, 5, 8][..]),
        ("Token --> IR", &[2, 7, 9][..]),
        ("[@engine]:", &[1, 4, 8][..]),
    ] {
        let start = SOURCE.find(needle).unwrap();
        cuts.extend(offsets.iter().map(|offset| start + offset));
    }
    ChunkSchedule::ByteCuts {
        cuts: cuts.into_iter().collect(),
    }
}

fn exercise_recovery(fixture: &Fixture) {
    let trace = fixture
        .traces
        .iter()
        .find(|trace| trace.id == "adversarial")
        .unwrap();
    assert!(trace.changes.len() > 4);

    let mut primary = Reducer::new();
    for change in trace.changes.iter().take(3) {
        assert!(matches!(
            primary.apply(change.clone()).unwrap(),
            ApplyOutcome::Applied { .. }
        ));
    }
    let recovery = primary.document().unwrap().snapshot();

    let mut replica = Reducer::new();
    replica.apply(trace.changes[0].clone()).unwrap();
    assert!(matches!(
        replica.apply(trace.changes[2].clone()).unwrap(),
        ApplyOutcome::RecoveryRequired { .. }
    ));
    assert!(matches!(
        replica.status(),
        ReducerStatus::NeedsSnapshot { .. }
    ));
    assert!(matches!(
        replica.recover_snapshot(recovery).unwrap(),
        ApplyOutcome::Recovered { .. }
    ));
    for change in trace.changes.iter().skip(3) {
        assert!(matches!(
            replica.apply(change.clone()).unwrap(),
            ApplyOutcome::Applied { .. }
        ));
    }
    assert_eq!(
        NormalizedSnapshot::from(replica.document().unwrap().snapshot()),
        fixture.expected.normalized_snapshot.clone().unwrap()
    );
}

fn exercise_native_state_and_processors(fixture: &Fixture) {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    let mut rendered = BTreeMap::<NodeId, NodeVersion>::new();
    for chunk in fixture
        .schedule("adversarial")
        .unwrap()
        .slices(SOURCE)
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
