mod support;

use mdstream_merman::{
    DEFAULT_CONFIGURATION_VERSION, MERMAID_ARTIFACT_PROTOCOL, MERMAID_MEDIA_TYPE, MermaidProcessor,
    MermaidProcessorOptions,
};
use mdstream_processors::{
    ArtifactHost, CompletionOutcome, ConfigurationVersion, ContentProcessor, HostError,
    ProcessingPolicy, ProcessorFailureCode, ProcessorLimits, ProcessorSlotState, run_catching,
};
use mdstream_protocol::Epoch;
use mdstream_protocol::{
    CodeBlockSyntax, CodeFenceMarker, ContentKind, NodeStability, SemanticText,
};

use support::{
    EPOCH, NODE_ID, document_with_content, mermaid_document, paragraph_document,
    provisional_mermaid_document,
};

fn configuration() -> ConfigurationVersion {
    ConfigurationVersion::new(DEFAULT_CONFIGURATION_VERSION).unwrap()
}

#[test]
fn stable_typed_mermaid_installs_keyed_svg_without_mutating_canonical_state() {
    let reducer = mermaid_document("flowchart TD\nA[Start] --> B[Done]");
    let document = reducer.document().unwrap();
    let canonical_before = document.snapshot();
    let processor = MermaidProcessor::default();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(EPOCH).unwrap();

    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = request.key().slot().clone();
    assert_eq!(
        host.complete(document, run_catching(&processor, &request))
            .unwrap(),
        CompletionOutcome::Applied
    );

    let state = host.state(&slot).unwrap();
    assert_eq!(state.key(), request.key());
    let artifact = state.artifact().unwrap();
    assert_eq!(artifact.protocol(), MERMAID_ARTIFACT_PROTOCOL);
    assert_eq!(artifact.media_type(), MERMAID_MEDIA_TYPE);
    assert!(artifact.as_text().unwrap().starts_with("<svg"));
    assert_eq!(document.snapshot(), canonical_before);

    let metrics = processor.metrics();
    assert_eq!(metrics.renderer_invocations, 1);
    assert_eq!(metrics.materialized_svg_outputs, 1);
    assert_eq!(
        metrics.svg_output_bytes,
        artifact.as_text().unwrap().len() as u64
    );
    assert_eq!(metrics.svg_retention_rejections, 0);
    assert_eq!(
        metrics.max_live_input_output_bytes_proxy,
        document.source().len() + artifact.as_text().unwrap().len()
    );
}

#[test]
fn unsupported_and_invalid_content_become_structured_failures() {
    let processor = MermaidProcessor::default();
    for (reducer, expected_code) in [
        (
            paragraph_document("plain text"),
            ProcessorFailureCode::UnsupportedContent,
        ),
        (
            mermaid_document("flowchart TD\nA["),
            ProcessorFailureCode::InvalidContext,
        ),
        (mermaid_document(" "), ProcessorFailureCode::InvalidContext),
    ] {
        let document = reducer.document().unwrap();
        let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
        host.begin_epoch(EPOCH).unwrap();
        let request = host
            .begin(
                document,
                processor.descriptor().clone(),
                NODE_ID,
                configuration(),
                ProcessingPolicy::StableOnly,
            )
            .unwrap();
        let slot = request.key().slot().clone();
        host.complete(document, run_catching(&processor, &request))
            .unwrap();

        let ProcessorSlotState::Failed { failure, .. } = host.state(&slot).unwrap() else {
            panic!("unsupported or invalid Mermaid must fail without an artifact");
        };
        assert_eq!(failure.code(), expected_code);
    }
}

#[test]
fn provisional_rendering_is_disabled_by_default() {
    let reducer = provisional_mermaid_document("flowchart TD\nA --> B");
    let document = reducer.document().unwrap();
    let processor = MermaidProcessor::default();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(EPOCH).unwrap();

    assert!(matches!(
        host.begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::AllowProvisional,
        ),
        Err(HostError::ProvisionalProcessingDisabled(id)) if id == NODE_ID
    ));
    assert_eq!(processor.metrics().renderer_invocations, 0);

    let processor =
        MermaidProcessor::new(MermaidProcessorOptions::default().with_provisional_rendering(true));
    assert!(matches!(
        host.begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            ConfigurationVersion::new("merman.provisional.v1").unwrap(),
            ProcessingPolicy::StableOnly,
        ),
        Err(HostError::ProvisionalProcessingDisabled(id)) if id == NODE_ID
    ));
    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            ConfigurationVersion::new("merman.provisional.v1").unwrap(),
            ProcessingPolicy::AllowProvisional,
        )
        .unwrap();
    let slot = request.key().slot().clone();
    host.complete(document, run_catching(&processor, &request))
        .unwrap();
    assert!(host.artifact(&slot).is_some());
}

