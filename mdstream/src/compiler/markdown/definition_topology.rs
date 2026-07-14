use crate::compiler::draft::{DraftContentKind, DraftNode};

use super::{MarkdownError, normalization::child_hull};

pub(super) fn merge_definition_nodes(
    nodes: &mut Vec<DraftNode>,
    definitions: Vec<DraftNode>,
    allow_siblings: bool,
) -> Result<usize, MarkdownError> {
    if definitions.is_empty() {
        return Ok(0);
    }

    let capacity = nodes.len().saturating_add(definitions.len());
    let existing = std::mem::take(nodes);
    let mut definitions = definitions.into_iter().peekable();
    let mut merged = Vec::with_capacity(capacity);
    let mut inserted = 0usize;

    for mut node in existing {
        while let Some(definition) =
            definitions.next_if(|definition| definition.source.start < node.source.start)
        {
            if !allow_siblings {
                return Err(definition_list_owner_error());
            }
            merged.push(definition);
            inserted = inserted
                .checked_add(1)
                .ok_or(MarkdownError::NumericOverflow("definition node count"))?;
        }

        let mut descendants = Vec::new();
        while let Some(definition) =
            definitions.next_if(|definition| node.source.contains(definition.source))
        {
            descendants.push(definition);
        }
        if !descendants.is_empty() {
            merge_definition_descendants(&mut node, descendants)?;
        }
        merged.push(node);
    }

    for definition in definitions {
        if !allow_siblings {
            return Err(definition_list_owner_error());
        }
        merged.push(definition);
        inserted = inserted
            .checked_add(1)
            .ok_or(MarkdownError::NumericOverflow("definition node count"))?;
    }
    *nodes = merged;
    Ok(inserted)
}

fn merge_definition_descendants(
    container: &mut DraftNode,
    definitions: Vec<DraftNode>,
) -> Result<(), MarkdownError> {
    let role =
        definition_container_role(&container.content).ok_or(MarkdownError::UnexpectedEvent {
            event: "reference-definition",
            context: "non-container source range",
        })?;
    merge_definition_nodes(
        &mut container.children,
        definitions,
        role == DefinitionContainerRole::TraverseAndOwn,
    )?;
    refresh_definition_container_body(container)
}

fn definition_list_owner_error() -> MarkdownError {
    MarkdownError::UnexpectedEvent {
        event: "reference-definition",
        context: "list without a containing item",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DefinitionContainerRole {
    TraverseOnly,
    TraverseAndOwn,
}

fn definition_container_role(content: &DraftContentKind) -> Option<DefinitionContainerRole> {
    match content {
        DraftContentKind::List { .. } => Some(DefinitionContainerRole::TraverseOnly),
        DraftContentKind::BlockQuote { .. }
        | DraftContentKind::ListItem { .. }
        | DraftContentKind::FootnoteDefinition { .. }
        | DraftContentKind::Custom { opaque: false, .. } => {
            Some(DefinitionContainerRole::TraverseAndOwn)
        }
        _ => None,
    }
}

fn refresh_definition_container_body(container: &mut DraftNode) -> Result<(), MarkdownError> {
    if matches!(
        container.content,
        DraftContentKind::List { .. }
            | DraftContentKind::BlockQuote { .. }
            | DraftContentKind::ListItem { .. }
            | DraftContentKind::FootnoteDefinition { .. }
    ) {
        container.body = child_hull(&container.children).ok_or(MarkdownError::UnexpectedEvent {
            event: "reference-definition",
            context: "container without children",
        })?;
    }
    Ok(())
}
