use std::{fs, path::PathBuf};

use mdstream_bindings_core::{
    BINDING_OPTIONS_SCHEMA, BINDING_SCHEMA, BindingPayloadKind, ReducerSession, TRANSITION_SCHEMA,
};
use mdstream_protocol::{
    ChangeId, ChangeSet, ChildList, ChildListOwner, CitationProtocol, ContentKind, ContentNode,
    Epoch, NodeId, NodeProjection, NodeStability, ProjectionOp, ProtocolLimits, ResourceId,
    SemanticResource, SemanticResourceKind, SemanticText, Sequence, SourceCursor, SourceDelta,
    SourceRange, TextTransition, TransitionFacts, TransitionReducer, encode_change_json,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const FIXTURE_SCHEMA: &str = "mdstream.transition-golden/1";
const FIXTURE_PATH: &str = "../conformance/goldens/transition-v1.json";
const UPDATE_ENV: &str = "UPDATE_TRANSITION_GOLDEN";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionGoldenFixture {
    schema: String,
    binding_schema: String,
    transition_schema: String,
    description: String,
    cases: Vec<TransitionGoldenCase>,
    invalid_transition_schemas: Vec<InvalidTransitionSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionGoldenCase {
    id: String,
    description: String,
    covers: Vec<String>,
    wire_json: String,
    normalized: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvalidTransitionSchema {
    id: String,
    description: String,
    base_case: String,
    schema: String,
}

struct GoldenBuilder {
    binding: ReducerSession,
    oracle: TransitionReducer,
}

impl GoldenBuilder {
    fn new() -> Self {
        Self {
            binding: ReducerSession::new(&transition_options()).unwrap(),
            oracle: TransitionReducer::new(),
        }
    }

    fn capture(
        &mut self,
        id: &str,
        description: &str,
        covers: &[&str],
        change: ChangeSet,
    ) -> TransitionGoldenCase {
        let expected = self.oracle.apply(change.clone()).unwrap();
        let encoded = encode_change_json(&change, usize::MAX, ProtocolLimits::default()).unwrap();
        let output = self.binding.apply_change(&encoded).unwrap();
        let payload = output
            .payloads()
            .iter()
            .find(|payload| payload.kind() == BindingPayloadKind::ReducerUpdate)
            .expect("every applied change emits one reducer update");
        let wire_json = std::str::from_utf8(payload.bytes()).unwrap().to_owned();
        let wire: Value = serde_json::from_str(&wire_json).unwrap();

        assert_eq!(wire["schema"], BINDING_SCHEMA);
        assert_eq!(wire["transition"]["schema"], TRANSITION_SCHEMA);
        let actual_facts: TransitionFacts =
            serde_json::from_value(wire["transition"]["facts"].clone()).unwrap();
        assert_eq!(Some(actual_facts), expected.facts);

        TransitionGoldenCase {
            id: id.to_owned(),
            description: description.to_owned(),
            covers: covers.iter().map(|cover| (*cover).to_owned()).collect(),
            normalized: camelize_keys(wire),
            wire_json,
        }
    }
}

#[test]
fn transition_v1_golden_matches_the_real_binding_encoder() {
    let generated = generate_fixture();
    assert_semantic_coverage(&generated);
    update_fixture_if_requested(&generated);

    let committed = load_fixture();
    assert_eq!(
        committed, generated,
        "run {UPDATE_ENV}=1 to refresh the golden"
    );
}

#[test]
fn transition_v1_fixture_schema_denies_unknown_fields() {
    let fixture = load_fixture();
    let mut top_level = serde_json::to_value(&fixture).unwrap();
    top_level["unexpected"] = Value::Bool(true);
    assert!(serde_json::from_value::<TransitionGoldenFixture>(top_level).is_err());

    let mut case = serde_json::to_value(&fixture).unwrap();
    case["cases"][0]["unexpected"] = Value::Bool(true);
    assert!(serde_json::from_value::<TransitionGoldenFixture>(case).is_err());

    let mut invalid_schema = serde_json::to_value(&fixture).unwrap();
    invalid_schema["invalid_transition_schemas"][0]["unexpected"] = Value::Bool(true);
    assert!(serde_json::from_value::<TransitionGoldenFixture>(invalid_schema).is_err());
}

fn generate_fixture() -> TransitionGoldenFixture {
    let text_id = NodeId::new(1);
    let citation_id = NodeId::new(2);
    let paragraph_id = NodeId::new(3);
    let inserted_id = NodeId::new(4);
    let citation_paragraph_id = NodeId::new(5);
    let resource_id = ResourceId::new(1);
    let empty = ChildList::empty();
    let original_resource = citation_resource(resource_id, "old", None);

    let mut builder = GoldenBuilder::new();
    let start = ChangeSet::start_epoch(
        Epoch::new(1),
        ChangeId::new("golden:node-insert").unwrap(),
        None,
        SourceDelta::append(SourceCursor::new(0), "A"),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: SourceCursor::new(1),
            },
            ProjectionOp::InsertResource {
                resource: original_resource.clone(),
            },
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    text_id,
                    NodeStability::Provisional,
                    range(0, 1),
                    ContentKind::Text {
                        text: SemanticText::Source {},
                    },
                ),
            },
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    citation_id,
                    NodeStability::Stable,
                    range(0, 0),
                    ContentKind::CitationReference {
                        key: "paper".to_owned(),
                        target: Some(original_resource.reference()),
                    },
                ),
            },
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    paragraph_id,
                    NodeStability::Provisional,
                    range(0, 1),
                    ContentKind::Paragraph {},
                ),
            },
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    citation_paragraph_id,
                    NodeStability::Stable,
                    range(0, 0),
                    ContentKind::Paragraph {},
                ),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: citation_paragraph_id,
                },
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![citation_id],
                new_version: ChildList::new(vec![citation_id]).version().clone(),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: paragraph_id,
                },
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![text_id],
                new_version: ChildList::new(vec![text_id]).version().clone(),
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: empty.version().clone(),
                start: 0,
                delete_count: 0,
                insert: vec![citation_paragraph_id, paragraph_id],
                new_version: ChildList::new(vec![citation_paragraph_id, paragraph_id])
                    .version()
                    .clone(),
            },
        ],
    )
    .unwrap();
    let node_insert = builder.capture(
        "node_insert",
        "Starts a generation with inserted nodes, a semantic resource, and node/document child splices.",
        &["continuous", "node_insert", "structure_splice", "resource_insert"],
        start,
    );

    let (text_version, paragraph_version) = {
        let document = builder.oracle.document().unwrap();
        (
            document.node(text_id).unwrap().version.clone(),
            document.node(paragraph_id).unwrap().version.clone(),
        )
    };
    let append = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(1),
        ChangeId::new("golden:retained-append").unwrap(),
        SourceDelta::append(SourceCursor::new(1), "B"),
        vec![
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(1),
                new_cursor: SourceCursor::new(2),
            },
            ProjectionOp::ReplaceNode {
                node_id: text_id,
                expected_version: text_version,
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    range(0, 2),
                    range(0, 2),
                    ContentKind::Text {
                        text: SemanticText::Source {},
                    },
                ),
            },
            ProjectionOp::ReplaceNode {
                node_id: paragraph_id,
                expected_version: paragraph_version,
                projection: NodeProjection::new(
                    NodeStability::Provisional,
                    range(0, 2),
                    range(0, 2),
                    ContentKind::Paragraph {},
                ),
            },
        ],
    )
    .unwrap();
    let retained_append = builder.capture(
        "retained_append",
        "Appends retained source and advances an existing source-backed text projection.",
        &["continuous", "retained_append", "projection_append"],
        append,
    );

    let (text, paragraph, paragraph_children) = {
        let document = builder.oracle.document().unwrap();
        (
            document.node(text_id).unwrap().clone(),
            document.node(paragraph_id).unwrap().clone(),
            document
                .node(paragraph_id)
                .unwrap()
                .children
                .as_slice()
                .to_vec(),
        )
    };
    let stable_text = NodeProjection::new(
        NodeStability::Stable,
        text.source,
        text.body,
        text.content.clone(),
    );
    let stable_paragraph = NodeProjection::new(
        NodeStability::Stable,
        paragraph.source,
        paragraph.body,
        paragraph.content.clone(),
    );
    let mut spliced_children = paragraph_children;
    spliced_children.insert(1, inserted_id);
    let stabilize_and_splice = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(2),
        ChangeId::new("golden:stabilize-splice").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(2)),
        vec![
            ProjectionOp::InsertNode {
                node: ContentNode::leaf(
                    inserted_id,
                    NodeStability::Stable,
                    range(2, 2),
                    ContentKind::SoftBreak {},
                ),
            },
            ProjectionOp::StabilizeNode {
                node_id: text_id,
                expected_version: text.version,
                new_version: stable_text.version,
            },
            ProjectionOp::StabilizeNode {
                node_id: paragraph_id,
                expected_version: paragraph.version,
                new_version: stable_paragraph.version,
            },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Node {
                    node_id: paragraph_id,
                },
                expected_version: paragraph.children.version().clone(),
                start: 1,
                delete_count: 0,
                insert: vec![inserted_id],
                new_version: ChildList::new(spliced_children).version().clone(),
            },
        ],
    )
    .unwrap();
    let stabilize_and_splice = builder.capture(
        "stabilize_and_splice",
        "Stabilizes provisional projections while inserting a stable sibling into retained structure.",
        &["continuous", "node_insert", "stabilize", "structure_splice"],
        stabilize_and_splice,
    );

    let corrected_resource = citation_resource(resource_id, "new", Some("Revised"));
    let resource_version = builder
        .oracle
        .document()
        .unwrap()
        .resource(resource_id)
        .unwrap()
        .version
        .clone();
    let resource_correction = ChangeSet::new(
        Epoch::new(1),
        Sequence::new(3),
        ChangeId::new("golden:resource-correction").unwrap(),
        SourceDelta::unchanged(SourceCursor::new(2)),
        vec![ProjectionOp::ReplaceResource {
            resource_id,
            expected_version: resource_version,
            resource: corrected_resource,
        }],
    )
    .unwrap();
    let resource_correction = builder.capture(
        "resource_correction",
        "Corrects a semantic resource and reports the unchanged dependent node rebound to its new version.",
        &["continuous", "resource_correction", "affected_nodes"],
        resource_correction,
    );

    let predecessor = builder.oracle.document().unwrap().coordinate().clone();
    let full_replace = ChangeSet::start_epoch(
        Epoch::new(2),
        ChangeId::new("golden:full-replace").unwrap(),
        Some(predecessor),
        SourceDelta::append(SourceCursor::new(0), "Z"),
        vec![ProjectionOp::AdvanceProjection {
            expected_cursor: SourceCursor::new(0),
            new_cursor: SourceCursor::new(1),
        }],
    )
    .unwrap();
    let full_replace = builder.capture(
        "full_replace",
        "Starts a successor epoch and exposes the continuity-generation barrier as a coarse replacement.",
        &["full_replace", "continuity_generation"],
        full_replace,
    );

    TransitionGoldenFixture {
        schema: FIXTURE_SCHEMA.to_owned(),
        binding_schema: BINDING_SCHEMA.to_owned(),
        transition_schema: TRANSITION_SCHEMA.to_owned(),
        description: "Shared Rust/TypeScript golden for the finalized transition envelope. wire_json is the exact Rust bindings-core payload; normalized is the typed camelCase view expected from strict consumers.".to_owned(),
        cases: vec![
            node_insert,
            retained_append,
            stabilize_and_splice,
            resource_correction,
            full_replace,
        ],
        invalid_transition_schemas: vec![
            InvalidTransitionSchema {
                id: "old_draft".to_owned(),
                description: "The pre-final draft transition schema must not decode as /1.".to_owned(),
                base_case: "node_insert".to_owned(),
                schema: "mdstream.transitions/draft".to_owned(),
            },
            InvalidTransitionSchema {
                id: "future_version".to_owned(),
                description: "A future transition schema requires an explicit consumer upgrade.".to_owned(),
                base_case: "node_insert".to_owned(),
                schema: "mdstream.transitions/2".to_owned(),
            },
        ],
    }
}