#[test]
fn detected_but_unavailable_render_family_is_unsupported_content() {
    let reducer = mermaid_document("flowchart-elk TD\nA --> B");
    let document = reducer.document().unwrap();
    let processor = MermaidProcessor::default();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(EPOCH).unwrap();
    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = request.key().slot().clone();
    host.complete(document, run_catching(&processor, &request))
        .unwrap();
    let ProcessorSlotState::Failed { failure, .. } = host.state(&slot).unwrap() else {
        panic!("unavailable Merman render family must not retain an artifact");
    };
    assert_eq!(failure.code(), ProcessorFailureCode::UnsupportedContent);
}

#[test]
fn normalized_semantic_code_is_rendered_instead_of_the_raw_body() {
    let reducer = document_with_content(
        EPOCH,
        NODE_ID,
        "this raw body is intentionally not Mermaid",
        NodeStability::Stable,
        ContentKind::CodeBlock {
            syntax: CodeBlockSyntax::Fenced {
                marker: CodeFenceMarker::Backtick,
                length: 3,
            },
            info: Some("Mermaid extra-info".to_string()),
            text: SemanticText::Normalized {
                value: "flowchart TD\nA --> B".to_string(),
            },
        },
    );
    let document = reducer.document().unwrap();
    let processor = MermaidProcessor::default();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(EPOCH).unwrap();
    let request = host
        .begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = request.key().slot().clone();
    host.complete(document, run_catching(&processor, &request))
        .unwrap();

    assert!(host.artifact(&slot).unwrap().as_text().is_some());
}

#[test]
fn svg_identity_is_slot_derived_and_stable_across_request_generations() {
    let reducer = mermaid_document("flowchart TD\nA --> B");
    let document = reducer.document().unwrap();
    let processor = MermaidProcessor::default();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(EPOCH).unwrap();

    let first = host
        .begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let slot = first.key().slot().clone();
    host.complete(document, run_catching(&processor, &first))
        .unwrap();
    let first_svg = host.artifact(&slot).unwrap().as_text().unwrap().to_string();

    let second = host
        .begin(
            document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert_ne!(first.key().generation(), second.key().generation());
    host.complete(document, run_catching(&processor, &second))
        .unwrap();
    assert_eq!(
        host.artifact(&slot).unwrap().as_text(),
        Some(first_svg.as_str())
    );
}

#[test]
fn replacement_and_epoch_reset_reject_old_render_results_as_stale() {
    let old_reducer = mermaid_document("flowchart TD\nA --> B");
    let replacement_reducer = mermaid_document("flowchart TD\nA --> C");
    let old_document = old_reducer.document().unwrap();
    let replacement_document = replacement_reducer.document().unwrap();
    let processor = MermaidProcessor::default();
    let mut host = ArtifactHost::new(ProcessorLimits::default()).unwrap();
    host.begin_epoch(EPOCH).unwrap();

    let old_request = host
        .begin(
            old_document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let old_result = run_catching(&processor, &old_request);
    let replacement = host
        .begin(
            replacement_document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    assert!(old_request.is_cancelled());
    host.complete(replacement_document, run_catching(&processor, &replacement))
        .unwrap();
    let slot = replacement.key().slot().clone();
    let current_key = replacement.key().clone();
    let current_artifact = host.artifact(&slot).unwrap().clone();

    assert_eq!(
        host.complete(replacement_document, old_result).unwrap(),
        CompletionOutcome::Stale
    );
    assert_eq!(host.state(&slot).unwrap().key(), &current_key);
    assert_eq!(host.artifact(&slot), Some(&current_artifact));

    let reset_request = host
        .begin(
            replacement_document,
            processor.descriptor().clone(),
            NODE_ID,
            configuration(),
            ProcessingPolicy::StableOnly,
        )
        .unwrap();
    let reset_result = run_catching(&processor, &reset_request);
    host.begin_epoch(Epoch::new(8)).unwrap();
    assert!(reset_request.is_cancelled());
    assert_eq!(
        host.complete(replacement_document, reset_result).unwrap(),
        CompletionOutcome::Stale
    );
    assert_eq!(host.metrics().retained_artifacts, 0);
    assert_eq!(host.metrics().retained_artifact_bytes, 0);
}

#[test]
fn manifests_keep_merman_out_of_the_core_workspace() {
    let adapter = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = adapter.parent().unwrap();
    let root_manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let adapter_manifest = std::fs::read_to_string(adapter.join("Cargo.toml")).unwrap();

    assert!(root_manifest.contains("exclude = [\"fuzz\", \"mdstream-merman\"]"));
    assert!(adapter_manifest.contains("rust-version = \"1.95\""));
    assert!(adapter_manifest.contains("version = \"=0.8.0-alpha.3\""));
    assert!(adapter_manifest.contains("version = \"=0.4.0\""));
    for core_manifest in [
        "mdstream/Cargo.toml",
        "mdstream-protocol/Cargo.toml",
        "mdstream-processors/Cargo.toml",
    ] {
        let manifest = std::fs::read_to_string(root.join(core_manifest)).unwrap();
        assert!(
            !manifest
                .lines()
                .any(|line| line.trim_start().starts_with("merman")),
            "{core_manifest} must not depend on Merman"
        );
    }
}
