use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use mdstream_processors::{
    ContentProcessor, ProcessorArtifact, ProcessorCapabilities, ProcessorDescriptor,
    ProcessorFailure, ProcessorFailureCode, ProcessorInput, ProcessorRequest,
};
use mdstream_protocol::{ContentKind, NodeStability, SemanticText};
use merman::render::{HeadlessError, HeadlessRenderer, RenderError, ResourceLimitExceeded};

use crate::MermaidProcessorOptions;

/// Versioned protocol for an untrusted, derived Mermaid SVG artifact.
///
/// Consumers must apply their own embedding policy. Do not inject the payload
/// into an HTML document with an unrestricted `innerHTML` sink.
pub const MERMAID_ARTIFACT_PROTOCOL: &str = "mdstream.mermaid.svg/1";
/// Media type of the raw derived SVG payload.
pub const MERMAID_MEDIA_TYPE: &str = "image/svg+xml";
const PROCESSOR_ID: &str = "mdstream.merman";
const PROCESSOR_VERSION: &str = "0.8.0-alpha.3+mdstream.1";

/// Adapter-owned measurements that do not claim allocator or RSS coverage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MermaidProcessorMetrics {
    /// Calls that crossed the source preflight and entered Merman.
    pub renderer_invocations: u64,
    /// Complete SVG strings returned by Merman, including retention rejects.
    pub materialized_svg_outputs: u64,
    /// Aggregate UTF-8 bytes in complete SVG strings returned by Merman.
    pub svg_output_bytes: u64,
    /// Materialized SVG strings rejected before artifact construction.
    pub svg_retention_rejections: u64,
    /// Maximum `source.len() + svg.len()` observed at the adapter boundary.
    ///
    /// This is a deterministic live input/output proxy. It excludes Merman's
    /// semantic model, layout graph, temporary renderer allocations, allocator
    /// overhead, and process RSS, so it must not be interpreted as peak memory.
    pub max_live_input_output_bytes_proxy: usize,
}

#[derive(Debug, Default)]
struct MetricsState {
    renderer_invocations: AtomicU64,
    materialized_svg_outputs: AtomicU64,
    svg_output_bytes: AtomicU64,
    svg_retention_rejections: AtomicU64,
    max_live_input_output_bytes_proxy: AtomicUsize,
}

impl MetricsState {
    fn snapshot(&self) -> MermaidProcessorMetrics {
        MermaidProcessorMetrics {
            renderer_invocations: self.renderer_invocations.load(Ordering::Relaxed),
            materialized_svg_outputs: self.materialized_svg_outputs.load(Ordering::Relaxed),
            svg_output_bytes: self.svg_output_bytes.load(Ordering::Relaxed),
            svg_retention_rejections: self.svg_retention_rejections.load(Ordering::Relaxed),
            max_live_input_output_bytes_proxy: self
                .max_live_input_output_bytes_proxy
                .load(Ordering::Relaxed),
        }
    }

    fn record_renderer_invocation(&self) {
        saturating_add(&self.renderer_invocations, 1);
    }

    fn record_materialized_svg(&self, source_bytes: usize, svg_bytes: usize) {
        saturating_add(&self.materialized_svg_outputs, 1);
        saturating_add(
            &self.svg_output_bytes,
            u64::try_from(svg_bytes).unwrap_or(u64::MAX),
        );
        self.max_live_input_output_bytes_proxy
            .fetch_max(source_bytes.saturating_add(svg_bytes), Ordering::Relaxed);
    }

    fn record_retention_rejection(&self) {
        saturating_add(&self.svg_retention_rejections, 1);
    }
}

fn saturating_add(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_add(value))
    });
}

/// Trusted, synchronous headless Mermaid processor.
///
/// Schedule it outside reducer, FFI, and artifact-host critical sections via
/// [`mdstream_processors::run_catching`]. Cancellation is checked immediately
/// before and after Merman, but it cannot preempt an in-progress synchronous
/// parse/layout/render call. This type is not a sandbox.
///
/// `max_svg_bytes` covers raw SVG payload bytes. The artifact host separately
/// charges the protocol and media-type envelope against `max_artifact_bytes`.
pub struct MermaidProcessor {
    descriptor: ProcessorDescriptor,
    options: MermaidProcessorOptions,
    renderer: HeadlessRenderer,
    metrics: MetricsState,
}

impl MermaidProcessor {
    pub fn new(options: MermaidProcessorOptions) -> Self {
        let capabilities = if options.allows_provisional() {
            ProcessorCapabilities::with_provisional()
        } else {
            ProcessorCapabilities::stable_only()
        };
        let descriptor = ProcessorDescriptor::new(PROCESSOR_ID, PROCESSOR_VERSION, capabilities)
            .expect("built-in Merman processor identifiers are valid");
        let renderer = HeadlessRenderer::new()
            .with_strict_parsing()
            .with_resource_limits(options.renderer_limits());
        Self {
            descriptor,
            options,
            renderer,
            metrics: MetricsState::default(),
        }
    }

