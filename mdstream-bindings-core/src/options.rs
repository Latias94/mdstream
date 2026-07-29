use mdstream::{CompilerLimits, CustomBlockSpec, EngineLimits, StreamEngine, StreamEngineBuilder};
use mdstream_processors::{ArtifactHost, ProcessorLimits};
use mdstream_protocol::{ProtocolLimits, Reducer};
use serde::Deserialize;

use crate::errors::{BindingError, engine_error, processor_limits_error};

pub const BINDING_OPTIONS_SCHEMA: &str = "mdstream.bindings-options/0.4";
const MAX_OPTIONS_BYTES: usize = 64 * 1024;
const JSON_ESCAPE_FACTOR: usize = 6;
const BINDING_ENVELOPE_BYTES: usize = 4 * 1024;
const IMPACT_ID_BYTES: usize = 96;
const TRANSITION_ENVELOPE_BYTES: usize = 8 * 1024;
const TRANSITION_KEY_BYTES: usize = 192;
const NODE_TRANSITION_BYTES: usize = 2 * 1024;
const RESOURCE_TRANSITION_BYTES: usize = 1024;
const STRUCTURE_TRANSITION_BYTES: usize = 1024;
const NODE_STRUCTURAL_ITEM_BYTES: usize = 64;
const NODE_STRUCTURAL_LISTS: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WireLimits {
    pub max_command_bytes: usize,
    pub max_encoded_change_bytes: usize,
    pub max_encoded_snapshot_bytes: usize,
    pub max_reducer_update_bytes: usize,
    pub max_processor_payload_bytes: usize,
    pub max_artifact_event_bytes: usize,
    pub max_view_bytes: usize,
}

impl Default for WireLimits {
    fn default() -> Self {
        Self {
            max_command_bytes: 512 * 1024 * 1024,
            max_encoded_change_bytes: 384 * 1024 * 1024,
            max_encoded_snapshot_bytes: 256 * 1024 * 1024,
            max_reducer_update_bytes: 64 * 1024 * 1024,
            max_processor_payload_bytes: 32 * 1024 * 1024,
            max_artifact_event_bytes: 8 * 1024 * 1024,
            max_view_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Default)]
pub(crate) struct BindingOptions {
    protocol: ProtocolLimits,
    compiler: CompilerLimits,
    engine: EngineLimits,
    processor: ProcessorLimits,
    wire: WireLimits,
    capture_transitions: bool,
    custom_blocks: Vec<CustomBlockSpec>,
}

impl BindingOptions {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, BindingError> {
        if bytes.len() > MAX_OPTIONS_BYTES {
            return Err(BindingError::resource(
                "bindings.options_bytes",
                MAX_OPTIONS_BYTES,
                bytes.len(),
            ));
        }
        if bytes.is_empty() {
            return Ok(Self::default());
        }

        let raw: RawBindingOptions = serde_json::from_slice(bytes).map_err(|error| {
            BindingError::options(format!("invalid binding options JSON: {error}"))
        })?;
        if raw.schema != BINDING_OPTIONS_SCHEMA {
            return Err(BindingError::new(
                crate::BindingStatus::UnsupportedSchema,
                "bindings.unsupported_options_schema",
                format!("unsupported binding options schema: {}", raw.schema),
            ));
        }

        let mut options = Self::default();
        if let Some(protocol) = raw.protocol {
            protocol.apply(&mut options.protocol)?;
        }
        if let Some(compiler) = raw.compiler {
            compiler.apply(&mut options.compiler)?;
        }
        if let Some(engine) = raw.engine {
            engine.apply(&mut options.engine)?;
        }
        if let Some(processor) = raw.processor {
            processor.apply(&mut options.processor)?;
        }
        if let Some(wire) = raw.wire {
            wire.apply(&mut options.wire)?;
        }
        options.capture_transitions = raw.capture_transitions;
        options.custom_blocks = raw
            .custom_blocks
            .into_iter()
            .map(RawCustomBlock::build)
            .collect::<Result<_, _>>()?;
        options
            .processor
            .validate()
            .map_err(processor_limits_error)?;
        Ok(options)
    }

    pub(crate) fn into_engine(
        self,
    ) -> Result<(StreamEngine, ProtocolLimits, WireLimits), BindingError> {
        let required = self
            .engine
            .minimum_encoded_change_bytes()
            .ok_or_else(|| BindingError::options("engine change wire bound overflowed"))?;
        if self.wire.max_encoded_change_bytes < required {
            return Err(BindingError::options(format!(
                "wire.max_encoded_change_bytes must be at least {required} for engine.max_change_bytes {}",
                self.engine.max_change_bytes
            )));
        }

        let protocol = self.protocol;
        let mut builder = StreamEngineBuilder::new()
            .protocol_limits(self.protocol)
            .compiler_limits(self.compiler)
            .engine_limits(self.engine);
        for spec in self.custom_blocks {
            builder = builder.custom_block(spec);
        }
        let engine = builder.build().map_err(|error| {
            let mapped = engine_error(mdstream::EngineError::Compiler(error));
            BindingError::new(
                crate::BindingStatus::Options,
                "bindings.invalid_engine_configuration",
                mapped.message(),
            )
        })?;
        Ok((engine, protocol, self.wire))
    }

