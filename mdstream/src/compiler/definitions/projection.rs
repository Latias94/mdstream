use std::collections::BTreeMap;

use mdstream_protocol::{
    ChangePayloadCost, ContentKind, LinkStyle, NodeProjection, ProjectionOp, ProtocolLimits,
    ResourceRef,
};

use crate::compiler::draft::{
    DraftContentKind, DraftForest, DraftNode, DraftResource, DraftResourceIndex, DraftResourceKey,
    DraftResourceRole,
};
use crate::compiler::types::CompilerError;

use super::model::{DefinitionFact, DefinitionKey, DefinitionNamespace, DefinitionValue};
use super::state::DefinitionView;

#[derive(Debug, Clone)]
pub(crate) struct SemanticCorrection {
    pub(crate) cost: ChangePayloadCost,
    pub(crate) operation: ProjectionOp,
}

pub(super) fn dependency_key(content: &ContentKind) -> Option<DefinitionKey> {
    match content {
        ContentKind::Link {
            reference_label: Some(label),
            ..
        }
        | ContentKind::Image {
            reference_label: Some(label),
            ..
        } => Some(DefinitionKey::reference(label)),
        ContentKind::FootnoteDefinition { label, .. }
        | ContentKind::FootnoteReference { label, .. } => Some(DefinitionKey::footnote(label)),
        ContentKind::CitationDefinition { key, .. }
        | ContentKind::CitationReference { key, .. } => Some(DefinitionKey::citation(key)),
        _ => None,
    }
}

pub(super) fn corrected_projection(
    mut projection: NodeProjection,
    key: &DefinitionKey,
    definition: Option<&DefinitionFact>,
    target: Option<&ResourceRef>,
) -> Result<Option<NodeProjection>, CompilerError> {
    let current_target = projection.content.resource_ref().cloned();
    if current_target.as_ref().map(|current| current.id) == target.map(|target| target.id) {
        return Ok(None);
    }
    match &mut projection.content {
        ContentKind::Link {
            target: current,
            style,
            ..
        }
        | ContentKind::Image {
            target: current,
            style,
            ..
        } => {
            *current = target.cloned();
            *style = resolved_style(*style, current.is_some());
        }
        ContentKind::CitationReference {
            key: current_key,
            target: current,
        } => {
            if let Some(definition) = definition {
                *current_key = definition
                    .citation_key()
                    .ok_or_else(|| {
                        CompilerError::InvalidIdentity(
                            "citation definition has no citation key".to_string(),
                        )
                    })?
                    .to_string();
            }
            *current = target.cloned();
        }
        ContentKind::FootnoteReference {
            label,
            target: current,
        } => {
            if let Some(definition) = definition {
                *label = definition.label.clone();
            }
            *current = target.cloned();
        }
        ContentKind::FootnoteDefinition { .. } | ContentKind::CitationDefinition { .. } => {
            return Err(CompilerError::InvalidReconciliation(
                "a stable definition cannot lose its semantic resource".to_string(),
            ));
        }
        _ => {
            return Err(CompilerError::InvalidReconciliation(format!(
                "node indexed under {key:?} is not definition-dependent"
            )));
        }
    }
    projection.version = projection.derived_version();
    Ok(Some(projection))
}

pub(super) fn collect_footnote_definitions(nodes: &[DraftNode], output: &mut Vec<DefinitionFact>) {
    for node in nodes {
        if let DraftContentKind::FootnoteDefinition { label, .. } = &node.content {
            output.push(DefinitionFact::footnote(label.clone(), node.source));
        }
        collect_footnote_definitions(&node.children, output);
    }
}

pub(super) fn enrich_forest(
    forest: &mut DraftForest,
    definitions: &DefinitionView<'_, '_>,
    limits: ProtocolLimits,
) -> Result<BTreeMap<DefinitionKey, DraftResourceIndex>, CompilerError> {
    let previous_resources = std::mem::take(&mut forest.resources);
    let mut resources = Vec::with_capacity(previous_resources.len());
    let mut inline_remap = BTreeMap::new();
    for (old_index, resource) in previous_resources.into_iter().enumerate() {
        if resource.key.reference_label.is_some() {
            continue;
        }
        let next = DraftResourceIndex::new(resources.len());
        inline_remap.insert(DraftResourceIndex::new(old_index), next);
        resources.push(resource);
    }

    let mut resource_indices = BTreeMap::new();
    for node in &mut forest.roots {
        enrich_node(
            node,
            definitions,
            &inline_remap,
            &mut resources,
            &mut resource_indices,
            limits,
        )?;
    }
    forest.resources = resources;
    Ok(resource_indices)
}