    pub const fn options(&self) -> MermaidProcessorOptions {
        self.options
    }

    pub fn metrics(&self) -> MermaidProcessorMetrics {
        self.metrics.snapshot()
    }

    fn semantic_source<'a>(&self, input: &'a ProcessorInput) -> Result<&'a str, ProcessorFailure> {
        let ContentKind::CodeBlock { text, .. } = &input.node().content else {
            return Err(unsupported_content());
        };
        if !input.node().content.is_mermaid_code_block() {
            return Err(unsupported_content());
        }
        if input.node().stability == NodeStability::Provisional
            && !self.options.allows_provisional()
        {
            return Err(ProcessorFailure::new(
                ProcessorFailureCode::InvalidRequest,
                "Merman processor requires a stable Mermaid node",
            ));
        }
        Ok(match text {
            SemanticText::Source {} => input.body(),
            SemanticText::Normalized { value } => value,
        })
    }

    fn render(&self, request: &ProcessorRequest) -> Result<String, ProcessorFailure> {
        if request.is_cancelled() {
            return Err(cancelled());
        }
        let source = self.semantic_source(request.input())?;
        self.options
            .resource_limits()
            .check_source_bytes(source)
            .map_err(resource_limit_failure)?;

        self.metrics.record_renderer_invocation();
        let diagram_id = format!(
            "mdstream-{}-{}",
            request.key().slot().epoch().get(),
            request.key().slot().node_id().get()
        );
        let svg = self
            .renderer
            .clone()
            .with_diagram_id(&diagram_id)
            .render_svg_sync(source)
            .map_err(headless_failure)?
            .ok_or_else(|| {
                ProcessorFailure::new(
                    ProcessorFailureCode::InvalidContext,
                    "Merman did not detect a complete Mermaid diagram",
                )
            })?;

        self.metrics
            .record_materialized_svg(source.len(), svg.len());
        if request.is_cancelled() {
            return Err(cancelled());
        }
        if let Some(max) = self.options.resource_limits().max_svg_bytes
            && svg.len() > max
        {
            self.metrics.record_retention_rejection();
            return Err(ProcessorFailure::new(
                ProcessorFailureCode::ResourceLimit,
                format!(
                    "resource limit exceeded during svg_retention: max_svg_bytes actual={} max={max}; SVG was already materialized and is not retained",
                    svg.len()
                ),
            ));
        }
        Ok(svg)
    }
}

impl Default for MermaidProcessor {
    fn default() -> Self {
        Self::new(MermaidProcessorOptions::default())
    }
}

impl ContentProcessor for MermaidProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        &self.descriptor
    }

    fn process(&self, request: &ProcessorRequest) -> Result<ProcessorArtifact, ProcessorFailure> {
        let svg = self.render(request)?;
        ProcessorArtifact::text(MERMAID_ARTIFACT_PROTOCOL, MERMAID_MEDIA_TYPE, svg).map_err(
            |error| {
                ProcessorFailure::new(
                    ProcessorFailureCode::Processor,
                    format!("Merman artifact construction failed: {error}"),
                )
            },
        )
    }
}

fn unsupported_content() -> ProcessorFailure {
    ProcessorFailure::new(
        ProcessorFailureCode::UnsupportedContent,
        "Merman processor requires a typed Mermaid code block",
    )
}

fn cancelled() -> ProcessorFailure {
    ProcessorFailure::new(
        ProcessorFailureCode::Cancelled,
        "Merman processor request cancelled",
    )
}

fn resource_limit_failure(error: ResourceLimitExceeded) -> ProcessorFailure {
    ProcessorFailure::new(ProcessorFailureCode::ResourceLimit, error.to_string())
}

fn headless_failure(error: HeadlessError) -> ProcessorFailure {
    match error {
        HeadlessError::Parse(merman::Error::UnsupportedDiagram { diagram_type }) => {
            ProcessorFailure::new(
                ProcessorFailureCode::UnsupportedContent,
                format!("Merman does not support Mermaid diagram type `{diagram_type}`"),
            )
        }
        HeadlessError::Parse(error) => ProcessorFailure::new(
            ProcessorFailureCode::InvalidContext,
            format!("Merman parse failed: {error}"),
        ),
        HeadlessError::Render(RenderError::ResourceLimitExceeded(error)) => {
            resource_limit_failure(error)
        }
        HeadlessError::Render(RenderError::UnsupportedDiagram { diagram_type }) => {
            ProcessorFailure::new(
                ProcessorFailureCode::UnsupportedContent,
                format!("Merman cannot render Mermaid diagram type `{diagram_type}`"),
            )
        }
        HeadlessError::Render(error) => ProcessorFailure::new(
            ProcessorFailureCode::Processor,
            format!("Merman render failed: {error}"),
        ),
    }
}
