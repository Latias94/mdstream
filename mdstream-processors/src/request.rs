use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use mdstream_protocol::{
    ContentNode, Document, NodeId, NodeStability, ProcessorInputVersion, SemanticResource,
};

use crate::{
    ConfigurationVersion, HostError, ProcessorArtifact, ProcessorFailure, ProcessorId,
    ProcessorRequestKey, ProcessorVersion,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessorCapabilities {
    provisional: bool,
}

impl ProcessorCapabilities {
    pub const fn stable_only() -> Self {
        Self { provisional: false }
    }

    pub const fn with_provisional() -> Self {
        Self { provisional: true }
    }

    pub const fn accepts_provisional(self) -> bool {
        self.provisional
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProcessingPolicy {
    #[default]
    StableOnly,
    AllowProvisional,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessorDescriptor {
    id: ProcessorId,
    version: ProcessorVersion,
    capabilities: ProcessorCapabilities,
}

impl ProcessorDescriptor {
    pub fn new(
        id: impl Into<String>,
        version: impl Into<String>,
        capabilities: ProcessorCapabilities,
    ) -> Result<Self, crate::IdentifierError> {
        Ok(Self {
            id: ProcessorId::new(id)?,
            version: ProcessorVersion::new(version)?,
            capabilities,
        })
    }

    pub fn id(&self) -> &ProcessorId {
        &self.id
    }

    pub fn version(&self) -> &ProcessorVersion {
        &self.version
    }

    pub const fn capabilities(&self) -> ProcessorCapabilities {
        self.capabilities
    }
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// An owned, node-local snapshot of exactly what a processor may inspect.
///
/// Descendant projections are intentionally excluded; a processor that needs
/// a subtree or document view requires a separate host-owned input contract.
pub struct ProcessorInput {
    node: ContentNode,
    body: String,
    resource: Option<SemanticResource>,
    version: ProcessorInputVersion,
    byte_len: usize,
}

impl ProcessorInput {
    pub(crate) fn from_parts(
        node: ContentNode,
        body: impl Into<String>,
        resource: Option<SemanticResource>,
    ) -> Result<Self, HostError> {
        let body = body.into();
        let version = node.processor_input_version_with_context(&body, resource.as_ref());
        let byte_len = node
            .checked_processor_input_byte_len_with_context(&body, resource.as_ref())
            .and_then(|byte_len| byte_len.checked_add(version.as_str().len()))
            .ok_or(HostError::CounterOverflow("processor.input_bytes"))?;
        Ok(Self {
            node,
            body,
            resource,
            version,
            byte_len,
        })
    }

    pub fn from_document(document: &Document, node_id: NodeId) -> Result<Self, HostError> {
        let (node, body, resource) = document_parts(document, node_id)?;
        Self::from_parts(node.clone(), body, resource.cloned())
    }

    pub(crate) fn version_from_document(
        document: &Document,
        node_id: NodeId,
    ) -> Result<ProcessorInputVersion, HostError> {
        let (node, body, resource) = document_parts(document, node_id)?;
        Ok(node.processor_input_version_with_context(body, resource))
    }

    pub fn node(&self) -> &ContentNode {
        &self.node
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn resource(&self) -> Option<&SemanticResource> {
        self.resource.as_ref()
    }

    pub fn version(&self) -> &ProcessorInputVersion {
        &self.version
    }

    /// Returns deterministic logical bytes charged to processor input budgets.
    pub const fn byte_len(&self) -> usize {
        self.byte_len
    }
}

fn document_parts(
    document: &Document,
    node_id: NodeId,
) -> Result<(&ContentNode, &str, Option<&SemanticResource>), HostError> {
    let node = document
        .node(node_id)
        .ok_or(HostError::NodeNotFound(node_id))?;
    let start =
        usize::try_from(node.body.start.get()).map_err(|_| HostError::InvalidBodyRange(node_id))?;
    let end =
        usize::try_from(node.body.end.get()).map_err(|_| HostError::InvalidBodyRange(node_id))?;
    let body = document
        .source()
        .get(start..end)
        .ok_or(HostError::InvalidBodyRange(node_id))?;
    let resource = node
        .content
        .referenced_resource()
        .and_then(|resource_id| document.resource(resource_id));
    Ok((node, body, resource))
}

#[derive(Debug, Clone)]
pub struct ProcessorRequest {
    key: ProcessorRequestKey,
    input: ProcessorInput,
    cancellation: CancellationToken,
}

impl ProcessorRequest {
    pub(crate) fn new(
        key: ProcessorRequestKey,
        input: ProcessorInput,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            key,
            input,
            cancellation,
        }
    }

    pub fn key(&self) -> &ProcessorRequestKey {
        &self.key
    }

    pub fn input(&self) -> &ProcessorInput {
        &self.input
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

/// Synchronous trusted processor logic, scheduled outside the artifact host.
pub trait ContentProcessor: Send + Sync {
    fn descriptor(&self) -> &ProcessorDescriptor;

    fn process(&self, request: &ProcessorRequest) -> Result<ProcessorArtifact, ProcessorFailure>;
}

pub(crate) fn provisional_allowed(
    stability: NodeStability,
    capabilities: ProcessorCapabilities,
    policy: ProcessingPolicy,
) -> bool {
    stability == NodeStability::Stable
        || (capabilities.accepts_provisional() && policy == ProcessingPolicy::AllowProvisional)
}

pub(crate) struct BeginRequest {
    pub descriptor: ProcessorDescriptor,
    pub configuration: ConfigurationVersion,
    pub input: ProcessorInput,
}
