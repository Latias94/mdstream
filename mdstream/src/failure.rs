/// Whether retrying a rejected append at the caller's original chunk boundaries
/// can change admission without changing the canonical document contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitSafety {
    /// The failed work was local to one atomic append transaction.
    RetryAtOriginalBoundaries,
    /// Splitting cannot make this failure admissible or would change semantics.
    NotSafe,
}

impl SplitSafety {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RetryAtOriginalBoundaries => "retry_at_original_boundaries",
            Self::NotSafe => "not_safe",
        }
    }
}

/// A typed resource limit scoped to one append transaction.
///
/// Unlike cumulative document, parser, and lifecycle limits, these limits can
/// become admissible when an already accepted joined append is replayed through
/// its original caller boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendLimitKind {
    ChangeOperations,
    ChangeStructuralItems,
    ChangeMetadataBytes,
    ChangeBytes,
    TransactionBytes,
}

impl AppendLimitKind {
    pub const fn field(self) -> &'static str {
        match self {
            Self::ChangeOperations => "change.operations",
            Self::ChangeStructuralItems => "change.structural_items",
            Self::ChangeMetadataBytes => "change.metadata",
            Self::ChangeBytes => "engine.change_bytes",
            Self::TransactionBytes => "engine.transaction_bytes",
        }
    }
}
