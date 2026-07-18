use std::str::FromStr;

use mdstream_processors::{ProcessingPolicy, ProcessorFailureCode};
use mdstream_protocol::DecimalIdError;
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};
use serde_json::value::RawValue;

use crate::{BINDING_SCHEMA, BindingError, BindingStatus, errors::check_size};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub(crate) enum EngineCommand {
    Append { schema: String, chunk: String },
    Finish { schema: String },
    Reset { schema: String },
    Snapshot { schema: String },
}

impl EngineCommand {
    fn schema(&self) -> &str {
        match self {
            Self::Append { schema, .. }
            | Self::Finish { schema }
            | Self::Reset { schema }
            | Self::Snapshot { schema } => schema,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ReducerCommand<'a> {
    ApplyChange {
        schema: String,
        change: &'a RawValue,
    },
    RecoverSnapshot {
        schema: String,
        snapshot: &'a RawValue,
    },
    Snapshot {
        schema: String,
    },
    NodeView {
        schema: String,
        node_id: String,
    },
    ResourceView {
        schema: String,
        resource_id: String,
    },
    PendingSourceView {
        schema: String,
    },
    BeginProcessor {
        schema: String,
        node_id: String,
        processor_id: String,
        processor_version: String,
        configuration_version: String,
        accepts_provisional: bool,
        allow_provisional: bool,
    },
    BeginProcessorIfCurrent {
        schema: String,
        expected_epoch: String,
        node_id: String,
        expected_node_version: String,
        processor_id: String,
        processor_version: String,
        configuration_version: String,
        accepts_provisional: bool,
        allow_provisional: bool,
    },
    CompleteProcessor {
        schema: String,
        request_id: String,
        outcome: &'a RawValue,
    },
    CancelProcessor {
        schema: String,
        request_id: String,
    },
    ArtifactView {
        schema: String,
        epoch: String,
        node_id: String,
        processor_id: String,
    },
}

const FIELD_SCHEMA: u32 = 1 << 0;
const FIELD_KIND: u32 = 1 << 1;
const FIELD_CHANGE: u32 = 1 << 2;
const FIELD_SNAPSHOT: u32 = 1 << 3;
const FIELD_NODE_ID: u32 = 1 << 4;
const FIELD_RESOURCE_ID: u32 = 1 << 5;
const FIELD_PROCESSOR_ID: u32 = 1 << 6;
const FIELD_PROCESSOR_VERSION: u32 = 1 << 7;
const FIELD_CONFIGURATION_VERSION: u32 = 1 << 8;
const FIELD_ACCEPTS_PROVISIONAL: u32 = 1 << 9;
const FIELD_ALLOW_PROVISIONAL: u32 = 1 << 10;
const FIELD_REQUEST_ID: u32 = 1 << 11;
const FIELD_OUTCOME: u32 = 1 << 12;
const FIELD_EPOCH: u32 = 1 << 13;
const FIELD_EXPECTED_EPOCH: u32 = 1 << 14;
const FIELD_EXPECTED_NODE_VERSION: u32 = 1 << 15;
const COMMON_FIELDS: u32 = FIELD_SCHEMA | FIELD_KIND;

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum ReducerField {
    Schema,
    Kind,
    Change,
    Snapshot,
    NodeId,
    ResourceId,
    ProcessorId,
    ProcessorVersion,
    ConfigurationVersion,
    AcceptsProvisional,
    AllowProvisional,
    RequestId,
    Outcome,
    Epoch,
    ExpectedEpoch,
    ExpectedNodeVersion,
    #[serde(other)]
    Unknown,
}

impl<'de> Deserialize<'de> for ReducerCommand<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(ReducerCommandVisitor)
    }
}

struct ReducerCommandVisitor;

impl<'de> Visitor<'de> for ReducerCommandVisitor {
    type Value = ReducerCommand<'de>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a versioned mdstream reducer command")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut seen = 0_u32;
        let mut schema = None;
        let mut kind = None;
        let mut change = None;
        let mut snapshot = None;
        let mut node_id = None;
        let mut resource_id = None;
        let mut processor_id = None;
        let mut processor_version = None;
        let mut configuration_version = None;
        let mut accepts_provisional = None;
        let mut allow_provisional = None;
        let mut request_id = None;
        let mut outcome = None;
        let mut epoch = None;
        let mut expected_epoch = None;
        let mut expected_node_version = None;

