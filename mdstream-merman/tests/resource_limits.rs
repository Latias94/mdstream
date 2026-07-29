mod support;

use mdstream_merman::{MermaidProcessor, MermaidProcessorOptions};
use mdstream_processors::{
    ArtifactHost, ConfigurationVersion, ContentProcessor, ProcessingPolicy, ProcessorFailureCode,
    ProcessorLimits, ProcessorSlotState, run_catching,
};
use mdstream_protocol::{
    CodeBlockSyntax, CodeFenceMarker, ContentKind, NodeId, NodeStability, SemanticText,
};
use merman::render::RenderResourceLimits;

use support::{EPOCH, NODE_ID, document_with_content, mermaid_document};

fn process(
    source: &str,
    limits: RenderResourceLimits,
) -> (ProcessorSlotState, mdstream_merman::MermaidProcessorMetrics) {
    let reducer = mermaid_document(source);
    let document = reducer.document().unwrap();
    let processor =
        MermaidProcessor::new(MermaidProcessorOptions::default().with_resource_limits(limits));
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(EPOCH).unwrap();
    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            ConfigurationVersion::new("merman.test.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = request.key().slot().clone();
    host.complete(document, run_catching(&processor, &request))
        .unwrap();
    (host.state(&slot).unwrap().clone(), processor.metrics())
}

fn unbounded() -> RenderResourceLimits {
    RenderResourceLimits::unbounded_for_trusted_input()
}

fn assert_ready(state: &ProcessorSlotState) -> &str {
    state
        .artifact()
        .and_then(|artifact| artifact.as_text())
        .expect("render must retain an SVG artifact")
}

fn assert_limit_failure<'a>(state: &'a ProcessorSlotState, field: &str) -> &'a str {
    let ProcessorSlotState::Failed { failure, .. } = state else {
        panic!("limit violation must become structured failed state");
    };
    assert_eq!(failure.code(), ProcessorFailureCode::ResourceLimit);
    assert!(failure.message().contains(field), "{}", failure.message());
    failure.message()
}

#[test]
fn source_limit_is_inclusive_and_rejects_before_renderer_invocation() {
    let source = "flowchart TD\nA --> B";
    let mut exact = unbounded();
    exact.max_source_bytes = Some(source.len());
    let (state, metrics) = process(source, exact);
    assert_ready(&state);
    assert_eq!(metrics.renderer_invocations, 1);

    let mut plus_one = exact;
    plus_one.max_source_bytes = Some(source.len() - 1);
    let (state, metrics) = process(source, plus_one);
    let message = assert_limit_failure(&state, "max_source_bytes");
    assert!(message.contains("source"));
    assert_eq!(metrics.renderer_invocations, 0);
    assert_eq!(metrics.materialized_svg_outputs, 0);
}

