#![forbid(unsafe_code)]

//! WebAssembly transport for mdstream binding sessions.

use std::str::FromStr;

use mdstream_bindings_core::{
    BINDING_OPTIONS_SCHEMA, BINDING_SCHEMA, BindingError, BindingMetrics, BindingOutput,
    BindingPayload, BindingPayloadKind, BindingStatus, EngineSession, ProcessorFailureCode,
    ReducerSession, error_payload_json_bytes,
};
use mdstream_protocol::{DecimalIdError, RequestGeneration};
use wasm_bindgen::prelude::*;

const WASM_ABI_VERSION: u32 = 1;
const METRICS_FRAME_VERSION: u8 = 1;
const BINDING_METRICS_KIND: u8 = 1;
const PROCESSOR_METRICS_KIND: u8 = 2;

#[cfg(feature = "panic-hook")]
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen(js_name = abiVersion)]
pub fn abi_version() -> u32 {
    WASM_ABI_VERSION
}

#[wasm_bindgen(js_name = packageVersion)]
pub fn package_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen(js_name = bindingSchema)]
pub fn binding_schema() -> String {
    BINDING_SCHEMA.to_string()
}

#[wasm_bindgen(js_name = bindingOptionsSchema)]
pub fn binding_options_schema() -> String {
    BINDING_OPTIONS_SCHEMA.to_string()
}

#[wasm_bindgen]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MdstreamPayloadKind {
    Change = 1,
    Snapshot = 2,
    ReducerUpdate = 3,
    NodeView = 4,
    ResourceView = 5,
    ProcessorRequest = 6,
    ProcessorCompletion = 7,
    ArtifactChange = 8,
    ArtifactView = 9,
}

#[wasm_bindgen]
#[derive(Debug)]
pub struct MdstreamOutput {
    payloads: Vec<Option<BindingPayload>>,
}

impl From<BindingOutput> for MdstreamOutput {
    fn from(output: BindingOutput) -> Self {
        Self {
            payloads: output.into_payloads().into_iter().map(Some).collect(),
        }
    }
}

#[wasm_bindgen]
impl MdstreamOutput {
    #[wasm_bindgen(getter = len)]
    pub fn payload_count(&self) -> usize {
        self.payloads.len()
    }

    #[wasm_bindgen(js_name = remaining)]
    pub fn remaining(&self) -> usize {
        self.payloads.iter().flatten().count()
    }

    pub fn kind(&self, index: usize) -> Result<MdstreamPayloadKind, JsValue> {
        let payload = self.payload(index)?;
        Ok(payload_kind(payload.kind()))
    }

    pub fn count(&self, kind: MdstreamPayloadKind) -> usize {
        self.payloads
            .iter()
            .flatten()
            .filter(|payload| payload_kind(payload.kind()) == kind)
            .count()
    }

    pub fn take(&mut self, index: usize) -> Result<Vec<u8>, JsValue> {
        let payload = self
            .payloads
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| output_index_error(index))?;
        Ok(payload.into_bytes())
    }
}

impl MdstreamOutput {
    fn payload(&self, index: usize) -> Result<&BindingPayload, JsValue> {
        self.payloads
            .get(index)
            .and_then(Option::as_ref)
            .ok_or_else(|| output_index_error(index))
    }
}

#[wasm_bindgen]
#[derive(Debug)]
pub struct MdstreamEngineSession {
    inner: EngineSession,
}

#[wasm_bindgen]
impl MdstreamEngineSession {
    #[wasm_bindgen(constructor)]
    pub fn new(options_json: Option<String>) -> Result<MdstreamEngineSession, JsValue> {
        let inner = EngineSession::new(options_json.as_deref().unwrap_or_default().as_bytes())
            .map_err(binding_error_to_js)?;
        Ok(Self { inner })
    }

    pub fn append(&mut self, chunk: &str) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .append(chunk.as_bytes())
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    pub fn finish(&mut self) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .finish()
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    pub fn reset(&mut self) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .reset()
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    pub fn snapshot(&mut self) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .snapshot()
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    pub fn metrics(&self) -> Vec<u8> {
        binding_metrics_bytes(self.inner.metrics())
    }
}

#[wasm_bindgen]
#[derive(Debug)]
pub struct MdstreamReducerSession {
    inner: ReducerSession,
}