        while let Some(field) = map.next_key()? {
            match field {
                ReducerField::Schema => {
                    mark_field::<A::Error>(&mut seen, FIELD_SCHEMA, "schema")?;
                    schema = Some(map.next_value()?);
                }
                ReducerField::Kind => {
                    mark_field::<A::Error>(&mut seen, FIELD_KIND, "kind")?;
                    kind = Some(map.next_value()?);
                }
                ReducerField::Change => {
                    mark_field::<A::Error>(&mut seen, FIELD_CHANGE, "change")?;
                    change = Some(map.next_value()?);
                }
                ReducerField::Snapshot => {
                    mark_field::<A::Error>(&mut seen, FIELD_SNAPSHOT, "snapshot")?;
                    snapshot = Some(map.next_value()?);
                }
                ReducerField::NodeId => {
                    mark_field::<A::Error>(&mut seen, FIELD_NODE_ID, "node_id")?;
                    node_id = Some(map.next_value()?);
                }
                ReducerField::ResourceId => {
                    mark_field::<A::Error>(&mut seen, FIELD_RESOURCE_ID, "resource_id")?;
                    resource_id = Some(map.next_value()?);
                }
                ReducerField::ProcessorId => {
                    mark_field::<A::Error>(&mut seen, FIELD_PROCESSOR_ID, "processor_id")?;
                    processor_id = Some(map.next_value()?);
                }
                ReducerField::ProcessorVersion => {
                    mark_field::<A::Error>(
                        &mut seen,
                        FIELD_PROCESSOR_VERSION,
                        "processor_version",
                    )?;
                    processor_version = Some(map.next_value()?);
                }
                ReducerField::ConfigurationVersion => {
                    mark_field::<A::Error>(
                        &mut seen,
                        FIELD_CONFIGURATION_VERSION,
                        "configuration_version",
                    )?;
                    configuration_version = Some(map.next_value()?);
                }
                ReducerField::AcceptsProvisional => {
                    mark_field::<A::Error>(
                        &mut seen,
                        FIELD_ACCEPTS_PROVISIONAL,
                        "accepts_provisional",
                    )?;
                    accepts_provisional = Some(map.next_value()?);
                }
                ReducerField::AllowProvisional => {
                    mark_field::<A::Error>(
                        &mut seen,
                        FIELD_ALLOW_PROVISIONAL,
                        "allow_provisional",
                    )?;
                    allow_provisional = Some(map.next_value()?);
                }
                ReducerField::RequestId => {
                    mark_field::<A::Error>(&mut seen, FIELD_REQUEST_ID, "request_id")?;
                    request_id = Some(map.next_value()?);
                }
                ReducerField::Outcome => {
                    mark_field::<A::Error>(&mut seen, FIELD_OUTCOME, "outcome")?;
                    outcome = Some(map.next_value()?);
                }
                ReducerField::Epoch => {
                    mark_field::<A::Error>(&mut seen, FIELD_EPOCH, "epoch")?;
                    epoch = Some(map.next_value()?);
                }
                ReducerField::ExpectedEpoch => {
                    mark_field::<A::Error>(&mut seen, FIELD_EXPECTED_EPOCH, "expected_epoch")?;
                    expected_epoch = Some(map.next_value()?);
                }
                ReducerField::ExpectedNodeVersion => {
                    mark_field::<A::Error>(
                        &mut seen,
                        FIELD_EXPECTED_NODE_VERSION,
                        "expected_node_version",
                    )?;
                    expected_node_version = Some(map.next_value()?);
                }
                ReducerField::Unknown => {
                    return Err(de::Error::custom("unknown reducer command field"));
                }
            }
        }