#[test]
fn flowchart_and_class_model_limits_are_inclusive_before_svg_rendering() {
    let flowchart = "flowchart TD\nA --> B";
    type Configure = fn(&mut RenderResourceLimits);
    let cases: [(&str, Configure, Configure); 2] = [
        (
            "max_flowchart_nodes",
            |limits: &mut RenderResourceLimits| limits.max_flowchart_nodes = Some(2),
            |limits: &mut RenderResourceLimits| limits.max_flowchart_nodes = Some(1),
        ),
        (
            "max_flowchart_edges",
            |limits: &mut RenderResourceLimits| limits.max_flowchart_edges = Some(1),
            |limits: &mut RenderResourceLimits| limits.max_flowchart_edges = Some(0),
        ),
    ];
    for (field, configure_exact, configure_plus_one) in cases {
        let mut exact = unbounded();
        configure_exact(&mut exact);
        assert_ready(&process(flowchart, exact).0);
        let mut plus_one = unbounded();
        configure_plus_one(&mut plus_one);
        let (state, metrics) = process(flowchart, plus_one);
        let message = assert_limit_failure(&state, field);
        assert!(message.contains("layout_model"));
        assert_eq!(metrics.materialized_svg_outputs, 0);
    }

    let flowchart_subgraph = "flowchart TD\nsubgraph Group\nA --> B\nend";
    let mut exact = unbounded();
    exact.max_flowchart_subgraphs = Some(1);
    assert_ready(&process(flowchart_subgraph, exact).0);
    let mut plus_one = exact;
    plus_one.max_flowchart_subgraphs = Some(0);
    assert_limit_failure(
        &process(flowchart_subgraph, plus_one).0,
        "max_flowchart_subgraphs",
    );

    let class = "classDiagram\nclass A\nclass B\nA --> B";
    let mut exact = unbounded();
    exact.max_class_nodes = Some(2);
    exact.max_class_edges = Some(1);
    assert_ready(&process(class, exact).0);

    let mut nodes_plus_one = exact;
    nodes_plus_one.max_class_nodes = Some(1);
    assert_limit_failure(&process(class, nodes_plus_one).0, "max_class_nodes");
    let mut edges_plus_one = exact;
    edges_plus_one.max_class_edges = Some(0);
    assert_limit_failure(&process(class, edges_plus_one).0, "max_class_edges");

    let class_namespace = "classDiagram\nnamespace Shapes {\nclass Triangle\n}";
    let mut exact = unbounded();
    exact.max_class_namespaces = Some(1);
    assert_ready(&process(class_namespace, exact).0);
    let mut plus_one = exact;
    plus_one.max_class_namespaces = Some(0);
    assert_limit_failure(
        &process(class_namespace, plus_one).0,
        "max_class_namespaces",
    );
}

#[test]
fn label_limit_is_inclusive_and_fails_before_svg_rendering() {
    let source = "flowchart TD\nA[Alpha] --> B[Beta]";
    const LABEL_BYTES: usize = 20;
    let mut exact = unbounded();
    exact.max_label_bytes = Some(LABEL_BYTES);
    assert_ready(&process(source, exact).0);

    let mut plus_one = exact;
    plus_one.max_label_bytes = Some(LABEL_BYTES - 1);
    let (state, metrics) = process(source, plus_one);
    let message = assert_limit_failure(&state, "max_label_bytes");
    assert!(message.contains("layout_model"));
    assert_eq!(metrics.materialized_svg_outputs, 0);

    let class = "classDiagram\nclass A\nclass B\nA --> B";
    const CLASS_LABEL_BYTES: usize = 31;
    let mut exact = unbounded();
    exact.max_label_bytes = Some(CLASS_LABEL_BYTES);
    assert_ready(&process(class, exact).0);
    let mut plus_one = exact;
    plus_one.max_label_bytes = Some(CLASS_LABEL_BYTES - 1);
    let (state, metrics) = process(class, plus_one);
    let message = assert_limit_failure(&state, "max_label_bytes");
    assert!(message.contains("layout_model"));
    assert_eq!(metrics.materialized_svg_outputs, 0);
}

#[test]
fn svg_limit_is_post_render_pre_retention_and_records_a_non_allocator_proxy() {
    let source = "flowchart TD\nA[Start] --> B[Done]";
    let (probe, _) = process(source, unbounded());
    let svg_bytes = assert_ready(&probe).len();

    let mut exact = unbounded();
    exact.max_svg_bytes = Some(svg_bytes);
    let (state, metrics) = process(source, exact);
    assert_eq!(assert_ready(&state).len(), svg_bytes);
    assert_eq!(metrics.materialized_svg_outputs, 1);
    assert_eq!(metrics.svg_output_bytes, svg_bytes as u64);
    assert_eq!(metrics.svg_retention_rejections, 0);
    assert_eq!(
        metrics.max_live_input_output_bytes_proxy,
        source.len() + svg_bytes
    );

    let mut plus_one = exact;
    plus_one.max_svg_bytes = Some(svg_bytes - 1);
    let (state, metrics) = process(source, plus_one);
    let message = assert_limit_failure(&state, "max_svg_bytes");
    assert!(message.contains("svg_retention"));
    assert!(message.contains("already materialized"));
    assert_eq!(metrics.materialized_svg_outputs, 1);
    assert_eq!(metrics.svg_output_bytes, svg_bytes as u64);
    assert_eq!(metrics.svg_retention_rejections, 1);
    assert_eq!(
        metrics.max_live_input_output_bytes_proxy,
        source.len() + svg_bytes
    );
}