#[wasm_bindgen]
impl MdstreamReducerSession {
    #[wasm_bindgen(constructor)]
    pub fn new(options_json: Option<String>) -> Result<MdstreamReducerSession, JsValue> {
        let inner = ReducerSession::new(options_json.as_deref().unwrap_or_default().as_bytes())
            .map_err(binding_error_to_js)?;
        Ok(Self { inner })
    }

    #[wasm_bindgen(js_name = applyChange)]
    pub fn apply_change(&mut self, change_json: &[u8]) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .apply_change(change_json)
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = recoverSnapshot)]
    pub fn recover_snapshot(&mut self, snapshot_json: &[u8]) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .recover_snapshot(snapshot_json)
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    pub fn snapshot(&mut self) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .snapshot()
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = nodeView)]
    pub fn node_view(&mut self, node_id: &str) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .node_view(parse_decimal_id(node_id, "node_id")?)
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = resourceView)]
    pub fn resource_view(&mut self, resource_id: &str) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .resource_view(parse_decimal_id(resource_id, "resource_id")?)
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = beginProcessor)]
    pub fn begin_processor(
        &mut self,
        node_id: &str,
        processor_id: &str,
        processor_version: &str,
        configuration_version: &str,
        accepts_provisional: bool,
        allow_provisional: bool,
    ) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .begin_processor(
                parse_decimal_id(node_id, "node_id")?,
                processor_id.to_string(),
                processor_version.to_string(),
                configuration_version.to_string(),
                accepts_provisional,
                allow_provisional,
            )
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = artifactView)]
    pub fn artifact_view(
        &mut self,
        epoch: &str,
        node_id: &str,
        processor_id: &str,
    ) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .artifact_view_for(
                parse_decimal_id(epoch, "epoch")?,
                parse_decimal_id(node_id, "node_id")?,
                processor_id.to_string(),
            )
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = completeProcessorText)]
    pub fn complete_processor_text(
        &mut self,
        request_id: &str,
        protocol: &str,
        media_type: &str,
        text: &str,
    ) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .complete_processor_text(
                parse_request_generation(request_id)?,
                protocol.to_string(),
                media_type.to_string(),
                text.to_string(),
            )
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = completeProcessorBinary)]
    pub fn complete_processor_binary(
        &mut self,
        request_id: &str,
        protocol: &str,
        media_type: &str,
        bytes: Vec<u8>,
    ) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .complete_processor_binary(
                parse_request_generation(request_id)?,
                protocol.to_string(),
                media_type.to_string(),
                bytes,
            )
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = failProcessor)]
    pub fn fail_processor(
        &mut self,
        request_id: &str,
        code: &str,
        message: &str,
    ) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .fail_processor(
                parse_request_generation(request_id)?,
                parse_processor_failure_code(code)?,
                message.to_string(),
            )
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    #[wasm_bindgen(js_name = cancelProcessor)]
    pub fn cancel_processor(&mut self, request_id: &str) -> Result<MdstreamOutput, JsValue> {
        self.inner
            .cancel_processor(parse_request_generation(request_id)?)
            .map(MdstreamOutput::from)
            .map_err(binding_error_to_js)
    }

    pub fn status(&self) -> String {
        match self.inner.status() {
            mdstream_protocol::ReducerStatus::Uninitialized => "uninitialized",
            mdstream_protocol::ReducerStatus::Ready => "ready",
            mdstream_protocol::ReducerStatus::NeedsSnapshot { .. } => "needs_snapshot",
        }
        .to_string()
    }

    pub fn metrics(&self) -> Vec<u8> {
        binding_metrics_bytes(self.inner.metrics())
    }

    #[wasm_bindgen(js_name = processorMetrics)]
    pub fn processor_metrics(&self) -> Vec<u8> {
        let metrics = self.inner.processor_metrics();
        metrics_frame(
            PROCESSOR_METRICS_KIND,
            &[
                usize_metric(metrics.slots),
                usize_metric(metrics.in_flight_jobs),
                usize_metric(metrics.in_flight_input_bytes),
                usize_metric(metrics.retained_artifacts),
                usize_metric(metrics.retained_artifact_bytes),
                usize_metric(metrics.pending_changes),
                usize_metric(metrics.pending_change_bytes),
                metrics.issued_requests,
                metrics.accepted_results,
                metrics.stale_results,
                metrics.released_artifacts,
                metrics.store_entry_visits,
                metrics.input_materializations,
            ],
        )
    }
}

