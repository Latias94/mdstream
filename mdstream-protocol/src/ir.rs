use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, MapAccess, Visitor},
};

use crate::{
    NodeId, NodeStability, NodeVersion, ProcessorInputVersion, ProtocolError, ResourceId,
    ResourceVersion, SourceCursor, StructureVersion,
};

pub const CITATION_PROTOCOL: &str = "mdstream.citation/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRange {
    pub start: SourceCursor,
    pub end: SourceCursor,
}

impl SourceRange {
    pub const fn new(start: SourceCursor, end: SourceCursor) -> Self {
        Self { start, end }
    }

    pub fn validate(self, source: &str) -> Result<(), ProtocolError> {
        self.validate_parts(source, "")
    }

    pub(crate) fn validate_parts(self, prefix: &str, suffix: &str) -> Result<(), ProtocolError> {
        let start = usize::try_from(self.start.get()).map_err(|_| ProtocolError::InvalidRange {
            start: self.start,
            end: self.end,
        })?;
        let end = usize::try_from(self.end.get()).map_err(|_| ProtocolError::InvalidRange {
            start: self.start,
            end: self.end,
        })?;
        let total = prefix
            .len()
            .checked_add(suffix.len())
            .ok_or(ProtocolError::CursorOverflow)?;
        let is_boundary = |offset: usize| {
            if offset <= prefix.len() {
                prefix.is_char_boundary(offset)
            } else {
                suffix.is_char_boundary(offset - prefix.len())
            }
        };
        if start > end || end > total || !is_boundary(start) || !is_boundary(end) {
            return Err(ProtocolError::InvalidRange {
                start: self.start,
                end: self.end,
            });
        }
        Ok(())
    }

