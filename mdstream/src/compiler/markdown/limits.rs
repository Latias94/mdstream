use mdstream_protocol::{ProtocolLimits, SemanticText};

use crate::compiler::draft::{DraftContentKind, DraftForest, DraftResource, DraftResourceRole};

use super::{MarkdownError, budget::DraftUsage};

pub(super) fn validate_draft_limits(
    forest: &DraftForest,
    limits: ProtocolLimits,
) -> Result<DraftUsage, MarkdownError> {
    check_count("roots", forest.roots.len(), limits.max_children_per_list)?;
    check_count("resources", forest.resources.len(), limits.max_resources)?;

    let mut node_count = 0usize;
    let mut structural_items = 0usize;
    let mut metadata_bytes = 0usize;
    let mut pending = forest
        .roots
        .iter()
        .map(|node| (node, 1usize))
        .collect::<Vec<_>>();
    while let Some((node, depth)) = pending.pop() {
        node_count = node_count
            .checked_add(1)
            .ok_or(MarkdownError::NumericOverflow("node count"))?;
        check_count("nodes", node_count, limits.max_nodes)?;
        check_count("tree.depth", depth, limits.max_tree_depth)?;
        check_count(
            "children",
            node.children.len(),
            limits.max_children_per_list,
        )?;
        let content_items = match &node.content {
            DraftContentKind::Table { alignments } => alignments.len(),
            _ => 0,
        };
        structural_items = structural_items
            .checked_add(1)
            .and_then(|items| items.checked_add(content_items))
            .ok_or(MarkdownError::NumericOverflow("structural items"))?;
        check_count(
            "document.structural_items",
            structural_items,
            limits.max_document_structural_items,
        )?;

        let node_metadata = draft_node_metadata(&node.content, limits)?;
        metadata_bytes = metadata_bytes
            .checked_add(node_metadata)
            .ok_or(MarkdownError::NumericOverflow("document metadata"))?;
        check_count(
            "document.metadata",
            metadata_bytes,
            limits.max_document_metadata_bytes,
        )?;

        let child_depth = depth
            .checked_add(1)
            .ok_or(MarkdownError::NumericOverflow("tree depth"))?;
        pending.extend(node.children.iter().map(|child| (child, child_depth)));
    }

    for resource in &forest.resources {
        let resource_metadata = draft_resource_metadata(resource, limits)?;
        metadata_bytes = metadata_bytes
            .checked_add(resource_metadata)
            .ok_or(MarkdownError::NumericOverflow("document metadata"))?;
        check_count(
            "document.metadata",
            metadata_bytes,
            limits.max_document_metadata_bytes,
        )?;
    }
    Ok(DraftUsage {
        roots: forest.roots.len(),
        nodes: node_count,
        resources: forest.resources.len(),
        structural_items,
        metadata_bytes,
    })
}

pub(super) fn draft_node_metadata(
    content: &DraftContentKind,
    limits: ProtocolLimits,
) -> Result<usize, MarkdownError> {
    let mut bytes = 0usize;
    match content {
        DraftContentKind::Text { text }
        | DraftContentKind::InlineCode { text }
        | DraftContentKind::Math { text, .. }
        | DraftContentKind::Html { text, .. } => {
            add_semantic_text(&mut bytes, text, limits)?;
        }
        DraftContentKind::CodeBlock { info, text, .. } => {
            if let Some(info) = info {
                add_metadata(&mut bytes, "code.info", info, limits)?;
            }
            add_semantic_text(&mut bytes, text, limits)?;
        }
        DraftContentKind::Link {
            reference_label: Some(label),
            ..
        } => add_metadata(&mut bytes, "reference.label", label, limits)?,
        DraftContentKind::Link {
            reference_label: None,
            ..
        } => {}
        DraftContentKind::Image {
            reference_label,
            alt,
            ..
        } => {
            if let Some(label) = reference_label {
                add_metadata(&mut bytes, "reference.label", label, limits)?;
            }
            add_semantic_text(&mut bytes, alt, limits)?;
        }
        DraftContentKind::FootnoteDefinition { label }
        | DraftContentKind::FootnoteReference { label } => {
            add_metadata(&mut bytes, "footnote.label", label, limits)?;
        }
        DraftContentKind::CitationReference { key, .. } => {
            add_metadata(&mut bytes, "citation.key", key, limits)?;
        }
        DraftContentKind::Custom {
            namespace,
            name,
            attributes,
            ..
        } => {
            check_count(
                "custom.attributes",
                attributes.len(),
                limits.max_attributes_per_node,
            )?;
            add_metadata(&mut bytes, "custom.namespace", namespace, limits)?;
            add_metadata(&mut bytes, "custom.name", name, limits)?;
            for (key, value) in attributes {
                add_metadata(&mut bytes, "custom.attribute.key", key, limits)?;
                add_metadata(&mut bytes, "custom.attribute.value", value, limits)?;
            }
        }
        DraftContentKind::Table { alignments } => {
            check_count(
                "table.alignments",
                alignments.len(),
                limits.max_children_per_list,
            )?;
        }
        _ => {}
    }
    check_count("node.metadata", bytes, limits.max_node_metadata_bytes)?;
    Ok(bytes)
}

pub(super) fn draft_resource_metadata(
    resource: &DraftResource,
    limits: ProtocolLimits,
) -> Result<usize, MarkdownError> {
    let mut bytes = 0usize;
    if resource.key.role == DraftResourceRole::Citation {
        if let Some(label) = &resource.key.reference_label {
            add_metadata(
                &mut bytes,
                "resource.citation.key",
                label.trim_start_matches('@'),
                limits,
            )?;
        }
    }
    add_metadata(
        &mut bytes,
        "resource.destination",
        &resource.destination,
        limits,
    )?;
    if let Some(title) = &resource.title {
        add_metadata(&mut bytes, "resource.title", title, limits)?;
    }
    check_count("resource.metadata", bytes, limits.max_node_metadata_bytes)?;
    Ok(bytes)
}

fn add_semantic_text(
    bytes: &mut usize,
    text: &SemanticText,
    limits: ProtocolLimits,
) -> Result<(), MarkdownError> {
    if let SemanticText::Normalized { value } = text {
        add_metadata(bytes, "semantic_text.value", value, limits)?;
    }
    Ok(())
}

fn add_metadata(
    bytes: &mut usize,
    field: &'static str,
    value: &str,
    limits: ProtocolLimits,
) -> Result<(), MarkdownError> {
    check_count(field, value.len(), limits.max_metadata_value_bytes)?;
    *bytes = bytes
        .checked_add(value.len())
        .ok_or(MarkdownError::NumericOverflow("metadata bytes"))?;
    Ok(())
}

fn check_count(field: &'static str, actual: usize, limit: usize) -> Result<(), MarkdownError> {
    if actual > limit {
        Err(MarkdownError::LimitExceeded {
            field,
            limit,
            actual,
        })
    } else {
        Ok(())
    }
}