fn assert_semantic_coverage(fixture: &TransitionGoldenFixture) {
    assert_eq!(fixture.schema, FIXTURE_SCHEMA);
    assert_eq!(fixture.binding_schema, BINDING_SCHEMA);
    assert_eq!(fixture.transition_schema, TRANSITION_SCHEMA);

    let node_insert = facts(fixture, "node_insert");
    let TransitionFacts::Continuous {
        nodes,
        structures,
        resources,
        ..
    } = node_insert
    else {
        panic!("node insertion must remain continuous");
    };
    assert_eq!(nodes.len(), 4);
    assert_eq!(structures.len(), 3);
    assert_eq!(resources.len(), 1);

    let retained_append = facts(fixture, "retained_append");
    let TransitionFacts::Continuous { nodes, .. } = retained_append else {
        panic!("retained append must remain continuous");
    };
    let appended = nodes
        .iter()
        .find(|node| node.key.node_id == NodeId::new(1))
        .unwrap();
    assert_eq!(
        appended.text,
        Some(TextTransition::ProjectionAppend {
            range: range(1, 2),
            text: "B".to_owned(),
        })
    );

    let stabilize_and_splice = facts(fixture, "stabilize_and_splice");
    let TransitionFacts::Continuous {
        nodes, structures, ..
    } = stabilize_and_splice
    else {
        panic!("stabilization and splice must remain continuous");
    };
    assert_eq!(structures.len(), 1);
    assert_eq!(structures[0].inserted[0].node_id, NodeId::new(4));
    assert_eq!(
        nodes
            .iter()
            .filter(|node| {
                node.before
                    .as_ref()
                    .is_some_and(|before| before.stability == NodeStability::Provisional)
                    && node
                        .after
                        .as_ref()
                        .is_some_and(|after| after.stability == NodeStability::Stable)
            })
            .count(),
        2
    );

    let resource_correction = facts(fixture, "resource_correction");
    let TransitionFacts::Continuous { resources, .. } = resource_correction else {
        panic!("resource correction must remain continuous");
    };
    assert_eq!(resources.len(), 1);
    assert!(resources[0].before_version.is_some());
    assert!(resources[0].after_version.is_some());
    assert_eq!(resources[0].affected_nodes[0].node_id, NodeId::new(2));

    assert!(matches!(
        facts(fixture, "full_replace"),
        TransitionFacts::FullReplace { .. }
    ));
    assert_eq!(
        fixture
            .invalid_transition_schemas
            .iter()
            .map(|invalid| invalid.schema.as_str())
            .collect::<Vec<_>>(),
        ["mdstream.transitions/draft", "mdstream.transitions/2"]
    );
}