fn binding_metrics_bytes(metrics: BindingMetrics) -> Vec<u8> {
    metrics_frame(
        BINDING_METRICS_KIND,
        &[
            metrics.commands,
            metrics.decoded_change_payloads,
            metrics.decoded_snapshot_payloads,
            metrics.change_payloads,
            metrics.snapshot_payloads,
            metrics.reducer_update_payloads,
            metrics.processor_request_payloads,
            metrics.processor_completion_payloads,
            metrics.artifact_change_payloads,
            metrics.artifact_view_payloads,
            metrics.materialized_node_views,
            metrics.materialized_resource_views,
            metrics.encoded_payload_bytes,
            metrics.pending_processor_requests,
        ],
    )
}

fn metrics_frame(kind: u8, values: &[u64]) -> Vec<u8> {
    let count = u8::try_from(values.len()).expect("metrics field count fits the frame");
    let mut frame = Vec::with_capacity(6 + values.len() * 8);
    frame.extend_from_slice(b"MDM");
    frame.push(METRICS_FRAME_VERSION);
    frame.push(kind);
    frame.push(count);
    for value in values {
        frame.extend_from_slice(&value.to_le_bytes());
    }
    frame
}

fn usize_metric(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

const fn payload_kind(kind: BindingPayloadKind) -> MdstreamPayloadKind {
    match kind {
        BindingPayloadKind::Change => MdstreamPayloadKind::Change,
        BindingPayloadKind::Snapshot => MdstreamPayloadKind::Snapshot,
        BindingPayloadKind::ReducerUpdate => MdstreamPayloadKind::ReducerUpdate,
        BindingPayloadKind::NodeView => MdstreamPayloadKind::NodeView,
        BindingPayloadKind::ResourceView => MdstreamPayloadKind::ResourceView,
        BindingPayloadKind::ProcessorRequest => MdstreamPayloadKind::ProcessorRequest,
        BindingPayloadKind::ProcessorCompletion => MdstreamPayloadKind::ProcessorCompletion,
        BindingPayloadKind::ArtifactChange => MdstreamPayloadKind::ArtifactChange,
        BindingPayloadKind::ArtifactView => MdstreamPayloadKind::ArtifactView,
    }
}

fn output_index_error(index: usize) -> JsValue {
    binding_error_to_js(BindingError::new(
        BindingStatus::InvalidArgument,
        "bindings.output_index",
        format!("binding output payload {index} is missing or already consumed"),
    ))
}

fn parse_request_generation(value: &str) -> Result<RequestGeneration, JsValue> {
    parse_decimal_id(value, "request_id")
}

fn parse_decimal_id<T>(value: &str, field: &'static str) -> Result<T, JsValue>
where
    T: FromStr<Err = DecimalIdError>,
{
    value.parse().map_err(|error| {
        binding_error_to_js(BindingError::new(
            BindingStatus::InvalidArgument,
            "bindings.decimal_id",
            format!("{field} is not a canonical decimal string: {error}"),
        ))
    })
}

fn parse_processor_failure_code(value: &str) -> Result<ProcessorFailureCode, JsValue> {
    let code = match value {
        "processor" => ProcessorFailureCode::Processor,
        "panic" => ProcessorFailureCode::Panic,
        "invalid_request" => ProcessorFailureCode::InvalidRequest,
        "cancelled" => ProcessorFailureCode::Cancelled,
        "unsupported_content" => ProcessorFailureCode::UnsupportedContent,
        "unresolved_context" => ProcessorFailureCode::UnresolvedContext,
        "invalid_context" => ProcessorFailureCode::InvalidContext,
        "resource_limit" => ProcessorFailureCode::ResourceLimit,
        _ => {
            return Err(binding_error_to_js(BindingError::new(
                BindingStatus::InvalidArgument,
                "bindings.processor_failure_code",
                format!("unsupported processor failure code {value:?}"),
            )));
        }
    };
    Ok(code)
}

fn binding_error_to_js(error: BindingError) -> JsValue {
    let bytes = error_payload_json_bytes(&error);
    let Ok(payload) = String::from_utf8(bytes) else {
        return JsValue::from_str(&format!("{}: {error}", error.status().code_name()));
    };
    js_sys::JSON::parse(&payload)
        .unwrap_or_else(|_| JsValue::from_str(&format!("{}: {error}", error.status().code_name())))
}
