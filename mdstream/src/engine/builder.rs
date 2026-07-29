use super::{CompilerError, CompilerLimits, CustomBlockSpec, EngineLimits, StreamEngine};
use crate::syntax::containers::names_match;
use mdstream_protocol::ProtocolLimits;

#[derive(Debug, Default)]
pub struct StreamEngineBuilder {
    custom_blocks: Vec<CustomBlockSpec>,
    protocol_limits: ProtocolLimits,
    compiler_limits: CompilerLimits,
    engine_limits: EngineLimits,
}

impl StreamEngineBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn custom_block(mut self, spec: CustomBlockSpec) -> Self {
        self.custom_blocks.push(spec);
        self
    }

    pub fn protocol_limits(mut self, limits: ProtocolLimits) -> Self {
        self.protocol_limits = limits;
        self
    }

    pub fn compiler_limits(mut self, limits: CompilerLimits) -> Self {
        self.compiler_limits = limits;
        self
    }

    pub fn engine_limits(mut self, limits: EngineLimits) -> Self {
        self.engine_limits = limits;
        self
    }

    pub fn build(self) -> Result<StreamEngine, CompilerError> {
        validate_custom_blocks(&self.custom_blocks, self.protocol_limits)?;
        Ok(StreamEngine::with_custom_blocks(
            self.custom_blocks,
            self.protocol_limits,
            self.compiler_limits,
            self.engine_limits,
        ))
    }
}

fn validate_custom_blocks(
    custom_blocks: &[CustomBlockSpec],
    limits: ProtocolLimits,
) -> Result<(), CompilerError> {
    for (index, spec) in custom_blocks.iter().enumerate() {
        validate_custom_metadata(spec, limits)?;
        for other in &custom_blocks[index + 1..] {
            let names_overlap = names_match(
                spec.name(),
                other.name(),
                spec.is_case_insensitive() || other.is_case_insensitive(),
            );
            if names_overlap {
                return Err(CompilerError::InvalidConfiguration(format!(
                    "custom block tag rules overlap for {:?} and {:?}",
                    spec.name(),
                    other.name()
                )));
            }
        }
    }
    Ok(())
}

fn validate_custom_metadata(
    spec: &CustomBlockSpec,
    limits: ProtocolLimits,
) -> Result<(), CompilerError> {
    for (field, value) in [
        ("custom.namespace", spec.namespace()),
        ("custom.name", spec.name()),
    ] {
        if value.len() > limits.max_metadata_value_bytes {
            return Err(CompilerError::InvalidConfiguration(format!(
                "{field} uses {} metadata bytes, exceeding the configured limit of {}",
                value.len(),
                limits.max_metadata_value_bytes
            )));
        }
    }

    let static_bytes = spec
        .namespace()
        .len()
        .checked_add(spec.name().len())
        .ok_or_else(|| {
            CompilerError::InvalidConfiguration(
                "custom block static metadata length overflowed".to_string(),
            )
        })?;
    for (field, limit) in [
        ("node.metadata", limits.max_node_metadata_bytes),
        ("change.metadata", limits.max_change_metadata_bytes),
        ("document.metadata", limits.max_document_metadata_bytes),
    ] {
        if static_bytes > limit {
            return Err(CompilerError::InvalidConfiguration(format!(
                "custom block static metadata uses {static_bytes} bytes, exceeding the configured {field} limit of {limit}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use mdstream_protocol::ProtocolLimits;

    use super::*;

    #[test]
    fn custom_block_specs_reject_invalid_protocol_identity_and_tag_names() {
        for (namespace, name) in [
            ("", "thinking"),
            ("bad namespace", "thinking"),
            ("app.custom/1", ""),
            ("x", "1bad"),
        ] {
            assert!(matches!(
                CustomBlockSpec::try_new(namespace, name),
                Err(crate::compiler::CompilerError::InvalidConfiguration(_))
            ));
        }
    }

    #[test]
    fn builder_rejects_overlapping_custom_block_rules() {
        let result = StreamEngineBuilder::new()
            .custom_block(CustomBlockSpec::try_new("app.one/1", "thinking").unwrap())
            .custom_block(
                CustomBlockSpec::try_new("app.two/1", "THINKING")
                    .unwrap()
                    .case_insensitive(false),
            )
            .build();

        assert!(matches!(
            result,
            Err(CompilerError::InvalidConfiguration(_))
        ));
    }

    #[test]
    fn builder_rejects_custom_metadata_outside_effective_limits() {
        let value_limited = ProtocolLimits {
            max_metadata_value_bytes: 3,
            ..ProtocolLimits::default()
        };
        for spec in [
            CustomBlockSpec::try_new("long", "x").unwrap(),
            CustomBlockSpec::try_new("x", "long").unwrap(),
        ] {
            assert!(matches!(
                StreamEngineBuilder::new()
                    .protocol_limits(value_limited)
                    .custom_block(spec)
                    .build(),
                Err(CompilerError::InvalidConfiguration(_))
            ));
        }

        for limits in [
            ProtocolLimits {
                max_node_metadata_bytes: 7,
                ..ProtocolLimits::default()
            },
            ProtocolLimits {
                max_change_metadata_bytes: 7,
                ..ProtocolLimits::default()
            },
            ProtocolLimits {
                max_document_metadata_bytes: 7,
                ..ProtocolLimits::default()
            },
        ] {
            assert!(matches!(
                StreamEngineBuilder::new()
                    .protocol_limits(limits)
                    .custom_block(CustomBlockSpec::try_new("space", "tag").unwrap())
                    .build(),
                Err(CompilerError::InvalidConfiguration(_))
            ));
        }

        StreamEngineBuilder::new()
            .protocol_limits(ProtocolLimits {
                max_metadata_value_bytes: 5,
                max_node_metadata_bytes: 8,
                max_change_metadata_bytes: 8,
                max_document_metadata_bytes: 8,
                ..ProtocolLimits::default()
            })
            .custom_block(CustomBlockSpec::try_new("space", "tag").unwrap())
            .build()
            .expect("exact static metadata budgets must be accepted");
    }
}