        let schema = schema.ok_or_else(|| de::Error::missing_field("schema"))?;
        let kind: String = kind.ok_or_else(|| de::Error::missing_field("kind"))?;
        match kind.as_str() {
            "apply_change" => {
                ensure_fields::<A::Error>(seen, COMMON_FIELDS | FIELD_CHANGE, &kind)?;
                Ok(ReducerCommand::ApplyChange {
                    schema,
                    change: change.ok_or_else(|| de::Error::missing_field("change"))?,
                })
            }
            "recover_snapshot" => {
                ensure_fields::<A::Error>(seen, COMMON_FIELDS | FIELD_SNAPSHOT, &kind)?;
                Ok(ReducerCommand::RecoverSnapshot {
                    schema,
                    snapshot: snapshot.ok_or_else(|| de::Error::missing_field("snapshot"))?,
                })
            }
            "snapshot" => {
                ensure_fields::<A::Error>(seen, COMMON_FIELDS, &kind)?;
                Ok(ReducerCommand::Snapshot { schema })
            }
            "node_view" => {
                ensure_fields::<A::Error>(seen, COMMON_FIELDS | FIELD_NODE_ID, &kind)?;
                Ok(ReducerCommand::NodeView {
                    schema,
                    node_id: node_id.ok_or_else(|| de::Error::missing_field("node_id"))?,
                })
            }
            "resource_view" => {
                ensure_fields::<A::Error>(seen, COMMON_FIELDS | FIELD_RESOURCE_ID, &kind)?;
                Ok(ReducerCommand::ResourceView {
                    schema,
                    resource_id: resource_id
                        .ok_or_else(|| de::Error::missing_field("resource_id"))?,
                })
            }
            "pending_source_view" => {
                ensure_fields::<A::Error>(seen, COMMON_FIELDS, &kind)?;
                Ok(ReducerCommand::PendingSourceView { schema })
            }
            "begin_processor" => {
                let required = COMMON_FIELDS
                    | FIELD_NODE_ID
                    | FIELD_PROCESSOR_ID
                    | FIELD_PROCESSOR_VERSION
                    | FIELD_CONFIGURATION_VERSION;
                ensure_fields::<A::Error>(
                    seen,
                    required | FIELD_ACCEPTS_PROVISIONAL | FIELD_ALLOW_PROVISIONAL,
                    &kind,
                )?;
                Ok(ReducerCommand::BeginProcessor {
                    schema,
                    node_id: required_field(node_id, "node_id")?,
                    processor_id: required_field(processor_id, "processor_id")?,
                    processor_version: required_field(processor_version, "processor_version")?,
                    configuration_version: required_field(
                        configuration_version,
                        "configuration_version",
                    )?,
                    accepts_provisional: accepts_provisional.unwrap_or(false),
                    allow_provisional: allow_provisional.unwrap_or(false),
                })
            }
            "begin_processor_if_current" => {
                let required = COMMON_FIELDS
                    | FIELD_EXPECTED_EPOCH
                    | FIELD_NODE_ID
                    | FIELD_EXPECTED_NODE_VERSION
                    | FIELD_PROCESSOR_ID
                    | FIELD_PROCESSOR_VERSION
                    | FIELD_CONFIGURATION_VERSION;
                ensure_fields::<A::Error>(
                    seen,
                    required | FIELD_ACCEPTS_PROVISIONAL | FIELD_ALLOW_PROVISIONAL,
                    &kind,
                )?;
                Ok(ReducerCommand::BeginProcessorIfCurrent {
                    schema,
                    expected_epoch: required_field(expected_epoch, "expected_epoch")?,
                    node_id: required_field(node_id, "node_id")?,
                    expected_node_version: required_field(
                        expected_node_version,
                        "expected_node_version",
                    )?,
                    processor_id: required_field(processor_id, "processor_id")?,
                    processor_version: required_field(processor_version, "processor_version")?,
                    configuration_version: required_field(
                        configuration_version,
                        "configuration_version",
                    )?,
                    accepts_provisional: accepts_provisional.unwrap_or(false),
                    allow_provisional: allow_provisional.unwrap_or(false),
                })
            }
            "complete_processor" => {
                let required = COMMON_FIELDS | FIELD_REQUEST_ID | FIELD_OUTCOME;
                ensure_fields::<A::Error>(seen, required, &kind)?;
                Ok(ReducerCommand::CompleteProcessor {
                    schema,
                    request_id: required_field(request_id, "request_id")?,
                    outcome: required_field(outcome, "outcome")?,
                })
            }
            "cancel_processor" => {
                ensure_fields::<A::Error>(seen, COMMON_FIELDS | FIELD_REQUEST_ID, &kind)?;
                Ok(ReducerCommand::CancelProcessor {
                    schema,
                    request_id: request_id.ok_or_else(|| de::Error::missing_field("request_id"))?,
                })
            }
            "artifact_view" => {
                let required = COMMON_FIELDS | FIELD_EPOCH | FIELD_NODE_ID | FIELD_PROCESSOR_ID;
                ensure_fields::<A::Error>(seen, required, &kind)?;
                Ok(ReducerCommand::ArtifactView {
                    schema,
                    epoch: required_field(epoch, "epoch")?,
                    node_id: required_field(node_id, "node_id")?,
                    processor_id: required_field(processor_id, "processor_id")?,
                })
            }
            _ => Err(de::Error::unknown_variant(
                &kind,
                &[
                    "apply_change",
                    "recover_snapshot",
                    "snapshot",
                    "node_view",
                    "resource_view",
                    "pending_source_view",
                    "begin_processor",
                    "begin_processor_if_current",
                    "complete_processor",
                    "cancel_processor",
                    "artifact_view",
                ],
            )),
        }
    }
}