fn facts(fixture: &TransitionGoldenFixture, id: &str) -> TransitionFacts {
    let case = fixture.cases.iter().find(|case| case.id == id).unwrap();
    let wire: Value = serde_json::from_str(&case.wire_json).unwrap();
    serde_json::from_value(wire["transition"]["facts"].clone()).unwrap()
}

fn citation_resource(
    id: ResourceId,
    destination_suffix: &str,
    title: Option<&str>,
) -> SemanticResource {
    SemanticResource::new(
        id,
        SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: "paper".to_owned(),
            destination: format!("https://example.test/{destination_suffix}"),
            title: title.map(str::to_owned),
        },
    )
}

fn transition_options() -> Vec<u8> {
    format!(
        r#"{{"schema":"{BINDING_OPTIONS_SCHEMA}","capture_transitions":true,"protocol":{{"max_source_bytes":"1024","max_nodes":"16","max_resources":"16","max_operations":"32","max_change_structural_items":"64","max_children_per_list":"16"}},"wire":{{"max_reducer_update_bytes":"1048576"}}}}"#
    )
    .into_bytes()
}

fn range(start: u64, end: u64) -> SourceRange {
    SourceRange::new(SourceCursor::new(start), SourceCursor::new(end))
}

fn camelize_keys(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(camelize_keys).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (camelize_key(&key), camelize_keys(value)))
                .collect::<Map<_, _>>(),
        ),
        scalar => scalar,
    }
}

fn camelize_key(key: &str) -> String {
    let mut output = String::with_capacity(key.len());
    let mut uppercase_next = false;
    for character in key.chars() {
        if character == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            output.extend(character.to_uppercase());
            uppercase_next = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_PATH)
}

fn load_fixture() -> TransitionGoldenFixture {
    let bytes = fs::read(fixture_path()).expect("transition golden is committed");
    serde_json::from_slice(&bytes).expect("transition golden follows its strict fixture schema")
}

fn update_fixture_if_requested(fixture: &TransitionGoldenFixture) {
    if std::env::var_os(UPDATE_ENV).is_none() {
        return;
    }
    let path = fixture_path();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = serde_json::to_vec_pretty(fixture).unwrap();
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap();
}
