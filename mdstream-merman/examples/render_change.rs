use mdstream_merman::{DEFAULT_CONFIGURATION_VERSION, MermaidProcessor};
use mdstream_processors::{
    ArtifactHost, ConfigurationVersion, ContentProcessor, ProcessingPolicy, ProcessorLimits,
    run_catching,
};
use mdstream_protocol::{
    ChangeId, ChangeSet, ChildList, ChildListOwner, CodeBlockSyntax, CodeFenceMarker, ContentKind,
    ContentNode, Epoch, NodeId, NodeStability, ProjectionOp, Reducer, SemanticText, SourceCursor,
    SourceDelta, SourceRange,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = "flowchart TD\nA[Token chunks] --> B[Content IR]";
    let epoch = Epoch::new(1);
    let node_id = NodeId::new(1);
    let end = SourceCursor::new(source.len() as u64);
    let range = SourceRange::new(SourceCursor::new(0), end);
    let node = ContentNode::leaf(
        node_id,
        NodeStability::Stable,
        range,
        ContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced {
                marker: CodeFenceMarker::Backtick,
                length: 3,
            },
            info: Some("mermaid".to_string()),
            text: SemanticText::Source {},
        },
    );
    let roots = ChildList::new(vec![node_id]);
    let change = ChangeSet::start_epoch(
        epoch,
        ChangeId::new("example:epoch:1")?,
        None,
        SourceDelta::append(SourceCursor::new(0), source),
        vec![
            ProjectionOp::InsertNode { node },
            ProjectionOp::SpliceChildren {
                owner: ChildListOwner::Document,
                expected_version: ChildList::empty().version().clone(),
                start: 0,
                delete_count: 0,
                insert: roots.as_slice().to_vec(),
                new_version: roots.version().clone(),
            },
            ProjectionOp::AdvanceProjection {
                expected_cursor: SourceCursor::new(0),
                new_cursor: end,
            },
        ],
    )?;
    let mut reducer = Reducer::new();
    reducer.apply(change)?;
    let document = reducer.document().expect("epoch change creates a document");

    let processor = MermaidProcessor::default();
    let mut host = ArtifactHost::new(ProcessorLimits::default())?;
    host.begin_epoch(epoch)?;
    let request = host.begin(
        document,
        processor.descriptor().clone(),
        node_id,
        ConfigurationVersion::new(DEFAULT_CONFIGURATION_VERSION)?,
        ProcessingPolicy::StableOnly,
    )?;
    let slot = request.key().slot().clone();
    host.complete(document, run_catching(&processor, &request))?;

    let svg = host
        .artifact(&slot)
        .and_then(|artifact| artifact.as_text())
        .expect("valid Mermaid produces a retained SVG");
    let metrics = processor.metrics();
    println!(
        "svg_bytes={} live_io_proxy={}",
        svg.len(),
        metrics.max_live_input_output_bytes_proxy
    );
    Ok(())
}
