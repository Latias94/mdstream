use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use mdstream_protocol::{
    ContentKind, Epoch, NodeId, NodeProjection, NodeStability, ResourceId, ResourceRef,
    SemanticResource, SemanticResourceKind,
};

use super::{
    DraftContentKind, DraftForest, DraftNode, DraftOriginHint, DraftResource, DraftResourceIndex,
    DraftResourceRole, SyntheticRole,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MaterializedNode {
    pub(crate) projection: NodeProjection,
    pub(crate) children: Vec<NodeId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MaterializedForest {
    pub(crate) roots: Vec<NodeId>,
    pub(crate) nodes: BTreeMap<NodeId, MaterializedNode>,
    pub(crate) resources: BTreeMap<ResourceId, SemanticResource>,
    pub(crate) resource_refs: Vec<ResourceRef>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IdentityLedger {
    node_origins: BTreeMap<NodeId, Box<[u8]>>,
    resource_origins: BTreeMap<ResourceId, Box<[u8]>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct IdentityCommit {
    node_origins: BTreeMap<NodeId, Box<[u8]>>,
    resource_origins: BTreeMap<ResourceId, Box<[u8]>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IdentityError {
    NodeCollision(NodeId),
    ResourceCollision(ResourceId),
    DuplicateLiveNode(NodeId),
    ResourceConflict(ResourceId),
    MissingResource(DraftResourceIndex),
    MissingRequiredResource,
    NumericOverflow(&'static str),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NodeCollision(id) => {
                write!(formatter, "node identity {id} maps to distinct origins")
            }
            Self::ResourceCollision(id) => {
                write!(formatter, "resource identity {id} maps to distinct origins")
            }
            Self::DuplicateLiveNode(id) => {
                write!(
                    formatter,
                    "node origin {id} appears twice in one draft forest"
                )
            }
            Self::ResourceConflict(id) => {
                write!(
                    formatter,
                    "resource origin {id} has conflicting semantic values"
                )
            }
            Self::MissingResource(index) => {
                write!(formatter, "draft resource {} does not exist", index.get())
            }
            Self::MissingRequiredResource => {
                formatter.write_str("definition node has no semantic resource")
            }
            Self::NumericOverflow(field) => {
                write!(formatter, "{field} exceeds the identity domain")
            }
        }
    }
}

impl std::error::Error for IdentityError {}

impl IdentityLedger {
    pub(crate) fn stage(
        &self,
        epoch: Epoch,
        draft: &DraftForest,
        stable_root_count: usize,
    ) -> Result<(MaterializedForest, IdentityCommit), IdentityError> {
        if stable_root_count > draft.roots.len() {
            return Err(IdentityError::NumericOverflow("stable root count"));
        }

        let mut stage = StagedIdentities::new(self);
        let (resources, resource_refs) = stage.materialize_resources(epoch, &draft.resources)?;
        let mut forest = MaterializedForest {
            roots: Vec::with_capacity(draft.roots.len()),
            nodes: BTreeMap::new(),
            resources,
            resource_refs: resource_refs.clone(),
        };
        let mut occurrences = BTreeMap::new();

        for (index, root) in draft.roots.iter().enumerate() {
            let occurrence = next_occurrence(&mut occurrences, root)?;
            let id = stage.node_id(epoch, None, root, occurrence)?;
            let stability = if index < stable_root_count {
                NodeStability::Stable
            } else {
                NodeStability::Provisional
            };
            materialize_node(
                &mut stage,
                epoch,
                id,
                root,
                stability,
                &resource_refs,
                &mut forest.nodes,
            )?;
            forest.roots.push(id);
        }

        Ok((forest, stage.finish()))
    }

    pub(crate) fn commit(&mut self, commit: IdentityCommit) {
        self.node_origins.extend(commit.node_origins);
        self.resource_origins.extend(commit.resource_origins);
    }
}

struct StagedIdentities<'ledger> {
    ledger: &'ledger IdentityLedger,
    new_nodes: BTreeMap<NodeId, Box<[u8]>>,
    new_resources: BTreeMap<ResourceId, Box<[u8]>>,
    live_nodes: BTreeSet<NodeId>,
}

impl<'ledger> StagedIdentities<'ledger> {
    fn new(ledger: &'ledger IdentityLedger) -> Self {
        Self {
            ledger,
            new_nodes: BTreeMap::new(),
            new_resources: BTreeMap::new(),
            live_nodes: BTreeSet::new(),
        }
    }

    fn materialize_resources(
        &mut self,
        epoch: Epoch,
        drafts: &[DraftResource],
    ) -> Result<(BTreeMap<ResourceId, SemanticResource>, Vec<ResourceRef>), IdentityError> {
        let mut resources = BTreeMap::new();
        let mut ids = Vec::with_capacity(drafts.len());
        for draft in drafts {
            let origin = resource_origin(epoch, draft)?;
            let id = ResourceId::digest(&origin);
            self.check_resource_origin(id, &origin)?;
            let resource_kind = match draft.key.role {
                DraftResourceRole::Link | DraftResourceRole::Image => SemanticResourceKind::Link {
                    destination: draft.destination.clone(),
                    title: draft.title.clone(),
                },
                DraftResourceRole::Footnote => SemanticResourceKind::Footnote {
                    label: draft.key.reference_label.clone().unwrap_or_default(),
                },
                DraftResourceRole::Citation => SemanticResourceKind::Citation {
                    protocol: mdstream_protocol::CitationProtocol::V1,
                    key: unicase::UniCase::new(
                        draft
                            .key
                            .reference_label
                            .as_deref()
                            .unwrap_or_default()
                            .trim_start_matches('@'),
                    )
                    .to_folded_case(),
                    destination: draft.destination.clone(),
                    title: draft.title.clone(),
                },
            };
            let resource = SemanticResource::new(id, resource_kind);
            if resources
                .insert(id, resource.clone())
                .is_some_and(|existing| existing != resource)
            {
                return Err(IdentityError::ResourceConflict(id));
            }
            ids.push(resource.reference());
        }
        Ok((resources, ids))
    }

    fn node_id(
        &mut self,
        epoch: Epoch,
        parent: Option<NodeId>,
        draft: &DraftNode,
        occurrence: u64,
    ) -> Result<NodeId, IdentityError> {
        let origin = node_origin(epoch, parent, draft, occurrence);
        let id = NodeId::digest(&origin);
        self.check_node_origin(id, &origin)?;
        if !self.live_nodes.insert(id) {
            return Err(IdentityError::DuplicateLiveNode(id));
        }
        Ok(id)
    }

    fn check_node_origin(&mut self, id: NodeId, origin: &[u8]) -> Result<(), IdentityError> {
        if self
            .ledger
            .node_origins
            .get(&id)
            .or_else(|| self.new_nodes.get(&id))
            .is_some_and(|known| known.as_ref() != origin)
        {
            return Err(IdentityError::NodeCollision(id));
        }
        if !self.ledger.node_origins.contains_key(&id) {
            self.new_nodes.insert(id, origin.into());
        }
        Ok(())
    }

    fn check_resource_origin(
        &mut self,
        id: ResourceId,
        origin: &[u8],
    ) -> Result<(), IdentityError> {
        if self
            .ledger
            .resource_origins
            .get(&id)
            .or_else(|| self.new_resources.get(&id))
            .is_some_and(|known| known.as_ref() != origin)
        {
            return Err(IdentityError::ResourceCollision(id));
        }
        if !self.ledger.resource_origins.contains_key(&id) {
            self.new_resources.insert(id, origin.into());
        }
        Ok(())
    }

    fn finish(self) -> IdentityCommit {
        IdentityCommit {
            node_origins: self.new_nodes,
            resource_origins: self.new_resources,
        }
    }
}

fn materialize_node(
    stage: &mut StagedIdentities<'_>,
    epoch: Epoch,
    root_id: NodeId,
    root: &DraftNode,
    stability: NodeStability,
    resource_refs: &[ResourceRef],
    output: &mut BTreeMap<NodeId, MaterializedNode>,
) -> Result<(), IdentityError> {
    let mut pending = vec![(root_id, root)];
    while let Some((id, draft)) = pending.pop() {
        let mut children = Vec::with_capacity(draft.children.len());
        let mut occurrences = BTreeMap::new();
        for child in &draft.children {
            let occurrence = next_occurrence(&mut occurrences, child)?;
            children.push(stage.node_id(epoch, Some(id), child, occurrence)?);
        }
        let child_tasks = children
            .iter()
            .copied()
            .zip(&draft.children)
            .collect::<Vec<_>>();

        let content = materialize_content(&draft.content, resource_refs)?;
        let node = MaterializedNode {
            projection: NodeProjection::new(stability, draft.source, draft.body, content),
            children,
        };
        if output.insert(id, node).is_some() {
            return Err(IdentityError::DuplicateLiveNode(id));
        }
        pending.extend(child_tasks.into_iter().rev());
    }
    Ok(())
}

fn materialize_content(
    draft: &DraftContentKind,
    resource_refs: &[ResourceRef],
) -> Result<ContentKind, IdentityError> {
    let content = match draft {
        DraftContentKind::Paragraph => ContentKind::Paragraph {},
        DraftContentKind::Heading { level } => ContentKind::Heading { level: *level },
        DraftContentKind::Text { text } => ContentKind::Text { text: text.clone() },
        DraftContentKind::Emphasis => ContentKind::Emphasis {},
        DraftContentKind::Strong => ContentKind::Strong {},
        DraftContentKind::Strikethrough => ContentKind::Strikethrough {},
        DraftContentKind::Link {
            target,
            reference_label,
            style,
        } => ContentKind::Link {
            target: resource_reference(*target, resource_refs)?,
            reference_label: reference_label.clone(),
            style: *style,
        },
        DraftContentKind::CitationReference { key, target } => ContentKind::CitationReference {
            key: key.clone(),
            target: resource_reference(*target, resource_refs)?,
        },
        DraftContentKind::Image {
            target,
            reference_label,
            style,
            alt,
        } => ContentKind::Image {
            target: resource_reference(*target, resource_refs)?,
            reference_label: reference_label.clone(),
            style: *style,
            alt: alt.clone(),
        },
        DraftContentKind::InlineCode { text } => ContentKind::InlineCode { text: text.clone() },
        DraftContentKind::CodeBlock { syntax, info, text } => ContentKind::CodeBlock {
            syntax: *syntax,
            info: info.clone(),
            text: text.clone(),
        },
        DraftContentKind::List {
            ordered,
            start,
            tight,
        } => ContentKind::List {
            ordered: *ordered,
            start: *start,
            tight: *tight,
        },
        DraftContentKind::ListItem { checked } => ContentKind::ListItem { checked: *checked },
        DraftContentKind::BlockQuote { style } => ContentKind::BlockQuote { style: *style },
        DraftContentKind::ThematicBreak => ContentKind::ThematicBreak {},
        DraftContentKind::Table { alignments } => ContentKind::Table {
            alignments: alignments.clone(),
        },
        DraftContentKind::TableHead => ContentKind::TableHead {},
        DraftContentKind::TableBody => ContentKind::TableBody {},
        DraftContentKind::TableRow => ContentKind::TableRow {},
        DraftContentKind::TableCell { column } => ContentKind::TableCell { column: *column },
        DraftContentKind::Html { block, text } => ContentKind::Html {
            block: *block,
            text: text.clone(),
        },
        DraftContentKind::Custom {
            namespace,
            name,
            opaque,
            attributes,
        } => ContentKind::Custom {
            namespace: namespace.clone(),
            name: name.clone(),
            opaque: *opaque,
            attributes: attributes.clone(),
        },
        DraftContentKind::Math { display, text } => ContentKind::Math {
            display: *display,
            text: text.clone(),
        },
        DraftContentKind::FootnoteDefinition { label, target } => ContentKind::FootnoteDefinition {
            label: label.clone(),
            target: required_resource_reference(*target, resource_refs)?,
        },
        DraftContentKind::FootnoteReference { label, target } => ContentKind::FootnoteReference {
            label: label.clone(),
            target: resource_reference(*target, resource_refs)?,
        },
        DraftContentKind::CitationDefinition { key, target } => ContentKind::CitationDefinition {
            key: key.clone(),
            target: required_resource_reference(*target, resource_refs)?,
        },
        DraftContentKind::SoftBreak => ContentKind::SoftBreak {},
        DraftContentKind::HardBreak => ContentKind::HardBreak {},
    };
    Ok(content)
}

fn resource_reference(
    index: Option<DraftResourceIndex>,
    resource_refs: &[ResourceRef],
) -> Result<Option<ResourceRef>, IdentityError> {
    index
        .map(|index| {
            resource_refs
                .get(index.get())
                .cloned()
                .ok_or(IdentityError::MissingResource(index))
        })
        .transpose()
}

fn required_resource_reference(
    index: Option<DraftResourceIndex>,
    resource_refs: &[ResourceRef],
) -> Result<ResourceRef, IdentityError> {
    resource_reference(index, resource_refs)?.ok_or(IdentityError::MissingRequiredResource)
}

fn next_occurrence(
    occurrences: &mut BTreeMap<(u64, u8), u64>,
    node: &DraftNode,
) -> Result<u64, IdentityError> {
    let key = (node.source.start.get(), origin_tag(node.origin));
    let occurrence = occurrences.entry(key).or_default();
    let current = *occurrence;
    *occurrence = occurrence
        .checked_add(1)
        .ok_or(IdentityError::NumericOverflow("node occurrence"))?;
    Ok(current)
}

fn node_origin(epoch: Epoch, parent: Option<NodeId>, node: &DraftNode, occurrence: u64) -> Vec<u8> {
    let mut origin = Vec::with_capacity(64);
    origin.extend_from_slice(b"mdstream.node-origin/1\0");
    origin.extend_from_slice(&epoch.get().to_be_bytes());
    match parent {
        Some(parent) => {
            origin.push(1);
            origin.extend_from_slice(&parent.get().to_be_bytes());
        }
        None => {
            origin.push(0);
            origin.extend_from_slice(&0_u128.to_be_bytes());
        }
    }
    origin.extend_from_slice(&node.source.start.get().to_be_bytes());
    origin.push(origin_tag(node.origin));
    origin.extend_from_slice(&occurrence.to_be_bytes());
    origin
}

fn resource_origin(epoch: Epoch, resource: &DraftResource) -> Result<Vec<u8>, IdentityError> {
    let mut origin = Vec::with_capacity(64);
    origin.extend_from_slice(b"mdstream.resource-origin/2\0");
    origin.extend_from_slice(&epoch.get().to_be_bytes());
    origin.push(match resource.key.role {
        DraftResourceRole::Link => 0,
        DraftResourceRole::Image => 1,
        DraftResourceRole::Footnote => 2,
        DraftResourceRole::Citation => 3,
    });
    origin.extend_from_slice(&resource.key.source.start.get().to_be_bytes());
    Ok(origin)
}

const fn origin_tag(origin: DraftOriginHint) -> u8 {
    match origin {
        DraftOriginHint::Parsed | DraftOriginHint::Synthetic(SyntheticRole::TightParagraph) => 0,
        DraftOriginHint::Synthetic(SyntheticRole::TableHeaderRow) => 1,
        DraftOriginHint::Synthetic(SyntheticRole::TableBody) => 2,
    }
}

#[cfg(test)]
mod tests {
    use mdstream_protocol::{Epoch, SemanticText, SourceCursor, SourceRange};

    use super::*;
    use crate::compiler::draft::{DraftResourceKey, DraftResourceRole};

    fn range(start: u64, end: u64) -> SourceRange {
        SourceRange::new(SourceCursor::new(start), SourceCursor::new(end))
    }

    fn text_forest(content: DraftContentKind) -> DraftForest {
        DraftForest {
            roots: vec![DraftNode::leaf(range(0, 1), range(0, 1), content)],
            resources: Vec::new(),
            pending_custom_start: None,
        }
    }

    #[test]
    fn staged_origins_are_not_visible_until_commit() {
        let draft = text_forest(DraftContentKind::Text {
            text: SemanticText::Source {},
        });
        let mut ledger = IdentityLedger::default();

        let (first, commit) = ledger.stage(Epoch::new(7), &draft, 0).unwrap();
        assert!(ledger.node_origins.is_empty());
        ledger.commit(commit);
        assert_eq!(ledger.node_origins.len(), 1);

        let reclassified = text_forest(DraftContentKind::Paragraph);
        let (second, _) = ledger.stage(Epoch::new(7), &reclassified, 0).unwrap();
        assert_eq!(first.roots, second.roots);
    }

    #[test]
    fn node_digest_collision_is_rejected_without_mutating_the_ledger() {
        let draft = text_forest(DraftContentKind::Text {
            text: SemanticText::Source {},
        });
        let epoch = Epoch::new(9);
        let origin = node_origin(epoch, None, &draft.roots[0], 0);
        let id = NodeId::digest(&origin);
        let mut ledger = IdentityLedger::default();
        ledger
            .node_origins
            .insert(id, Box::from(&b"other-origin"[..]));
        let before = ledger.node_origins.clone();

        assert!(matches!(
            ledger.stage(epoch, &draft, 0),
            Err(IdentityError::NodeCollision(collision)) if collision == id
        ));
        assert_eq!(ledger.node_origins, before);
    }

    #[test]
    fn resource_digest_collision_is_rejected_without_mutating_the_ledger() {
        let resource = DraftResource {
            key: DraftResourceKey {
                role: DraftResourceRole::Link,
                source: range(0, 8),
                reference_label: Some("docs".to_string()),
            },
            destination: "https://example.test".to_string(),
            title: None,
        };
        let epoch = Epoch::new(11);
        let origin = resource_origin(epoch, &resource).unwrap();
        let id = ResourceId::digest(&origin);
        let mut ledger = IdentityLedger::default();
        ledger
            .resource_origins
            .insert(id, Box::from(&b"other-resource-origin"[..]));
        let before = ledger.resource_origins.clone();
        let draft = DraftForest {
            roots: Vec::new(),
            resources: vec![resource],
            pending_custom_start: None,
        };

        assert!(matches!(
            ledger.stage(epoch, &draft, 0),
            Err(IdentityError::ResourceCollision(collision)) if collision == id
        ));
        assert_eq!(ledger.resource_origins, before);
    }
}
