use merman::render::RenderResourceLimits;

/// Configuration identifier suitable only for [`MermaidProcessorOptions::default`].
///
/// Callers that change any option must provide a distinct
/// [`mdstream_processors::ConfigurationVersion`] when beginning a request so
/// the artifact host's complete key describes the actual render configuration.
pub const DEFAULT_CONFIGURATION_VERSION: &str = "merman.default.v1";

/// Construction options for [`crate::MermaidProcessor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MermaidProcessorOptions {
    resource_limits: RenderResourceLimits,
    allow_provisional: bool,
}

impl MermaidProcessorOptions {
    /// Replaces Merman's source, supported model, and SVG-retention limits.
    ///
    /// Model, edge, and label fields are currently hard pre-layout limits only
    /// for Merman's flowchart and class families. Source limits cover every
    /// family before parsing. `max_svg_bytes` covers the complete output only
    /// after rendering and before artifact construction.
    #[must_use]
    pub const fn with_resource_limits(mut self, limits: RenderResourceLimits) -> Self {
        self.resource_limits = limits;
        self
    }

    /// Explicitly enables provisional-node processing capability.
    ///
    /// The artifact host must independently opt into
    /// `ProcessingPolicy::AllowProvisional`; both switches are required.
    #[must_use]
    pub const fn with_provisional_rendering(mut self, enabled: bool) -> Self {
        self.allow_provisional = enabled;
        self
    }

    pub const fn resource_limits(self) -> RenderResourceLimits {
        self.resource_limits
    }

    pub const fn allows_provisional(self) -> bool {
        self.allow_provisional
    }

    pub(crate) const fn renderer_limits(self) -> RenderResourceLimits {
        let mut limits = self.resource_limits;
        // Merman checks this only after allocating the complete SVG String.
        // The adapter needs the String so it can record its true output size
        // and distinguish pre-retention rejection from a renderer hard cap.
        limits.max_svg_bytes = None;
        limits
    }
}

impl Default for MermaidProcessorOptions {
    fn default() -> Self {
        Self {
            resource_limits: RenderResourceLimits::interactive(),
            allow_provisional: false,
        }
    }
}
