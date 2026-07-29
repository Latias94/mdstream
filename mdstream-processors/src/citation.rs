use mdstream_protocol::{CitationProtocol, ContentKind, SemanticResourceKind};

use crate::{
    ContentProcessor, ProcessorArtifact, ProcessorCapabilities, ProcessorDescriptor,
    ProcessorFailure, ProcessorFailureCode, ProcessorInput, ProcessorRequest,
};

pub const CITATION_ARTIFACT_PROTOCOL: &str = "mdstream.citation.resolved/1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CitationArtifact {
    key: String,
    destination: String,
    title: Option<String>,
}

impl CitationArtifact {
    pub fn new(
        key: impl Into<String>,
        destination: impl Into<String>,
        title: Option<String>,
    ) -> Self {
        Self {
            key: key.into(),
            destination: destination.into(),
            title,
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn destination(&self) -> &str {
        &self.destination
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub(crate) fn checked_byte_len(&self) -> Option<usize> {
        self.key
            .len()
            .checked_add(self.destination.len())?
            .checked_add(self.title.as_deref().map_or(0, str::len))
    }
}

#[derive(Debug, Clone)]
pub struct CitationProcessor {
    descriptor: ProcessorDescriptor,
}

impl CitationProcessor {
    pub fn new() -> Self {
        Self {
            descriptor: ProcessorDescriptor::new(
                "mdstream.citation",
                "v1",
                ProcessorCapabilities::stable_only(),
            )
            .expect("built-in citation processor identifiers are valid"),
        }
    }

    pub fn resolve(&self, input: &ProcessorInput) -> Result<CitationArtifact, ProcessorFailure> {
        let ContentKind::CitationReference { key, target } = &input.node().content else {
            return Err(ProcessorFailure::new(
                ProcessorFailureCode::UnsupportedContent,
                "citation processor requires a citation reference node",
            ));
        };
        let target = target.as_ref().ok_or_else(|| {
            ProcessorFailure::new(
                ProcessorFailureCode::UnresolvedContext,
                format!("citation `{key}` is unresolved"),
            )
        })?;
        let resource = input.resource().ok_or_else(|| {
            ProcessorFailure::new(
                ProcessorFailureCode::InvalidContext,
                format!("citation `{key}` has no matching resource"),
            )
        })?;
        if resource.id != target.id || resource.version != target.version {
            return Err(ProcessorFailure::new(
                ProcessorFailureCode::InvalidContext,
                format!("citation `{key}` resource identity or version does not match"),
            ));
        }
        let SemanticResourceKind::Citation {
            protocol: CitationProtocol::V1,
            key: resource_key,
            destination,
            title,
        } = &resource.content
        else {
            return Err(ProcessorFailure::new(
                ProcessorFailureCode::InvalidContext,
                format!("citation `{key}` resource has the wrong content type"),
            ));
        };
        if resource_key != key {
            return Err(ProcessorFailure::new(
                ProcessorFailureCode::InvalidContext,
                format!("citation key `{key}` does not match resource key `{resource_key}`"),
            ));
        }
        Ok(CitationArtifact::new(
            key.clone(),
            destination.clone(),
            title.clone(),
        ))
    }
}

impl Default for CitationProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl ContentProcessor for CitationProcessor {
    fn descriptor(&self) -> &ProcessorDescriptor {
        &self.descriptor
    }

    fn process(&self, request: &ProcessorRequest) -> Result<ProcessorArtifact, ProcessorFailure> {
        self.resolve(request.input())
            .map(ProcessorArtifact::citation)
    }
}

#[cfg(test)]
mod tests {
    use mdstream_protocol::{
        CitationProtocol, ContentKind, ContentNode, NodeId, NodeStability, ResourceId,
        SemanticResource, SemanticResourceKind, SourceCursor, SourceRange,
    };

    use super::*;

    #[test]
    fn inconsistent_owned_context_is_rejected() {
        let source = "[@paper]";
        let range = SourceRange::new(SourceCursor::new(0), SourceCursor::new(source.len() as u64));
        let resource = SemanticResource::new(
            ResourceId::new(9),
            SemanticResourceKind::Citation {
                protocol: CitationProtocol::V1,
                key: "different".to_string(),
                destination: "https://example.test/different".to_string(),
                title: None,
            },
        );
        let node = ContentNode::leaf(
            NodeId::new(42),
            NodeStability::Stable,
            range,
            ContentKind::CitationReference {
                key: "paper".to_string(),
                target: Some(resource.reference()),
            },
        );
        let input = ProcessorInput::from_parts(node, source, Some(resource)).unwrap();

        let failure = CitationProcessor::new().resolve(&input).unwrap_err();
        assert_eq!(failure.code(), ProcessorFailureCode::InvalidContext);
        assert!(failure.message().contains("does not match"));
    }
}
