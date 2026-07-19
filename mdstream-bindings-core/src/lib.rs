#![forbid(unsafe_code)]

//! Safe shared facade for stateful mdstream transports.

mod commands;
mod engine;
mod errors;
mod options;
mod wire;

pub use engine::{EngineSession, ReducerSession};
pub use errors::{BindingError, BindingStatus, error_payload_json_bytes};
pub use mdstream_processors::{ProcessorExpectation, ProcessorFailureCode};
pub use options::BINDING_OPTIONS_SCHEMA;
pub use wire::{
    BINDING_SCHEMA, BindingMetrics, BindingOutput, BindingPayload, BindingPayloadKind,
    TRANSITION_SCHEMA_DRAFT,
};
