use mdstream_protocol::SourceRange;
use unicase::UniCase;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DefinitionNamespace {
    Reference,
    Footnote,
    Citation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct DefinitionKey {
    pub(super) namespace: DefinitionNamespace,
    pub(super) folded_label: String,
}

impl DefinitionKey {
    pub(crate) fn reference(label: &str) -> Self {
        Self::new(DefinitionNamespace::Reference, label)
    }

    pub(crate) fn footnote(label: &str) -> Self {
        Self::new(DefinitionNamespace::Footnote, label)
    }

    pub(crate) fn citation(key: &str) -> Self {
        Self::new(DefinitionNamespace::Citation, &format!("@{key}"))
    }

    pub(super) fn citation_key(&self) -> Option<&str> {
        (self.namespace == DefinitionNamespace::Citation)
            .then(|| self.folded_label.strip_prefix('@'))
            .flatten()
    }

    fn new(namespace: DefinitionNamespace, label: &str) -> Self {
        Self {
            namespace,
            folded_label: UniCase::new(label).to_folded_case(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DefinitionValue {
    Reference {
        destination: String,
        title: Option<String>,
    },
    Footnote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DefinitionFact {
    pub(crate) key: DefinitionKey,
    pub(crate) label: String,
    pub(crate) source: SourceRange,
    pub(crate) value: DefinitionValue,
}

impl DefinitionFact {
    pub(crate) fn reference(
        label: impl Into<String>,
        source: SourceRange,
        destination: impl Into<String>,
        title: Option<String>,
    ) -> Self {
        let label = label.into();
        Self {
            key: DefinitionKey::reference(&label),
            label,
            source,
            value: DefinitionValue::Reference {
                destination: destination.into(),
                title,
            },
        }
    }

    pub(crate) fn citation(
        label: impl Into<String>,
        source: SourceRange,
        destination: impl Into<String>,
        title: Option<String>,
    ) -> Option<Self> {
        let label = label.into();
        let key = citation_display_key(&label)?;
        Some(Self {
            key: DefinitionKey::citation(key),
            label,
            source,
            value: DefinitionValue::Reference {
                destination: destination.into(),
                title,
            },
        })
    }

    pub(crate) fn footnote(label: impl Into<String>, source: SourceRange) -> Self {
        let label = label.into();
        Self {
            key: DefinitionKey::footnote(&label),
            label,
            source,
            value: DefinitionValue::Footnote,
        }
    }

    pub(crate) fn citation_key(&self) -> Option<&str> {
        self.key.citation_key()
    }
}

pub(crate) fn citation_display_key(label: &str) -> Option<&str> {
    let key = label.strip_prefix('@')?.trim();
    (!key.is_empty()).then_some(key)
}