fn mark_field<E: de::Error>(seen: &mut u32, field: u32, name: &'static str) -> Result<(), E> {
    if *seen & field != 0 {
        Err(de::Error::duplicate_field(name))
    } else {
        *seen |= field;
        Ok(())
    }
}

fn ensure_fields<E: de::Error>(seen: u32, allowed: u32, kind: &str) -> Result<(), E> {
    if seen & !allowed == 0 {
        Ok(())
    } else {
        Err(de::Error::custom(format!(
            "unexpected field for reducer command kind {kind}"
        )))
    }
}

fn required_field<T, E: de::Error>(value: Option<T>, field: &'static str) -> Result<T, E> {
    value.ok_or_else(|| de::Error::missing_field(field))
}

impl ReducerCommand<'_> {
    fn schema(&self) -> &str {
        match self {
            Self::ApplyChange { schema, .. }
            | Self::RecoverSnapshot { schema, .. }
            | Self::Snapshot { schema }
            | Self::NodeView { schema, .. }
            | Self::ResourceView { schema, .. }
            | Self::PendingSourceView { schema }
            | Self::BeginProcessor { schema, .. }
            | Self::BeginProcessorIfCurrent { schema, .. }
            | Self::CompleteProcessor { schema, .. }
            | Self::CancelProcessor { schema, .. }
            | Self::ArtifactView { schema, .. } => schema,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub(crate) enum ProcessorCompletion {
    Text {
        protocol: String,
        media_type: String,
        text: String,
    },
    Binary {
        protocol: String,
        media_type: String,
        bytes: Vec<u8>,
    },
    Failure {
        #[serde(deserialize_with = "deserialize_processor_failure_code")]
        code: ProcessorFailureCode,
        message: String,
    },
}

fn deserialize_processor_failure_code<'de, D>(
    deserializer: D,
) -> Result<ProcessorFailureCode, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    value.parse().map_err(de::Error::custom)
}

pub(crate) fn decode_engine_command(
    bytes: &[u8],
    max_bytes: usize,
) -> Result<EngineCommand, BindingError> {
    check_command_size(bytes, max_bytes)?;
    let command: EngineCommand = serde_json::from_slice(bytes)
        .map_err(|error| BindingError::command(format!("invalid engine command: {error}")))?;
    ensure_schema(command.schema())?;
    Ok(command)
}

pub(crate) fn decode_reducer_command(
    bytes: &[u8],
    max_bytes: usize,
) -> Result<ReducerCommand<'_>, BindingError> {
    check_command_size(bytes, max_bytes)?;
    let command: ReducerCommand = serde_json::from_slice(bytes)
        .map_err(|error| BindingError::command(format!("invalid reducer command: {error}")))?;
    ensure_schema(command.schema())?;
    Ok(command)
}

pub(crate) fn decode_processor_completion(
    value: &RawValue,
    max_bytes: usize,
) -> Result<ProcessorCompletion, BindingError> {
    check_size(
        "bindings.processor_completion_bytes",
        value.get().as_bytes(),
        max_bytes,
    )?;
    serde_json::from_str(value.get())
        .map_err(|error| BindingError::command(format!("invalid processor completion: {error}")))
}

