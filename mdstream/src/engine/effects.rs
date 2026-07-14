use mdstream_protocol::ChangeSet;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineOutput {
    changes: Vec<ChangeSet>,
}

impl EngineOutput {
    pub(crate) fn one(change: ChangeSet) -> Self {
        Self {
            changes: vec![change],
        }
    }

    pub fn changes(&self) -> &[ChangeSet] {
        &self.changes
    }

    pub fn into_changes(self) -> Vec<ChangeSet> {
        self.changes
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}
