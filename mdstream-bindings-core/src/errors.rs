use std::fmt;

use mdstream::{CompilerError, EngineError};
use mdstream_processors::{HostError, IdentifierError, ProcessorLimitsError};
use mdstream_protocol::{ProtocolError, ProtocolErrorCode};
use serde::Serialize;

const MAX_ERROR_MESSAGE_BYTES: usize = 16 * 1024;

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingStatus {
    Ok = 0,
    InvalidArgument = 1,
    Utf8 = 2,
    Options = 3,
    Command = 4,
    UnsupportedSchema = 5,
    Terminal = 6,
    Engine = 7,
    Protocol = 8,
    NeedsSnapshot = 9,
    Processor = 10,
    ResourceLimit = 11,
    Internal = 12,
    Panic = 13,
}

impl BindingStatus {
    pub const fn code(self) -> i32 {
        self as i32
    }

    pub const fn code_name(self) -> &'static str {
        match self {
            Self::Ok => "MDSTREAM_OK",
            Self::InvalidArgument => "MDSTREAM_INVALID_ARGUMENT",
            Self::Utf8 => "MDSTREAM_UTF8_ERROR",
            Self::Options => "MDSTREAM_OPTIONS_ERROR",
            Self::Command => "MDSTREAM_COMMAND_ERROR",
            Self::UnsupportedSchema => "MDSTREAM_UNSUPPORTED_SCHEMA",
            Self::Terminal => "MDSTREAM_TERMINAL",
            Self::Engine => "MDSTREAM_ENGINE_ERROR",
            Self::Protocol => "MDSTREAM_PROTOCOL_ERROR",
            Self::NeedsSnapshot => "MDSTREAM_NEEDS_SNAPSHOT",
            Self::Processor => "MDSTREAM_PROCESSOR_ERROR",
            Self::ResourceLimit => "MDSTREAM_RESOURCE_LIMIT_EXCEEDED",
            Self::Internal => "MDSTREAM_INTERNAL_ERROR",
            Self::Panic => "MDSTREAM_PANIC",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingError {
    status: BindingStatus,
    detail_code: String,
    message: String,
}

impl BindingError {
    pub fn new(
        status: BindingStatus,
        detail_code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            detail_code: detail_code.into(),
            message: truncate_utf8(message.into(), MAX_ERROR_MESSAGE_BYTES),
        }
    }

    pub const fn status(&self) -> BindingStatus {
        self.status
    }

    pub fn detail_code(&self) -> &str {
        &self.detail_code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn options(message: impl Into<String>) -> Self {
        Self::new(BindingStatus::Options, "bindings.invalid_options", message)
    }

    pub(crate) fn command(message: impl Into<String>) -> Self {
        Self::new(BindingStatus::Command, "bindings.invalid_command", message)
    }

    pub(crate) fn resource(field: &'static str, limit: usize, actual: usize) -> Self {
        Self::new(
            BindingStatus::ResourceLimit,
            "bindings.resource_limit",
            format!(
                "{field} uses {actual} {}, limit is {limit}",
                resource_unit(field)
            ),
        )
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(BindingStatus::Internal, "bindings.internal", message)
    }
}

impl fmt::Display for BindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BindingError {}

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    schema: &'static str,
    ok: bool,
    status: i32,
    status_name: &'static str,
    detail_code: &'a str,
    message: &'a str,
}

pub fn error_payload_json_bytes(error: &BindingError) -> Vec<u8> {
    serde_json::to_vec(&ErrorEnvelope {
        schema: crate::BINDING_SCHEMA,
        ok: false,
        status: error.status.code(),
        status_name: error.status.code_name(),
        detail_code: error.detail_code(),
        message: error.message(),
    })
    .unwrap_or_else(|_| {
        let status = BindingStatus::Internal;
        format!(
            r#"{{"schema":"{}","ok":false,"status":{},"status_name":"{}","detail_code":"bindings.error_encoding","message":"failed to encode binding error"}}"#,
            crate::BINDING_SCHEMA,
            status.code(),
            status.code_name()
        )
        .into_bytes()
    })
}

pub(crate) fn check_size(
    field: &'static str,
    bytes: &[u8],
    max_bytes: usize,
) -> Result<(), BindingError> {
    if bytes.len() > max_bytes {
        Err(BindingError::resource(field, max_bytes, bytes.len()))
    } else {
        Ok(())
    }
}

pub(crate) fn protocol_error(error: ProtocolError) -> BindingError {
    let status = match error.code() {
        ProtocolErrorCode::UnsupportedSchema => BindingStatus::UnsupportedSchema,
        ProtocolErrorCode::NeedsSnapshot => BindingStatus::NeedsSnapshot,
        ProtocolErrorCode::SourceTooLarge
        | ProtocolErrorCode::TooManyNodes
        | ProtocolErrorCode::TooManyOperations
        | ProtocolErrorCode::ValueTooLarge => BindingStatus::ResourceLimit,
        _ => BindingStatus::Protocol,
    };
    BindingError::new(
        status,
        format!("protocol.{}", protocol_code_name(error.code())),
        error.to_string(),
    )
}

pub(crate) fn engine_error(error: EngineError) -> BindingError {
    match error {
        EngineError::Finished => BindingError::new(
            BindingStatus::Terminal,
            "engine.finished",
            "stream engine is finalized",
        ),
        EngineError::LimitExceeded {
            field,
            limit,
            actual,
        } => BindingError::resource(field, limit, actual),
        EngineError::Protocol(error) => protocol_error(error),
        EngineError::Compiler(CompilerError::LimitExceeded {
            field,
            limit,
            actual,
        }) => BindingError::resource(field, limit, actual),
        EngineError::InternalInvariant(error) => BindingError::new(
            BindingStatus::Internal,
            "engine.internal_invariant",
            error.to_string(),
        ),
        other => BindingError::new(
            BindingStatus::Engine,
            "engine.transition",
            other.to_string(),
        ),
    }
}

pub(crate) fn host_error(error: HostError) -> BindingError {
    let status = match error {
        HostError::LimitExceeded { .. } => BindingStatus::ResourceLimit,
        HostError::NodeNotFound(_) | HostError::InvalidBodyRange(_) => {
            BindingStatus::InvalidArgument
        }
        _ => BindingStatus::Processor,
    };
    let detail_code = match &error {
        HostError::LimitExceeded { field, .. } => {
            let field = field.strip_prefix("processor.").unwrap_or(field);
            format!("processor.resource_limit.{field}")
        }
        _ => format!("processor.{}", error.code()),
    };
    BindingError::new(status, detail_code, error.to_string())
}

pub(crate) fn identifier_error(error: IdentifierError) -> BindingError {
    BindingError::new(
        BindingStatus::InvalidArgument,
        "processor.invalid_identifier",
        error.to_string(),
    )
}

pub(crate) fn processor_limits_error(error: ProcessorLimitsError) -> BindingError {
    BindingError::new(
        BindingStatus::Options,
        "processor.invalid_limits",
        error.to_string(),
    )
}

fn protocol_code_name(code: ProtocolErrorCode) -> &'static str {
    match code {
        ProtocolErrorCode::UnsupportedSchema => "unsupported_schema",
        ProtocolErrorCode::InvalidChange => "invalid_change",
        ProtocolErrorCode::InvalidSnapshot => "invalid_snapshot",
        ProtocolErrorCode::InvalidRange => "invalid_range",
        ProtocolErrorCode::CursorOverflow => "cursor_overflow",
        ProtocolErrorCode::MetadataOverflow => "metadata_overflow",
        ProtocolErrorCode::SequenceOverflow => "sequence_overflow",
        ProtocolErrorCode::SourceTooLarge => "source_too_large",
        ProtocolErrorCode::TooManyNodes => "too_many_nodes",
        ProtocolErrorCode::TooManyOperations => "too_many_operations",
        ProtocolErrorCode::ValueTooLarge => "value_too_large",
        ProtocolErrorCode::MissingNode => "missing_node",
        ProtocolErrorCode::MissingResource => "missing_resource",
        ProtocolErrorCode::DuplicateNode => "duplicate_node",
        ProtocolErrorCode::DuplicateResource => "duplicate_resource",
        ProtocolErrorCode::VersionMismatch => "version_mismatch",
        ProtocolErrorCode::ResourceVersionMismatch => "resource_version_mismatch",
        ProtocolErrorCode::IllegalLifecycle => "illegal_lifecycle",
        ProtocolErrorCode::NeedsSnapshot => "needs_snapshot",
        ProtocolErrorCode::SnapshotNotAllowed => "snapshot_not_allowed",
        ProtocolErrorCode::InvalidEpochStart => "invalid_epoch_start",
        ProtocolErrorCode::StaleSnapshot => "stale_snapshot",
    }
}

fn resource_unit(field: &str) -> &'static str {
    match field {
        "markdown.events" => "events",
        "markdown.footnote_overlap_work" => "work units",
        _ if field.ends_with("_bytes") => "bytes",
        _ => "items",
    }
}

fn truncate_utf8(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

#[cfg(test)]
mod tests {
    use super::{host_error, resource_unit};
    use mdstream_processors::HostError;

    #[test]
    fn resource_messages_use_domain_units() {
        assert_eq!(resource_unit("markdown.events"), "events");
        assert_eq!(
            resource_unit("markdown.footnote_overlap_work"),
            "work units"
        );
        assert_eq!(resource_unit("engine.change_bytes"), "bytes");
        assert_eq!(resource_unit("protocol.nodes"), "items");
    }

    #[test]
    fn processor_limit_detail_identifies_the_releasing_budget() {
        let error = host_error(HostError::LimitExceeded {
            field: "processor.in_flight_jobs",
            limit: 1,
            actual: 2,
        });
        assert_eq!(
            error.detail_code(),
            "processor.resource_limit.in_flight_jobs"
        );
    }
}
