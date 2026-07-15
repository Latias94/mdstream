use mdstream_protocol::{Epoch, NodeId, NodeVersion, ProcessorInputVersion, RequestGeneration};

use crate::{IdentifierError, validate_identifier};

macro_rules! identifier {
    ($name:ident, $field:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                validate_identifier($field, &value, false)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identifier!(ProcessorId, "processor.id");
identifier!(ProcessorVersion, "processor.version");
identifier!(ConfigurationVersion, "processor.configuration_version");

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessorSlotKey {
    epoch: Epoch,
    node_id: NodeId,
    processor_id: ProcessorId,
}

impl ProcessorSlotKey {
    pub fn new(epoch: Epoch, node_id: NodeId, processor_id: ProcessorId) -> Self {
        Self {
            epoch,
            node_id,
            processor_id,
        }
    }

    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub const fn node_id(&self) -> NodeId {
        self.node_id
    }

    pub fn processor_id(&self) -> &ProcessorId {
        &self.processor_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessorRequestKey {
    slot: ProcessorSlotKey,
    node_version: NodeVersion,
    input_version: ProcessorInputVersion,
    processor_version: ProcessorVersion,
    configuration_version: ConfigurationVersion,
    generation: RequestGeneration,
}

impl ProcessorRequestKey {
    pub(crate) fn new(
        slot: ProcessorSlotKey,
        node_version: NodeVersion,
        input_version: ProcessorInputVersion,
        processor_version: ProcessorVersion,
        configuration_version: ConfigurationVersion,
        generation: RequestGeneration,
    ) -> Self {
        Self {
            slot,
            node_version,
            input_version,
            processor_version,
            configuration_version,
            generation,
        }
    }

    pub fn slot(&self) -> &ProcessorSlotKey {
        &self.slot
    }

    pub fn node_version(&self) -> &NodeVersion {
        &self.node_version
    }

    pub fn input_version(&self) -> &ProcessorInputVersion {
        &self.input_version
    }

    pub fn processor_version(&self) -> &ProcessorVersion {
        &self.processor_version
    }

    pub fn configuration_version(&self) -> &ConfigurationVersion {
        &self.configuration_version
    }

    pub const fn generation(&self) -> RequestGeneration {
        self.generation
    }
}