fn enrich_node(
    node: &mut DraftNode,
    definitions: &DefinitionView<'_, '_>,
    inline_remap: &BTreeMap<DraftResourceIndex, DraftResourceIndex>,
    resources: &mut Vec<DraftResource>,
    resource_indices: &mut BTreeMap<DefinitionKey, DraftResourceIndex>,
    limits: ProtocolLimits,
) -> Result<(), CompilerError> {
    match &mut node.content {
        DraftContentKind::Link {
            target,
            reference_label,
            style,
        } => {
            if let Some(label) = reference_label {
                let key = DefinitionKey::reference(label);
                *target =
                    definition_target(&key, definitions, resources, resource_indices, limits)?;
                *style = resolved_style(*style, target.is_some());
            } else {
                *target = remap_inline_target(*target, inline_remap)?;
            }
        }
        DraftContentKind::Image {
            target,
            reference_label,
            style,
            ..
        } => {
            if let Some(label) = reference_label {
                let key = DefinitionKey::reference(label);
                *target =
                    definition_target(&key, definitions, resources, resource_indices, limits)?;
                *style = resolved_style(*style, target.is_some());
            } else {
                *target = remap_inline_target(*target, inline_remap)?;
            }
        }
        DraftContentKind::CitationDefinition { key, target }
        | DraftContentKind::CitationReference { key, target } => {
            let definition_key = DefinitionKey::citation(key);
            if let Some(definition) = definitions.get(&definition_key) {
                if let Some(canonical) = definition.citation_key() {
                    *key = canonical.to_string();
                }
            }
            *target = definition_target(
                &definition_key,
                definitions,
                resources,
                resource_indices,
                limits,
            )?;
        }
        DraftContentKind::FootnoteDefinition { label, target }
        | DraftContentKind::FootnoteReference { label, target } => {
            let definition_key = DefinitionKey::footnote(label);
            if let Some(definition) = definitions.get(&definition_key) {
                *label = definition.label.clone();
            }
            *target = definition_target(
                &definition_key,
                definitions,
                resources,
                resource_indices,
                limits,
            )?;
        }
        _ => {}
    }

    for child in &mut node.children {
        enrich_node(
            child,
            definitions,
            inline_remap,
            resources,
            resource_indices,
            limits,
        )?;
    }
    Ok(())
}

fn remap_inline_target(
    target: Option<DraftResourceIndex>,
    remap: &BTreeMap<DraftResourceIndex, DraftResourceIndex>,
) -> Result<Option<DraftResourceIndex>, CompilerError> {
    target
        .map(|target| {
            remap.get(&target).copied().ok_or_else(|| {
                CompilerError::InvalidIdentity(
                    "inline resource disappeared during semantic enrichment".to_string(),
                )
            })
        })
        .transpose()
}

pub(super) fn definition_target(
    key: &DefinitionKey,
    definitions: &DefinitionView<'_, '_>,
    resources: &mut Vec<DraftResource>,
    resource_indices: &mut BTreeMap<DefinitionKey, DraftResourceIndex>,
    limits: ProtocolLimits,
) -> Result<Option<DraftResourceIndex>, CompilerError> {
    let Some(definition) = definitions.get(key) else {
        return Ok(None);
    };
    if let Some(index) = resource_indices.get(key) {
        return Ok(Some(*index));
    }
    if resources.len() >= limits.max_resources {
        return Err(CompilerError::LimitExceeded {
            field: "resources",
            limit: limits.max_resources,
            actual: resources.len().saturating_add(1),
        });
    }
    let (role, destination, title) = match (&definition.key.namespace, &definition.value) {
        (DefinitionNamespace::Reference, DefinitionValue::Reference { destination, title }) => {
            (DraftResourceRole::Link, destination.clone(), title.clone())
        }
        (DefinitionNamespace::Citation, DefinitionValue::Reference { destination, title }) => (
            DraftResourceRole::Citation,
            destination.clone(),
            title.clone(),
        ),
        (DefinitionNamespace::Footnote, DefinitionValue::Footnote) => {
            (DraftResourceRole::Footnote, String::new(), None)
        }
        _ => {
            return Err(CompilerError::InvalidIdentity(
                "definition namespace and payload disagree".to_string(),
            ));
        }
    };
    let index = DraftResourceIndex::new(resources.len());
    resources.push(DraftResource {
        key: DraftResourceKey {
            role,
            source: definition.source,
            reference_label: Some(definition.label.clone()),
        },
        destination,
        title,
    });
    resource_indices.insert(key.clone(), index);
    Ok(Some(index))
}

const fn resolved_style(style: LinkStyle, resolved: bool) -> LinkStyle {
    match (style, resolved) {
        (LinkStyle::Reference | LinkStyle::ReferenceUnknown, true) => LinkStyle::Reference,
        (LinkStyle::Reference | LinkStyle::ReferenceUnknown, false) => LinkStyle::ReferenceUnknown,
        (LinkStyle::Collapsed | LinkStyle::CollapsedUnknown, true) => LinkStyle::Collapsed,
        (LinkStyle::Collapsed | LinkStyle::CollapsedUnknown, false) => LinkStyle::CollapsedUnknown,
        (LinkStyle::Shortcut | LinkStyle::ShortcutUnknown, true) => LinkStyle::Shortcut,
        (LinkStyle::Shortcut | LinkStyle::ShortcutUnknown, false) => LinkStyle::ShortcutUnknown,
        (style, _) => style,
    }
}