    pub const fn contains(self, other: Self) -> bool {
        self.start.get() <= other.start.get() && other.end.get() <= self.end.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableAlignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStyle {
    Inline,
    Reference,
    ReferenceUnknown,
    Collapsed,
    CollapsedUnknown,
    Shortcut,
    ShortcutUnknown,
    Autolink,
    Email,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockQuoteKind {
    #[default]
    Plain,
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeFenceMarker {
    Backtick,
    Tilde,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum CodeBlockSyntax {
    Indented,
    Fenced {
        marker: CodeFenceMarker,
        length: u32,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CitationProtocol {
    #[default]
    #[serde(rename = "mdstream.citation/1")]
    V1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
/// How a renderer obtains the semantic text represented by a node.
///
/// `Source` means the canonical body range is already the semantic value.
/// `Normalized` carries a value that cannot be recovered by slicing source,
/// such as decoded entities, escapes, or normalized code whitespace.
pub enum SemanticText {
    Source {},
    Normalized { value: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RetainedTextMetrics {
    pub(crate) bytes: usize,
    pub(crate) capacity: usize,
}

impl RetainedTextMetrics {
    pub(crate) const ZERO: Self = Self {
        bytes: 0,
        capacity: 0,
    };

    pub(crate) fn checked_add(self, other: Self) -> Result<Self, ProtocolError> {
        Ok(Self {
            bytes: self
                .bytes
                .checked_add(other.bytes)
                .ok_or(ProtocolError::MetadataOverflow)?,
            capacity: self
                .capacity
                .checked_add(other.capacity)
                .ok_or(ProtocolError::MetadataOverflow)?,
        })
    }

    pub(crate) fn checked_sub(self, other: Self) -> Result<Self, ProtocolError> {
        Ok(Self {
            bytes: self
                .bytes
                .checked_sub(other.bytes)
                .ok_or(ProtocolError::MetadataOverflow)?,
            capacity: self
                .capacity
                .checked_sub(other.capacity)
                .ok_or(ProtocolError::MetadataOverflow)?,
        })
    }

    fn add_text(&mut self, bytes: usize, capacity: usize) -> Result<(), ProtocolError> {
        *self = self.checked_add(Self { bytes, capacity })?;
        Ok(())
    }

    fn add_string(&mut self, value: &String) -> Result<(), ProtocolError> {
        self.add_text(value.len(), value.capacity())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum SemanticResourceKind {
    Link {
        destination: String,
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        title: Option<String>,
    },
    Footnote {
        label: String,
    },
    Citation {
        protocol: CitationProtocol,
        key: String,
        destination: String,
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        title: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticResource {
    pub id: ResourceId,
    pub version: ResourceVersion,
    pub content: SemanticResourceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRef {
    pub id: ResourceId,
    pub version: ResourceVersion,
}

impl SemanticResource {
    pub fn new(id: ResourceId, content: SemanticResourceKind) -> Self {
        let mut resource = Self {
            id,
            version: ResourceVersion::digest(&[]),
            content,
        };
        resource.version = resource.derived_version();
        resource
    }

    pub fn derived_version(&self) -> ResourceVersion {
        ResourceVersion::digest_json(&self.content)
    }

    pub fn reference(&self) -> ResourceRef {
        ResourceRef {
            id: self.id,
            version: self.version.clone(),
        }
    }

    pub(crate) fn validate_local(&self, limits: ProtocolLimits) -> Result<usize, ProtocolError> {
        if self.version != self.derived_version() {
            return Err(ProtocolError::ResourceVersionMismatch(self.id));
        }
        let mut tally = MetadataTally::new(limits);
        match &self.content {
            SemanticResourceKind::Link { destination, title } => {
                tally.add("resource.link.destination", destination)?;
                if let Some(title) = title {
                    tally.add("resource.link.title", title)?;
                }
            }
            SemanticResourceKind::Footnote { label } => {
                require_nonempty("resource.footnote.label", label)?;
                tally.add("resource.footnote.label", label)?;
            }
            SemanticResourceKind::Citation {
                key,
                destination,
                title,
                ..
            } => {
                require_nonempty("resource.citation.key", key)?;
                tally.add("resource.citation.key", key)?;
                tally.add("resource.citation.destination", destination)?;
                if let Some(title) = title {
                    tally.add("resource.citation.title", title)?;
                }
            }
        }
        tally.finish("resource.metadata")
    }

    pub(crate) fn retained_text_metrics(&self) -> Result<RetainedTextMetrics, ProtocolError> {
        let mut metrics = RetainedTextMetrics::ZERO;
        metrics.add_text(self.version.as_str().len(), self.version.capacity())?;
        match &self.content {
            SemanticResourceKind::Link { destination, title } => {
                metrics.add_string(destination)?;
                if let Some(title) = title {
                    metrics.add_string(title)?;
                }
            }
            SemanticResourceKind::Footnote { label } => metrics.add_string(label)?,
            SemanticResourceKind::Citation {
                key,
                destination,
                title,
                ..
            } => {
                metrics.add_string(key)?;
                metrics.add_string(destination)?;
                if let Some(title) = title {
                    metrics.add_string(title)?;
                }
            }
        }
        Ok(metrics)
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildList {
    version: StructureVersion,
    children: Arc<Vec<NodeId>>,
}

impl Clone for ChildList {
    fn clone(&self) -> Self {
        Self {
            version: self.version.clone(),
            children: Arc::new(self.children.as_ref().clone()),
        }
    }
}

impl ChildList {
    pub fn new(children: Vec<NodeId>) -> Self {
        let mut list = Self {
            version: StructureVersion::digest(&[]),
            children: Arc::new(children),
        };
        list.version = list.derived_version();
        list
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn as_slice(&self) -> &[NodeId] {
        self.children.as_slice()
    }

    pub const fn version(&self) -> &StructureVersion {
        &self.version
    }

    pub fn len(&self) -> usize {
        self.children.len()
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &NodeId> {
        self.children.iter()
    }

    pub fn get(&self, index: usize) -> Option<&NodeId> {
        self.children.get(index)
    }

    pub fn version_for(children: impl IntoIterator<Item = NodeId>) -> StructureVersion {
        extend_structure_version(StructureVersion::digest(&[]), children)
    }

    pub fn derived_version(&self) -> StructureVersion {
        Self::version_for(self.iter().copied())
    }

    pub fn version_after_append(&self, children: &[NodeId]) -> StructureVersion {
        extend_structure_version(self.version.clone(), children.iter().copied())
    }

    pub(crate) fn clone_shared(&self) -> Self {
        Self {
            version: self.version.clone(),
            children: Arc::clone(&self.children),
        }
    }

    pub(crate) fn retained_version_text_metrics(
        &self,
    ) -> Result<RetainedTextMetrics, ProtocolError> {
        let mut metrics = RetainedTextMetrics::ZERO;
        metrics.add_text(self.version.as_str().len(), self.version.capacity())?;
        Ok(metrics)
    }

    pub(crate) fn append_validated(
        &mut self,
        children: Vec<NodeId>,
        new_version: StructureVersion,
    ) {
        Arc::get_mut(&mut self.children)
            .expect("validated child-list appends own their storage")
            .extend(children);
        self.version = new_version;
    }

    pub(crate) fn validate_local(&self, limits: ProtocolLimits) -> Result<(), ProtocolError> {
        if self.version != self.derived_version() {
            return Err(ProtocolError::InvalidChange(
                "child-list version does not match its ordered children".to_string(),
            ));
        }
        if self.children.len() > limits.max_children_per_list {
            return Err(ProtocolError::ValueTooLarge {
                field: "child_list.children",
                limit: limits.max_children_per_list,
                actual: self.children.len(),
            });
        }
        let mut unique = BTreeSet::new();
        for child in self.children.iter() {
            if !unique.insert(*child) {
                return Err(ProtocolError::DuplicateNode(*child));
            }
        }
        Ok(())
    }
}

fn extend_structure_version(
    mut version: StructureVersion,
    children: impl IntoIterator<Item = NodeId>,
) -> StructureVersion {
    let mut input = Vec::with_capacity(version.as_str().len() + 1 + std::mem::size_of::<u128>());
    for child in children {
        input.clear();
        input.extend_from_slice(version.as_str().as_bytes());
        input.push(0);
        input.extend_from_slice(&child.get().to_be_bytes());
        version = StructureVersion::digest(&input);
    }
    version
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum ContentKind {
    Paragraph {},
    Heading {
        level: u8,
    },
    Text {
        text: SemanticText,
    },
    Emphasis {},
    Strong {},
    Strikethrough {},
    Link {
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        target: Option<ResourceRef>,
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        reference_label: Option<String>,
        style: LinkStyle,
    },
    Image {
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        target: Option<ResourceRef>,
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        reference_label: Option<String>,
        style: LinkStyle,
        alt: SemanticText,
    },
    InlineCode {
        text: SemanticText,
    },
    CodeBlock {
        syntax: CodeBlockSyntax,
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        info: Option<String>,
        text: SemanticText,
    },
    List {
        ordered: bool,
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        start: Option<u32>,
        tight: bool,
    },
    ListItem {
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        checked: Option<bool>,
    },
    BlockQuote {
        style: BlockQuoteKind,
    },
    ThematicBreak {},
    Table {
        alignments: Vec<TableAlignment>,
    },
    TableHead {},
    TableBody {},
    TableRow {},
    TableCell {
        column: u32,
    },
    Html {
        block: bool,
        text: SemanticText,
    },
    Math {
        display: bool,
        text: SemanticText,
    },
    FootnoteDefinition {
        label: String,
        target: ResourceRef,
    },
    FootnoteReference {
        label: String,
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        target: Option<ResourceRef>,
    },
    CitationDefinition {
        key: String,
        target: ResourceRef,
    },
    CitationReference {
        key: String,
        #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
        target: Option<ResourceRef>,
    },
    SoftBreak {},
    HardBreak {},
    Custom {
        namespace: String,
        name: String,
        opaque: bool,
        #[serde(deserialize_with = "deserialize_unique_attributes")]
        attributes: BTreeMap<String, String>,
    },
}

fn deserialize_unique_attributes<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct UniqueAttributesVisitor;

    impl<'de> Visitor<'de> for UniqueAttributesVisitor {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object with unique custom attribute names")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut attributes = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                if attributes.insert(key.clone(), value).is_some() {
                    return Err(A::Error::custom(format!(
                        "duplicate custom attribute name `{key}`"
                    )));
                }
            }
            Ok(attributes)
        }
    }

    deserializer.deserialize_map(UniqueAttributesVisitor)
}

impl ContentKind {
    pub fn is_mermaid_code_block(&self) -> bool {
        self.code_language()
            .is_some_and(|language| language.eq_ignore_ascii_case("mermaid"))
    }

    pub fn code_language(&self) -> Option<&str> {
        let Self::CodeBlock {
            info: Some(info), ..
        } = self
        else {
            return None;
        };
        info.split_whitespace().next()
    }

    pub fn referenced_resource(&self) -> Option<ResourceId> {
        self.resource_ref().map(|reference| reference.id)
    }

    pub fn resource_ref(&self) -> Option<&ResourceRef> {
        match self {
            Self::Link { target, .. }
            | Self::Image { target, .. }
            | Self::FootnoteReference { target, .. }
            | Self::CitationReference { target, .. } => target.as_ref(),
            Self::FootnoteDefinition { target, .. } | Self::CitationDefinition { target, .. } => {
                Some(target)
            }
            _ => None,
        }
    }

    pub(crate) fn resource_ref_mut(&mut self) -> Option<&mut ResourceRef> {
        match self {
            Self::Link { target, .. }
            | Self::Image { target, .. }
            | Self::FootnoteReference { target, .. }
            | Self::CitationReference { target, .. } => target.as_mut(),
            Self::FootnoteDefinition { target, .. } | Self::CitationDefinition { target, .. } => {
                Some(target)
            }
            _ => None,
        }
    }

    fn retained_text_metrics(&self) -> Result<RetainedTextMetrics, ProtocolError> {
        let mut metrics = RetainedTextMetrics::ZERO;
        match self {
            Self::Text { text }
            | Self::InlineCode { text }
            | Self::Html { text, .. }
            | Self::Math { text, .. } => add_semantic_text(&mut metrics, text)?,
            Self::Link {
                target,
                reference_label,
                ..
            } => {
                add_resource_ref(&mut metrics, target.as_ref())?;
                if let Some(label) = reference_label {
                    metrics.add_string(label)?;
                }
            }
            Self::Image {
                target,
                reference_label,
                alt,
                ..
            } => {
                add_resource_ref(&mut metrics, target.as_ref())?;
                if let Some(label) = reference_label {
                    metrics.add_string(label)?;
                }
                add_semantic_text(&mut metrics, alt)?;
            }
            Self::CodeBlock { info, text, .. } => {
                if let Some(info) = info {
                    metrics.add_string(info)?;
                }
                add_semantic_text(&mut metrics, text)?;
            }
            Self::FootnoteDefinition { label, target }
            | Self::CitationDefinition { key: label, target } => {
                metrics.add_string(label)?;
                add_resource_ref(&mut metrics, Some(target))?;
            }
            Self::FootnoteReference { label, target }
            | Self::CitationReference { key: label, target } => {
                metrics.add_string(label)?;
                add_resource_ref(&mut metrics, target.as_ref())?;
            }
            Self::Custom {
                namespace,
                name,
                attributes,
                ..
            } => {
                metrics.add_string(namespace)?;
                metrics.add_string(name)?;
                for (key, value) in attributes {
                    metrics.add_string(key)?;
                    metrics.add_string(value)?;
                }
            }
            Self::Paragraph {}
            | Self::Heading { .. }
            | Self::Emphasis {}
            | Self::Strong {}
            | Self::Strikethrough {}
            | Self::List { .. }
            | Self::ListItem { .. }
            | Self::BlockQuote { .. }
            | Self::ThematicBreak {}
            | Self::Table { .. }
            | Self::TableHead {}
            | Self::TableBody {}
            | Self::TableRow {}
            | Self::TableCell { .. }
            | Self::SoftBreak {}
            | Self::HardBreak {} => {}
        }
        Ok(metrics)
    }
}

fn add_semantic_text(
    metrics: &mut RetainedTextMetrics,
    text: &SemanticText,
) -> Result<(), ProtocolError> {
    if let SemanticText::Normalized { value } = text {
        metrics.add_string(value)?;
    }
    Ok(())
}

fn add_resource_ref(
    metrics: &mut RetainedTextMetrics,
    reference: Option<&ResourceRef>,
) -> Result<(), ProtocolError> {
    if let Some(reference) = reference {
        metrics.add_text(
            reference.version.as_str().len(),
            reference.version.capacity(),
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Projection-local node state, excluding child topology.
///
/// Replacing a projection never resends the child list. Processor inputs are
/// node-local: [`ContentNode::processor_input_version`] covers this projection
/// and its direct child identity/order, but never descendant projections.
pub struct NodeProjection {
    pub version: NodeVersion,
    pub stability: NodeStability,
    pub source: SourceRange,
    pub body: SourceRange,
    pub content: ContentKind,
}

impl NodeProjection {
    pub fn new(
        stability: NodeStability,
        source: SourceRange,
        body: SourceRange,
        content: ContentKind,
    ) -> Self {
        let mut projection = Self {
            version: NodeVersion::digest(&[]),
            stability,
            source,
            body,
            content,
        };
        projection.version = projection.derived_version();
        projection
    }

    pub fn derived_version(&self) -> NodeVersion {
        #[derive(Serialize)]
        struct LocalProjection<'a> {
            stability: NodeStability,
            source: SourceRange,
            body: SourceRange,
            content: &'a ContentKind,
        }

        NodeVersion::digest_json(&LocalProjection {
            stability: self.stability,
            source: self.source,
            body: self.body,
            content: &self.content,
        })
    }

    pub(crate) fn validate_local_parts(
        &self,
        node_id: NodeId,
        prefix: &str,
        suffix: &str,
        limits: ProtocolLimits,
    ) -> Result<usize, ProtocolError> {
        self.source.validate_parts(prefix, suffix)?;
        self.body.validate_parts(prefix, suffix)?;
        if !self.source.contains(self.body) {
            return Err(ProtocolError::InvalidChange(
                "node body range must be contained by its source range".to_string(),
            ));
        }
        self.validate_shape(node_id, limits)
    }

    pub(crate) fn validate_shape(
        &self,
        node_id: NodeId,
        limits: ProtocolLimits,
    ) -> Result<usize, ProtocolError> {
        if self.version != self.derived_version() {
            return Err(ProtocolError::VersionMismatch(node_id));
        }
        validate_kind(&self.content, limits)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// A canonical typed node with stable identity and ordered child ownership.
pub struct ContentNode {
    pub id: NodeId,
    pub version: NodeVersion,
    pub stability: NodeStability,
    pub source: SourceRange,
    pub body: SourceRange,
    pub children: ChildList,
    pub content: ContentKind,
}

impl ContentNode {
    pub fn leaf(
        id: NodeId,
        stability: NodeStability,
        source: SourceRange,
        content: ContentKind,
    ) -> Self {
        Self::new(id, stability, source, source, Vec::new(), content)
    }

    pub fn new(
        id: NodeId,
        stability: NodeStability,
        source: SourceRange,
        body: SourceRange,
        children: Vec<NodeId>,
        content: ContentKind,
    ) -> Self {
        Self::from_projection(
            id,
            NodeProjection::new(stability, source, body, content),
            ChildList::new(children),
        )
    }

    pub fn projection(&self) -> NodeProjection {
        NodeProjection {
            version: self.version.clone(),
            stability: self.stability,
            source: self.source,
            body: self.body,
            content: self.content.clone(),
        }
    }

    pub fn derived_version(&self) -> NodeVersion {
        self.projection().derived_version()
    }

    /// Derives a node-local processor input version.
    ///
    /// The version covers the local projection plus the direct child identity
    /// and order. It is not a recursive subtree version: processors that read
    /// descendant projections need a host-owned subtree/document input key.
    pub fn processor_input_version(&self) -> ProcessorInputVersion {
        #[derive(Serialize)]
        struct ProcessorInput<'a> {
            projection: &'a NodeVersion,
            structure: &'a StructureVersion,
        }

        ProcessorInputVersion::digest_json(&ProcessorInput {
            projection: &self.version,
            structure: &self.children.version,
        })
    }

    /// Derives the version of the complete node-local input exposed to a
    /// processor.
    ///
    /// In addition to the local projection and direct child structure, this
    /// covers the canonical body bytes and the directly referenced semantic
    /// resource. Descendant projections remain outside this node-local key.
    pub fn processor_input_version_with_context(
        &self,
        body: &str,
        resource: Option<&SemanticResource>,
    ) -> ProcessorInputVersion {
        self.checked_processor_input_version_and_byte_len_with_context(body, resource)
            .expect("canonical processor inputs always serialize within the address space")
            .0
    }

    /// Derives the complete input version and logical owned-input byte cost in
    /// one canonical serialization pass.
    ///
    /// The byte cost covers the serialized node, escaped body, direct resource,
    /// context field names, and the separately cached version string.
    pub fn checked_processor_input_version_and_byte_len_with_context(
        &self,
        body: &str,
        resource: Option<&SemanticResource>,
    ) -> Option<(ProcessorInputVersion, usize)> {
        #[derive(Serialize)]
        struct ProcessorContext<'a> {
            node: &'a ContentNode,
            body: &'a str,
            resource: Option<&'a SemanticResource>,
        }

        let (version, context_bytes) =
            ProcessorInputVersion::digest_json_with_len(&ProcessorContext {
                node: self,
                body,
                resource,
            })?;
        let byte_len = context_bytes.checked_add(version.as_str().len())?;
        Some((version, byte_len))
    }

    /// Returns deterministic logical bytes for the owned node-local processor
    /// input, excluding the separately cached [`ProcessorInputVersion`].
    ///
    /// This is the canonical context JSON cost and excludes only the separately
    /// cached input-version string. It is a protocol-level budget measure, not
    /// allocator-retained memory.
    pub fn checked_processor_input_byte_len_with_context(
        &self,
        body: &str,
        resource: Option<&SemanticResource>,
    ) -> Option<usize> {
        let (version, byte_len) =
            self.checked_processor_input_version_and_byte_len_with_context(body, resource)?;
        byte_len.checked_sub(version.as_str().len())
    }

    pub(crate) fn clone_shared(&self) -> Self {
        Self {
            id: self.id,
            version: self.version.clone(),
            stability: self.stability,
            source: self.source,
            body: self.body,
            children: self.children.clone_shared(),
            content: self.content.clone(),
        }
    }

    pub(crate) fn from_projection(
        id: NodeId,
        projection: NodeProjection,
        children: ChildList,
    ) -> Self {
        Self {
            id,
            version: projection.version,
            stability: projection.stability,
            source: projection.source,
            body: projection.body,
            children,
            content: projection.content,
        }
    }

    pub(crate) fn validate_local(
        &self,
        source: &str,
        limits: ProtocolLimits,
    ) -> Result<usize, ProtocolError> {
        self.validate_local_parts(source, "", limits)
    }

    pub(crate) fn validate_local_parts(
        &self,
        prefix: &str,
        suffix: &str,
        limits: ProtocolLimits,
    ) -> Result<usize, ProtocolError> {
        let metadata = self
            .projection()
            .validate_local_parts(self.id, prefix, suffix, limits)?;
        self.children.validate_local(limits)?;
        Ok(metadata)
    }

    pub(crate) fn validate_shape(&self, limits: ProtocolLimits) -> Result<usize, ProtocolError> {
        let metadata = self.projection().validate_shape(self.id, limits)?;
        self.children.validate_local(limits)?;
        validate_child_arity(&self.content, self.children.children.len())?;
        Ok(metadata)
    }

    pub(crate) fn retained_text_metrics(&self) -> Result<RetainedTextMetrics, ProtocolError> {
        let mut metrics = self.content.retained_text_metrics()?;
        metrics.add_text(self.version.as_str().len(), self.version.capacity())?;
        metrics = metrics.checked_add(self.children.retained_version_text_metrics()?)?;
        Ok(metrics)
    }
}

pub(crate) fn validate_kind(
    content: &ContentKind,
    limits: ProtocolLimits,
) -> Result<usize, ProtocolError> {
    let mut tally = MetadataTally::new(limits);
    match content {
        ContentKind::Heading { level } if !(1..=6).contains(level) => {
            return Err(ProtocolError::InvalidChange(
                "heading level must be between 1 and 6".to_string(),
            ));
        }
        ContentKind::List { ordered, start, .. }
            if (*ordered && start.is_none()) || (!*ordered && start.is_some()) =>
        {
            return Err(ProtocolError::InvalidChange(
                "ordered lists require a start and unordered lists forbid one".to_string(),
            ));
        }
        ContentKind::List {
            start: Some(start), ..
        } if *start > 999_999_999 => {
            return Err(ProtocolError::InvalidChange(
                "ordered-list markers may contain at most nine digits".to_string(),
            ));
        }
        ContentKind::Table { alignments } if alignments.len() > limits.max_children_per_list => {
            return Err(ProtocolError::ValueTooLarge {
                field: "table.alignments",
                limit: limits.max_children_per_list,
                actual: alignments.len(),
            });
        }
        ContentKind::Text { text }
        | ContentKind::InlineCode { text }
        | ContentKind::Math { text, .. }
        | ContentKind::Html { text, .. } => tally_semantic_text(&mut tally, text)?,
        ContentKind::CodeBlock { syntax, info, text } => {
            validate_code_block_contract(*syntax, info.as_deref())?;
            if let Some(info) = info {
                tally.add("code.info", info)?;
            }
            tally_semantic_text(&mut tally, text)?;
        }
        ContentKind::Link {
            target,
            reference_label,
            style,
        } => {
            validate_link_contract(*style, target.is_some(), reference_label.is_some())?;
            if let Some(label) = reference_label {
                tally.add("reference.label", label)?;
            }
        }
        ContentKind::Image {
            target,
            reference_label,
            style,
            alt,
        } => {
            validate_link_contract(*style, target.is_some(), reference_label.is_some())?;
            if let Some(label) = reference_label {
                tally.add("reference.label", label)?;
            }
            tally_semantic_text(&mut tally, alt)?;
        }
        ContentKind::FootnoteDefinition { label, .. }
        | ContentKind::FootnoteReference { label, .. } => {
            require_nonempty("footnote.label", label)?;
            tally.add("footnote.label", label)?;
        }
        ContentKind::CitationDefinition { key, .. }
        | ContentKind::CitationReference { key, .. } => {
            require_nonempty("citation.key", key)?;
            tally.add("citation.key", key)?;
        }
        ContentKind::Custom {
            namespace,
            name,
            attributes,
            ..
        } => {
            require_nonempty("custom.namespace", namespace)?;
            require_nonempty("custom.name", name)?;
            tally.add("custom.namespace", namespace)?;
            tally.add("custom.name", name)?;
            if attributes.len() > limits.max_attributes_per_node {
                return Err(ProtocolError::ValueTooLarge {
                    field: "custom.attributes",
                    limit: limits.max_attributes_per_node,
                    actual: attributes.len(),
                });
            }
            for (key, value) in attributes {
                tally.add("custom.attribute.key", key)?;
                tally.add("custom.attribute.value", value)?;
            }
        }
        _ => {}
    }
    tally.finish("node.metadata")
}

fn tally_semantic_text(
    tally: &mut MetadataTally,
    text: &SemanticText,
) -> Result<(), ProtocolError> {
    if let SemanticText::Normalized { value } = text {
        tally.add("semantic_text.value", value)?;
    }
    Ok(())
}

fn validate_code_block_contract(
    syntax: CodeBlockSyntax,
    info: Option<&str>,
) -> Result<(), ProtocolError> {
    match syntax {
        CodeBlockSyntax::Indented if info.is_some() => {
            return Err(ProtocolError::InvalidChange(
                "indented code blocks forbid info strings".to_string(),
            ));
        }
        CodeBlockSyntax::Fenced { length, .. } if length < 3 => {
            return Err(ProtocolError::InvalidChange(
                "code fence length must be at least three".to_string(),
            ));
        }
        CodeBlockSyntax::Fenced {
            marker: CodeFenceMarker::Backtick,
            ..
        } if info.is_some_and(|info| info.contains('`')) => {
            return Err(ProtocolError::InvalidChange(
                "backtick code fence info strings cannot contain backticks".to_string(),
            ));
        }
        _ => {}
    }

    if info.is_some_and(|info| info.split_whitespace().next().is_none()) {
        Err(ProtocolError::InvalidChange(
            "code info strings cannot be empty".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn validate_link_contract(
    style: LinkStyle,
    has_target: bool,
    has_label: bool,
) -> Result<(), ProtocolError> {
    let valid = match style {
        LinkStyle::Inline | LinkStyle::Autolink | LinkStyle::Email => has_target && !has_label,
        LinkStyle::Reference | LinkStyle::Collapsed | LinkStyle::Shortcut => {
            has_target && has_label
        }
        LinkStyle::ReferenceUnknown | LinkStyle::CollapsedUnknown | LinkStyle::ShortcutUnknown => {
            !has_target && has_label
        }
    };
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::InvalidChange(
            "link style, target, and reference label are inconsistent".to_string(),
        ))
    }
}

fn validate_child_arity(content: &ContentKind, child_count: usize) -> Result<(), ProtocolError> {
    if child_count > 0 && !may_have_children(content) {
        Err(ProtocolError::InvalidChange(
            "leaf content kind cannot own children".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn may_have_children(content: &ContentKind) -> bool {
    matches!(
        content,
        ContentKind::Paragraph {}
            | ContentKind::Heading { .. }
            | ContentKind::Emphasis {}
            | ContentKind::Strong {}
            | ContentKind::Strikethrough {}
            | ContentKind::Link { .. }
            | ContentKind::List { .. }
            | ContentKind::ListItem { .. }
            | ContentKind::BlockQuote { .. }
            | ContentKind::Table { .. }
            | ContentKind::TableHead {}
            | ContentKind::TableBody {}
            | ContentKind::TableRow {}
            | ContentKind::TableCell { .. }
            | ContentKind::FootnoteDefinition { .. }
            | ContentKind::CitationReference { .. }
            | ContentKind::Custom { opaque: false, .. }
    )
}

pub(crate) fn validate_child_kind(
    owner: Option<&ContentKind>,
    child: &ContentKind,
) -> Result<(), ProtocolError> {
    let valid = match owner {
        None => is_root_block(child),
        Some(
            ContentKind::Paragraph {}
            | ContentKind::Heading { .. }
            | ContentKind::Emphasis {}
            | ContentKind::Strong {}
            | ContentKind::Strikethrough {}
            | ContentKind::TableCell { .. },
        ) => is_phrasing(child),
        Some(ContentKind::Link { .. }) => {
            is_phrasing(child) && !matches!(child, ContentKind::Link { .. })
        }
        Some(ContentKind::CitationReference { .. }) => {
            is_phrasing(child)
                && !matches!(
                    child,
                    ContentKind::Link { .. } | ContentKind::CitationReference { .. }
                )
        }
        Some(ContentKind::List { .. }) => matches!(child, ContentKind::ListItem { .. }),
        Some(
            ContentKind::ListItem { .. }
            | ContentKind::BlockQuote { .. }
            | ContentKind::FootnoteDefinition { .. },
        ) => is_root_block(child),
        Some(ContentKind::Table { .. }) => {
            matches!(child, ContentKind::TableHead {} | ContentKind::TableBody {})
        }
        Some(ContentKind::TableHead {} | ContentKind::TableBody {}) => {
            matches!(child, ContentKind::TableRow {})
        }
        Some(ContentKind::TableRow {}) => matches!(child, ContentKind::TableCell { .. }),
        Some(ContentKind::Custom { opaque: false, .. }) => !is_table_internal(child),
        Some(_) => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::InvalidChange(
            "content kinds violate the canonical parent/child grammar".to_string(),
        ))
    }
}

pub(crate) struct ChildSequenceValidator {
    state: ChildSequenceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildSequenceCompleteness {
    Prefix,
    Complete,
}

enum ChildSequenceState {
    None,
    Table { head_seen: bool, body_seen: bool },
    TableHead { row_count: usize },
    TableRow { next_column: usize },
}

impl ChildSequenceValidator {
    pub(crate) fn new(owner: Option<&ContentKind>) -> Self {
        Self::resume(owner, 0, None).expect("an empty canonical child sequence always validates")
    }

    pub(crate) fn resume(
        owner: Option<&ContentKind>,
        prefix_len: usize,
        last_child: Option<&ContentKind>,
    ) -> Result<Self, ProtocolError> {
        let state = match owner {
            Some(ContentKind::Table { .. }) => match (prefix_len, last_child) {
                (0, None) => ChildSequenceState::Table {
                    head_seen: false,
                    body_seen: false,
                },
                (1, Some(ContentKind::TableHead {})) => ChildSequenceState::Table {
                    head_seen: true,
                    body_seen: false,
                },
                (2, Some(ContentKind::TableBody {})) => ChildSequenceState::Table {
                    head_seen: true,
                    body_seen: true,
                },
                _ => {
                    return Err(ProtocolError::InvalidChange(
                        "table sections must contain at most one head followed by at most one body"
                            .to_string(),
                    ));
                }
            },
            Some(ContentKind::TableHead {}) if prefix_len <= 1 => {
                if prefix_len == 1 && !matches!(last_child, Some(ContentKind::TableRow {})) {
                    return Err(ProtocolError::InvalidChange(
                        "table heads may contain exactly one row".to_string(),
                    ));
                }
                ChildSequenceState::TableHead {
                    row_count: prefix_len,
                }
            }
            Some(ContentKind::TableHead {}) => {
                return Err(ProtocolError::InvalidChange(
                    "table heads may contain exactly one row".to_string(),
                ));
            }
            Some(ContentKind::TableRow {}) => ChildSequenceState::TableRow {
                next_column: prefix_len,
            },
            _ => ChildSequenceState::None,
        };
        Ok(Self { state })
    }

    pub(crate) fn push(&mut self, child: &ContentKind) -> Result<(), ProtocolError> {
        match &mut self.state {
            ChildSequenceState::None => Ok(()),
            ChildSequenceState::Table {
                head_seen,
                body_seen,
            } => match child {
                ContentKind::TableHead {} if !*head_seen && !*body_seen => {
                    *head_seen = true;
                    Ok(())
                }
                ContentKind::TableBody {} if *head_seen && !*body_seen => {
                    *body_seen = true;
                    Ok(())
                }
                _ => Err(ProtocolError::InvalidChange(
                    "table sections must contain at most one head followed by at most one body"
                        .to_string(),
                )),
            },
            ChildSequenceState::TableHead { row_count } => {
                if !matches!(child, ContentKind::TableRow {}) || *row_count != 0 {
                    return Err(ProtocolError::InvalidChange(
                        "table heads may contain exactly one row".to_string(),
                    ));
                }
                *row_count = 1;
                Ok(())
            }
            ChildSequenceState::TableRow { next_column } => {
                let ContentKind::TableCell { column } = child else {
                    return Err(ProtocolError::InvalidChange(
                        "table rows may contain only table cells".to_string(),
                    ));
                };
                if usize::try_from(*column).ok() != Some(*next_column) {
                    return Err(ProtocolError::InvalidChange(
                        "table cell columns must be zero-based and contiguous".to_string(),
                    ));
                }
                *next_column = next_column.checked_add(1).ok_or_else(|| {
                    ProtocolError::InvalidChange("table column count overflow".to_string())
                })?;
                Ok(())
            }
        }
    }

    pub(crate) fn finish(
        &self,
        completeness: ChildSequenceCompleteness,
    ) -> Result<(), ProtocolError> {
        if completeness == ChildSequenceCompleteness::Prefix {
            return Ok(());
        }
        match self.state {
            ChildSequenceState::Table {
                head_seen: true,
                body_seen: true,
            }
            | ChildSequenceState::TableHead { row_count: 1 }
            | ChildSequenceState::TableRow { .. }
            | ChildSequenceState::None => Ok(()),
            ChildSequenceState::Table { .. } => Err(ProtocolError::InvalidChange(
                "stable tables require one head followed by one body".to_string(),
            )),
            ChildSequenceState::TableHead { .. } => Err(ProtocolError::InvalidChange(
                "stable table heads require exactly one row".to_string(),
            )),
        }
    }
}

pub(crate) fn validate_table_row_width(
    child_count: usize,
    columns: usize,
    completeness: ChildSequenceCompleteness,
) -> Result<(), ProtocolError> {
    let valid = match completeness {
        ChildSequenceCompleteness::Prefix => child_count <= columns,
        ChildSequenceCompleteness::Complete => child_count == columns,
    };
    if valid {
        Ok(())
    } else {
        Err(ProtocolError::InvalidChange(
            "table row width must match its table alignment schema".to_string(),
        ))
    }
}

fn is_table_internal(content: &ContentKind) -> bool {
    matches!(
        content,
        ContentKind::TableHead {}
            | ContentKind::TableBody {}
            | ContentKind::TableRow {}
            | ContentKind::TableCell { .. }
    )
}

fn is_phrasing(content: &ContentKind) -> bool {
    matches!(
        content,
        ContentKind::Text { .. }
            | ContentKind::Emphasis {}
            | ContentKind::Strong {}
            | ContentKind::Strikethrough {}
            | ContentKind::Link { .. }
            | ContentKind::Image { .. }
            | ContentKind::InlineCode { .. }
            | ContentKind::Math { .. }
            | ContentKind::FootnoteReference { .. }
            | ContentKind::CitationReference { .. }
            | ContentKind::SoftBreak {}
            | ContentKind::HardBreak {}
            | ContentKind::Html { block: false, .. }
            | ContentKind::Custom { opaque: false, .. }
    )
}

fn is_root_block(content: &ContentKind) -> bool {
    matches!(
        content,
        ContentKind::Paragraph {}
            | ContentKind::Heading { .. }
            | ContentKind::CodeBlock { .. }
            | ContentKind::List { .. }
            | ContentKind::BlockQuote { .. }
            | ContentKind::ThematicBreak {}
            | ContentKind::Table { .. }
            | ContentKind::Html { block: true, .. }
            | ContentKind::Math { display: true, .. }
            | ContentKind::FootnoteDefinition { .. }
            | ContentKind::CitationDefinition { .. }
            | ContentKind::Custom { .. }
    )
}

struct MetadataTally {
    bytes: usize,
    limits: ProtocolLimits,
}

impl MetadataTally {
    const fn new(limits: ProtocolLimits) -> Self {
        Self { bytes: 0, limits }
    }

    fn add(&mut self, field: &'static str, value: &str) -> Result<(), ProtocolError> {
        if value.len() > self.limits.max_metadata_value_bytes {
            return Err(ProtocolError::ValueTooLarge {
                field,
                limit: self.limits.max_metadata_value_bytes,
                actual: value.len(),
            });
        }
        self.bytes = self
            .bytes
            .checked_add(value.len())
            .ok_or(ProtocolError::MetadataOverflow)?;
        Ok(())
    }

    fn finish(self, field: &'static str) -> Result<usize, ProtocolError> {
        if self.bytes > self.limits.max_node_metadata_bytes {
            Err(ProtocolError::ValueTooLarge {
                field,
                limit: self.limits.max_node_metadata_bytes,
                actual: self.bytes,
            })
        } else {
            Ok(self.bytes)
        }
    }
}

fn require_nonempty(field: &'static str, value: &str) -> Result<(), ProtocolError> {
    if value.is_empty() {
        Err(ProtocolError::InvalidChange(format!(
            "{field} cannot be empty"
        )))
    } else {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Rust-side budgets for canonical IR validation and reduction.
///
/// This type is deliberately not a wire contract because `usize` has
/// platform-dependent width. Binding-specific option envelopes must use fixed
/// width or decimal-string fields and convert explicitly.
pub struct ProtocolLimits {
    pub max_source_bytes: usize,
    pub max_nodes: usize,
    pub max_resources: usize,
    pub max_operations: usize,
    pub max_change_structural_items: usize,
    pub max_document_structural_items: usize,
    pub max_children_per_list: usize,
    pub max_attributes_per_node: usize,
    pub max_metadata_value_bytes: usize,
    pub max_node_metadata_bytes: usize,
    pub max_change_metadata_bytes: usize,
    pub max_document_metadata_bytes: usize,
    pub max_tree_depth: usize,
}

impl Default for ProtocolLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 16 * 1024 * 1024,
            max_nodes: 100_000,
            max_resources: 100_000,
            max_operations: 10_000,
            max_change_structural_items: 100_000,
            max_document_structural_items: 1_000_000,
            max_children_per_list: 10_000,
            max_attributes_per_node: 256,
            max_metadata_value_bytes: 64 * 1024,
            max_node_metadata_bytes: 256 * 1024,
            max_change_metadata_bytes: 4 * 1024 * 1024,
            max_document_metadata_bytes: 16 * 1024 * 1024,
            max_tree_depth: 256,
        }
    }
}