pub(crate) fn parse_decimal_id<T>(value: &str, field: &'static str) -> Result<T, BindingError>
where
    T: FromStr<Err = DecimalIdError>,
{
    value.parse().map_err(|error| {
        let requirement = match error {
            DecimalIdError::Overflow => "exceeds its supported integer range",
            DecimalIdError::Empty | DecimalIdError::InvalidSyntax | DecimalIdError::LeadingZero => {
                "must be a canonical unsigned decimal string"
            }
        };
        BindingError::new(
            BindingStatus::InvalidArgument,
            "bindings.decimal_id",
            format!("{field} {requirement}"),
        )
    })
}

pub(crate) fn processing_policy(allow_provisional: bool) -> ProcessingPolicy {
    if allow_provisional {
        ProcessingPolicy::AllowProvisional
    } else {
        ProcessingPolicy::StableOnly
    }
}

fn check_command_size(bytes: &[u8], max_bytes: usize) -> Result<(), BindingError> {
    check_size("bindings.command_bytes", bytes, max_bytes)
}

fn ensure_schema(schema: &str) -> Result<(), BindingError> {
    if schema == BINDING_SCHEMA {
        Ok(())
    } else {
        Err(BindingError::new(
            BindingStatus::UnsupportedSchema,
            "bindings.unsupported_command_schema",
            format!("unsupported binding command schema: {schema}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = "mdstream.bindings/0.4";

    #[test]
    fn reducer_commands_borrow_large_payload_fields_and_cover_every_variant() {
        let apply = format!(
            r#"{{"schema":"{SCHEMA}","kind":"apply_change","change":{{"nested":[1,2,3]}}}}"#
        );
        match decode_reducer_command(apply.as_bytes(), usize::MAX).unwrap() {
            ReducerCommand::ApplyChange { change, .. } => {
                assert_eq!(change.get(), r#"{"nested":[1,2,3]}"#);
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let snapshot = format!(
            r#"{{"schema":"{SCHEMA}","kind":"recover_snapshot","snapshot":{{"digest":"x"}}}}"#
        );
        assert!(matches!(
            decode_reducer_command(snapshot.as_bytes(), usize::MAX).unwrap(),
            ReducerCommand::RecoverSnapshot { .. }
        ));

        for command in [
            format!(r#"{{"schema":"{SCHEMA}","kind":"snapshot"}}"#),
            format!(r#"{{"schema":"{SCHEMA}","kind":"node_view","node_id":"1"}}"#),
            format!(r#"{{"schema":"{SCHEMA}","kind":"resource_view","resource_id":"2"}}"#),
            format!(r#"{{"schema":"{SCHEMA}","kind":"pending_source_view"}}"#),
            format!(
                r#"{{"schema":"{SCHEMA}","kind":"begin_processor","node_id":"1","processor_id":"test.echo","processor_version":"v1","configuration_version":"default"}}"#
            ),
            format!(
                r#"{{"schema":"{SCHEMA}","kind":"begin_processor_if_current","expected_epoch":"1","node_id":"1","expected_node_version":"node:v1","processor_id":"test.echo","processor_version":"v1","configuration_version":"default"}}"#
            ),
            format!(
                r#"{{"schema":"{SCHEMA}","kind":"complete_processor","request_id":"1","outcome":{{"kind":"failure","code":"cancelled","message":"stop"}}}}"#
            ),
            format!(r#"{{"schema":"{SCHEMA}","kind":"cancel_processor","request_id":"1"}}"#),
            format!(
                r#"{{"schema":"{SCHEMA}","kind":"artifact_view","epoch":"1","node_id":"1","processor_id":"test.echo"}}"#
            ),
        ] {
            decode_reducer_command(command.as_bytes(), usize::MAX).unwrap();
        }
    }

    #[test]
    fn reducer_commands_reject_ambiguous_or_incomplete_envelopes() {
        for invalid in [
            format!(r#"{{"schema":"{SCHEMA}","kind":"node_view"}}"#),
            format!(r#"{{"schema":"{SCHEMA}","kind":"snapshot","node_id":"1"}}"#),
            format!(r#"{{"schema":"{SCHEMA}","kind":"snapshot","schema":"{SCHEMA}"}}"#),
            format!(r#"{{"schema":"{SCHEMA}","kind":"snapshot","extra":true}}"#),
            format!(r#"{{"schema":"{SCHEMA}","kind":"unknown"}}"#),
        ] {
            assert_eq!(
                decode_reducer_command(invalid.as_bytes(), usize::MAX)
                    .unwrap_err()
                    .status(),
                BindingStatus::Command
            );
        }
    }

    #[test]
    fn processor_completion_is_size_checked_before_materialization() {
        let raw = serde_json::value::to_raw_value(&serde_json::json!({
            "kind": "text",
            "protocol": "test.echo/1",
            "media_type": "text/plain",
            "text": "payload"
        }))
        .unwrap();
        assert_eq!(
            decode_processor_completion(&raw, raw.get().len() - 1)
                .unwrap_err()
                .status(),
            BindingStatus::ResourceLimit
        );
        assert!(matches!(
            decode_processor_completion(&raw, raw.get().len()).unwrap(),
            ProcessorCompletion::Text { text, .. } if text == "payload"
        ));
    }
}
