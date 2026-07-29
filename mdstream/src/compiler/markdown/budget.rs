use mdstream_protocol::ProtocolLimits;

use super::MarkdownError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DraftUsage {
    pub(crate) roots: usize,
    pub(crate) nodes: usize,
    pub(crate) resources: usize,
    pub(crate) structural_items: usize,
    pub(crate) metadata_bytes: usize,
}

pub(super) struct DraftBudget {
    limits: ProtocolLimits,
    baseline: DraftUsage,
    usage: DraftUsage,
}

impl DraftBudget {
    pub(super) const fn new(limits: ProtocolLimits, baseline: DraftUsage) -> Self {
        Self {
            limits,
            baseline,
            usage: baseline,
        }
    }

    pub(super) fn reserve_node(
        &mut self,
        content_structural_items: usize,
    ) -> Result<(), MarkdownError> {
        self.reserve_nodes(1, content_structural_items)
    }

    pub(super) fn reserve_synthetic_nodes(&mut self, count: usize) -> Result<(), MarkdownError> {
        self.reserve_nodes(count, 0)
    }

    pub(super) const fn usage(&self) -> DraftUsage {
        self.usage
    }

    pub(super) const fn baseline(&self) -> DraftUsage {
        self.baseline
    }

    pub(super) fn reserve_node_payload(
        &mut self,
        root: bool,
        metadata_bytes: usize,
    ) -> Result<(), MarkdownError> {
        let roots = self
            .usage
            .roots
            .checked_add(usize::from(root))
            .ok_or(MarkdownError::NumericOverflow("root count"))?;
        check_limit("roots", roots, self.limits.max_children_per_list)?;
        let metadata_bytes = self
            .usage
            .metadata_bytes
            .checked_add(metadata_bytes)
            .ok_or(MarkdownError::NumericOverflow("document metadata"))?;
        check_limit(
            "document.metadata",
            metadata_bytes,
            self.limits.max_document_metadata_bytes,
        )?;
        self.usage.roots = roots;
        self.usage.metadata_bytes = metadata_bytes;
        Ok(())
    }

    pub(super) fn reserve_resource(&mut self, metadata_bytes: usize) -> Result<(), MarkdownError> {
        let resources = self
            .usage
            .resources
            .checked_add(1)
            .ok_or(MarkdownError::NumericOverflow("resource count"))?;
        check_limit("resources", resources, self.limits.max_resources)?;
        let metadata_bytes = self
            .usage
            .metadata_bytes
            .checked_add(metadata_bytes)
            .ok_or(MarkdownError::NumericOverflow("document metadata"))?;
        check_limit(
            "document.metadata",
            metadata_bytes,
            self.limits.max_document_metadata_bytes,
        )?;
        self.usage.resources = resources;
        self.usage.metadata_bytes = metadata_bytes;
        Ok(())
    }

    fn reserve_nodes(
        &mut self,
        count: usize,
        content_structural_items: usize,
    ) -> Result<(), MarkdownError> {
        let nodes = self
            .usage
            .nodes
            .checked_add(count)
            .ok_or(MarkdownError::NumericOverflow("node count"))?;
        check_limit("nodes", nodes, self.limits.max_nodes)?;

        let structural_items = self
            .usage
            .structural_items
            .checked_add(count)
            .and_then(|items| items.checked_add(content_structural_items))
            .ok_or(MarkdownError::NumericOverflow("structural items"))?;
        check_limit(
            "document.structural_items",
            structural_items,
            self.limits.max_document_structural_items,
        )?;

        self.usage = DraftUsage {
            nodes,
            structural_items,
            ..self.usage
        };
        Ok(())
    }
}

fn check_limit(field: &'static str, actual: usize, limit: usize) -> Result<(), MarkdownError> {
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