    pub(crate) const fn capture_transitions(&self) -> bool {
        self.capture_transitions
    }

    pub(crate) fn into_reducer(
        self,
    ) -> Result<(Reducer, ArtifactHost, ProtocolLimits, WireLimits), BindingError> {
        let reducer_update_bound =
            minimum_reducer_update_bytes(self.protocol, self.capture_transitions)
                .ok_or_else(|| BindingError::options("reducer update wire bound overflowed"))?;
        if self.wire.max_reducer_update_bytes < reducer_update_bound {
            return Err(BindingError::options(format!(
                "wire.max_reducer_update_bytes must be at least {reducer_update_bound} for configured protocol limits with capture_transitions={}",
                self.capture_transitions
            )));
        }

        let processor_payload_bound = self
            .processor
            .max_input_bytes
            .max(self.processor.max_artifact_bytes)
            .max(self.processor.max_error_bytes)
            .checked_mul(JSON_ESCAPE_FACTOR)
            .and_then(|bytes| bytes.checked_add(BINDING_ENVELOPE_BYTES))
            .ok_or_else(|| BindingError::options("processor payload wire bound overflowed"))?;
        if self.wire.max_processor_payload_bytes < processor_payload_bound {
            return Err(BindingError::options(format!(
                "wire.max_processor_payload_bytes must be at least {processor_payload_bound} for configured processor input and artifact limits"
            )));
        }

        let artifact_event_bound = self
            .processor
            .max_pending_change_bytes
            .checked_mul(JSON_ESCAPE_FACTOR)
            .and_then(|bytes| bytes.checked_add(BINDING_ENVELOPE_BYTES))
            .ok_or_else(|| BindingError::options("artifact event wire bound overflowed"))?;
        if self.wire.max_artifact_event_bytes < artifact_event_bound {
            return Err(BindingError::options(format!(
                "wire.max_artifact_event_bytes must be at least {artifact_event_bound} for configured processor limits"
            )));
        }

        let node_structural_items = self
            .protocol
            .max_children_per_list
            .checked_mul(NODE_STRUCTURAL_LISTS)
            .and_then(|items| items.checked_add(self.protocol.max_attributes_per_node))
            .ok_or_else(|| BindingError::options("node view structural bound overflowed"))?;
        let node_view_bound = node_structural_items
            .checked_mul(NODE_STRUCTURAL_ITEM_BYTES)
            .and_then(|bytes| bytes.checked_add(self.protocol.max_source_bytes))
            .and_then(|bytes| bytes.checked_add(self.protocol.max_node_metadata_bytes))
            .and_then(|bytes| bytes.checked_add(BINDING_ENVELOPE_BYTES))
            .and_then(|bytes| bytes.checked_mul(JSON_ESCAPE_FACTOR))
            .ok_or_else(|| BindingError::options("node view wire bound overflowed"))?;
        let artifact_view_bound = self
            .processor
            .max_artifact_bytes
            .max(self.processor.max_error_bytes)
            .checked_add(BINDING_ENVELOPE_BYTES)
            .and_then(|bytes| bytes.checked_mul(JSON_ESCAPE_FACTOR))
            .ok_or_else(|| BindingError::options("artifact view wire bound overflowed"))?;
        let required_view_bytes = node_view_bound.max(artifact_view_bound);
        if self.wire.max_view_bytes < required_view_bytes {
            return Err(BindingError::options(format!(
                "wire.max_view_bytes must be at least {required_view_bytes} for configured protocol and processor limits"
            )));
        }

        let reducer = Reducer::with_limits(self.protocol);
        let host = ArtifactHost::new(self.processor).map_err(processor_limits_error)?;
        Ok((reducer, host, self.protocol, self.wire))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBindingOptions {
    schema: String,
    #[serde(default)]
    protocol: Option<RawProtocolLimits>,
    #[serde(default)]
    compiler: Option<RawCompilerLimits>,
    #[serde(default)]
    engine: Option<RawEngineLimits>,
    #[serde(default)]
    processor: Option<RawProcessorLimits>,
    #[serde(default)]
    wire: Option<RawWireLimits>,
    #[serde(default)]
    capture_transitions: bool,
    #[serde(default)]
    custom_blocks: Vec<RawCustomBlock>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCustomBlock {
    namespace: String,
    name: String,
    #[serde(default = "default_true")]
    opaque: bool,
    #[serde(default = "default_true")]
    case_insensitive: bool,
}

impl RawCustomBlock {
    fn build(self) -> Result<CustomBlockSpec, BindingError> {
        CustomBlockSpec::try_new(self.namespace, self.name)
            .map(|spec| {
                spec.opaque(self.opaque)
                    .case_insensitive(self.case_insensitive)
            })
            .map_err(|error| BindingError::options(error.to_string()))
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct DecimalUsize(String);

impl DecimalUsize {
    fn parse(self, field: &'static str) -> Result<usize, BindingError> {
        parse_decimal_usize(&self.0, field)
    }
}

macro_rules! raw_limits {
    ($name:ident => $target:ty { $($field:ident),+ $(,)? }) => {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $name {
            $(#[serde(default)] $field: Option<DecimalUsize>,)+
        }

        impl $name {
            fn apply(self, target: &mut $target) -> Result<(), BindingError> {
                $(if let Some(value) = self.$field {
                    target.$field = value.parse(stringify!($field))?;
                })+
                Ok(())
            }
        }
    };
}

raw_limits!(RawProtocolLimits => ProtocolLimits {
    max_source_bytes,
    max_nodes,
    max_resources,
    max_operations,
    max_change_structural_items,
    max_document_structural_items,
    max_children_per_list,
    max_attributes_per_node,
    max_metadata_value_bytes,
    max_node_metadata_bytes,
    max_change_metadata_bytes,
    max_document_metadata_bytes,
    max_tree_depth,
});

raw_limits!(RawCompilerLimits => CompilerLimits {
    max_markdown_events,
    max_markdown_overlap_work,
    max_definitions,
    max_definition_edges,
    max_definition_metadata_bytes,
});

raw_limits!(RawEngineLimits => EngineLimits {
    max_change_bytes,
    max_transaction_bytes,
});

raw_limits!(RawProcessorLimits => ProcessorLimits {
    max_input_bytes,
    max_artifact_bytes,
    max_in_flight_jobs,
    max_in_flight_input_bytes,
    max_slots,
    max_retained_artifacts,
    max_retained_artifact_bytes,
    max_error_bytes,
    max_pending_changes,
    max_pending_change_bytes,
});

raw_limits!(RawWireLimits => WireLimits {
    max_command_bytes,
    max_encoded_change_bytes,
    max_encoded_snapshot_bytes,
    max_reducer_update_bytes,
    max_processor_payload_bytes,
    max_artifact_event_bytes,
    max_view_bytes,
});

fn minimum_reducer_update_bytes(
    protocol: ProtocolLimits,
    capture_transitions: bool,
) -> Option<usize> {
    // Old and new populations cover transitions that replace every legal node or resource.
    // Fixed record allowances include opaque IDs and continuity-qualified parent keys; only
    // source text needs the full JSON escape multiplier.
    let two_node_populations = protocol.max_nodes.checked_mul(2)?;
    let two_resource_populations = protocol.max_resources.checked_mul(2)?;
    // `changed_*` can contain disjoint before/after populations, while each
    // `removed_*` list can repeat the complete before population.
    let impact_ids = protocol
        .max_nodes
        .checked_mul(3)?
        .checked_add(protocol.max_resources.checked_mul(3)?)?;
    let root_ids = protocol.max_children_per_list.min(protocol.max_nodes);
    let mut bytes = impact_ids
        .checked_add(root_ids)?
        .checked_mul(IMPACT_ID_BYTES)?
        .checked_add(BINDING_ENVELOPE_BYTES)?;

    if !capture_transitions {
        return Some(bytes);
    }

    let resource_facts = two_resource_populations.min(protocol.max_operations);
    let structure_owners = protocol.max_nodes.checked_add(1)?;
    let structure_facts = structure_owners.min(protocol.max_operations);
    let inserted_splice_ids = protocol.max_change_structural_items.min(protocol.max_nodes);
    let splice_ids = protocol.max_nodes.checked_add(inserted_splice_ids)?;

    bytes = bytes
        .checked_add(TRANSITION_ENVELOPE_BYTES)?
        .checked_add(two_node_populations.checked_mul(NODE_TRANSITION_BYTES)?)?
        .checked_add(resource_facts.checked_mul(RESOURCE_TRANSITION_BYTES)?)?
        .checked_add(structure_facts.checked_mul(STRUCTURE_TRANSITION_BYTES)?)?
        .checked_add(two_node_populations.checked_mul(TRANSITION_KEY_BYTES)?)?
        .checked_add(splice_ids.checked_mul(TRANSITION_KEY_BYTES)?)?
        .checked_add(protocol.max_source_bytes.checked_mul(JSON_ESCAPE_FACTOR)?)?;
    Some(bytes)
}

fn parse_decimal_usize(value: &str, field: &'static str) -> Result<usize, BindingError> {
    let canonical = value == "0"
        || value
            .as_bytes()
            .first()
            .is_some_and(|first| matches!(first, b'1'..=b'9'))
            && value.as_bytes()[1..].iter().all(u8::is_ascii_digit);
    if !canonical {
        return Err(BindingError::options(format!(
            "{field} must be a canonical unsigned decimal string"
        )));
    }
    let parsed = value.parse::<u64>().map_err(|_| {
        BindingError::options(format!("{field} exceeds the supported unsigned range"))
    })?;
    usize::try_from(parsed)
        .map_err(|_| BindingError::options(format!("{field} does not fit this target")))
}