#[test]
fn artifact_host_limit_includes_protocol_and_media_type_envelope() {
    let source = "flowchart TD\nA --> B";
    let reducer = mermaid_document(source);
    let document = reducer.document().unwrap();
    let processor = MermaidProcessor::default();

    let mut probe_host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    probe_host.begin_epoch(EPOCH).unwrap();
    let probe = probe_host
        .begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            ConfigurationVersion::new("merman.host-limit.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = probe.key().slot().clone();
    probe_host
        .complete(document, run_catching(&processor, &probe))
        .unwrap();
    let artifact_bytes = probe_host.artifact(&slot).unwrap().byte_len();

    let run_with_host_limit = |max_artifact_bytes| {
        let mut host_limits = ProcessorLimits {
            max_artifact_bytes,
            ..ProcessorLimits::default()
        };
        host_limits.max_retained_artifact_bytes = host_limits
            .max_retained_artifact_bytes
            .max(max_artifact_bytes);
        let mut host = ArtifactHost::new(host_limits).unwrap();
        host.begin_epoch(EPOCH).unwrap();
        let request = host
            .begin(
                document,
                processor.descriptor().clone(),
                NODE_ID,
                ConfigurationVersion::new("merman.host-limit.v1").unwrap(),
                ProcessingPolicy::StableOnly,
            )
            .unwrap();
        let slot = request.key().slot().clone();
        host.complete(document, run_catching(&processor, &request))
            .unwrap();
        host.state(&slot).unwrap().clone()
    };

    assert_ready(&run_with_host_limit(artifact_bytes));
    assert_limit_failure(
        &run_with_host_limit(artifact_bytes - 1),
        "processor.artifact_bytes",
    );
}

#[test]
fn artifact_retention_limit_preserves_the_existing_artifact() {
    let first = mermaid_document("flowchart TD\nA --> B");
    let second_id = NodeId::new(42);
    let second = document_with_content(
        EPOCH,
        second_id,
        "flowchart TD\nC --> D",
        NodeStability::Stable,
        ContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced {
                marker: CodeFenceMarker::Backtick,
                length: 3,
            },
            info: Some("mermaid".to_string()),
            text: SemanticText::Source {},
        },
    );
    let first_document = first.document().unwrap();
    let second_document = second.document().unwrap();
    let processor = MermaidProcessor::default();
    let mut host = ArtifactHost::new(ProcessorLimits {
        max_retained_artifacts: 1,
        ..ProcessorLimits::default()
    })
    .unwrap();
    host.begin_epoch(EPOCH).unwrap();

    let first_request = host
        .begin(
            first_document,
            processor.descriptor().clone(),
            NODE_ID,
            ConfigurationVersion::new("merman.retention.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let first_slot = first_request.key().slot().clone();
    host.complete(first_document, run_catching(&processor, &first_request))
        .unwrap();
    let first_artifact = host.artifact(&first_slot).unwrap().clone();

    let second_request = host
        .begin(
            second_document,
            processor.descriptor().clone(),
            second_id,
            ConfigurationVersion::new("merman.retention.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let second_slot = second_request.key().slot().clone();
    host.complete(second_document, run_catching(&processor, &second_request))
        .unwrap();

    let message = assert_limit_failure(
        host.state(&second_slot).unwrap(),
        "processor.retained_artifacts",
    );
    assert!(message.contains("limit 1 exceeded by 2"));
    assert_eq!(host.artifact(&first_slot), Some(&first_artifact));
    assert_eq!(host.metrics().retained_artifacts, 1);
}
