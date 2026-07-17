// ignore_for_file: public_member_api_docs

import 'dart:typed_data';

import 'errors.dart';

/// Current JSON schema emitted by the binding facade.
const bindingSchema = 'mdstream.bindings/0.4';

/// Current JSON schema accepted for session options and commands.
const bindingOptionsSchema = 'mdstream.bindings-options/0.4';

/// Stable C ABI version implemented by this package.
const mdstreamAbiVersion = 1;

/// Opaque unsigned identifiers and counters remain decimal strings in Dart.
typedef DecimalCounter = String;
typedef Epoch = String;
typedef Sequence = String;
typedef SourceCursor = String;
typedef RequestGeneration = String;
typedef NodeId = String;
typedef ResourceId = String;
typedef ChangeId = String;
typedef NodeVersion = String;
typedef ResourceVersion = String;
typedef StructureVersion = String;
typedef ProcessorInputVersion = String;

/// Opaque canonical change bytes produced by the native engine.
///
/// The Dart binding can copy and transport this value but deliberately exposes
/// no canonical reducer operations.
final class CanonicalChangeBytes {
  CanonicalChangeBytes(List<int> bytes) : _bytes = _copyOctets(bytes);

  final Uint8List _bytes;

  /// Returns a defensive copy suitable for an FFI call.
  Uint8List get bytes => Uint8List.fromList(_bytes);

  /// Number of encoded bytes.
  int get byteLength => _bytes.length;
}

/// Opaque canonical recovery snapshot produced by native mdstream.
final class CanonicalSnapshotBytes {
  CanonicalSnapshotBytes(List<int> bytes) : _bytes = _copyOctets(bytes);

  final Uint8List _bytes;

  /// Returns a defensive copy suitable for an FFI call.
  Uint8List get bytes => Uint8List.fromList(_bytes);

  /// Number of encoded bytes.
  int get byteLength => _bytes.length;
}

/// Payload discriminants frozen by `mdstream.h` ABI version 1.
enum BindingPayloadKind {
  change(1, 'change'),
  snapshot(2, 'snapshot'),
  reducerUpdate(3, 'reducer_update'),
  nodeView(4, 'node_view'),
  resourceView(5, 'resource_view'),
  processorRequest(6, 'processor_request'),
  processorCompletion(7, 'processor_completion'),
  artifactChange(8, 'artifact_change'),
  artifactView(9, 'artifact_view');

  const BindingPayloadKind(this.value, this.viewKind);

  /// Numeric discriminant transported by the C ABI.
  final int value;

  /// Expected `kind` field for JSON binding views.
  final String viewKind;

  /// Resolves a numeric C ABI discriminant.
  static BindingPayloadKind fromValue(int value) {
    for (final kind in values) {
      if (kind.value == value) {
        return kind;
      }
    }
    throw MdstreamException(
      'unknown mdstream payload kind $value',
      status: 12,
      statusName: 'MDSTREAM_INTERNAL_ERROR',
      detailCode: 'bindings.unknown_payload_kind',
    );
  }
}

/// Status discriminants frozen by `mdstream.h` ABI version 1.
enum BindingStatus {
  ok(0, 'MDSTREAM_OK'),
  invalidArgument(1, 'MDSTREAM_INVALID_ARGUMENT'),
  utf8Error(2, 'MDSTREAM_UTF8_ERROR'),
  optionsError(3, 'MDSTREAM_OPTIONS_ERROR'),
  commandError(4, 'MDSTREAM_COMMAND_ERROR'),
  unsupportedSchema(5, 'MDSTREAM_UNSUPPORTED_SCHEMA'),
  terminal(6, 'MDSTREAM_TERMINAL'),
  engineError(7, 'MDSTREAM_ENGINE_ERROR'),
  protocolError(8, 'MDSTREAM_PROTOCOL_ERROR'),
  needsSnapshot(9, 'MDSTREAM_NEEDS_SNAPSHOT'),
  processorError(10, 'MDSTREAM_PROCESSOR_ERROR'),
  resourceLimitExceeded(11, 'MDSTREAM_RESOURCE_LIMIT_EXCEEDED'),
  internalError(12, 'MDSTREAM_INTERNAL_ERROR'),
  panic(13, 'MDSTREAM_PANIC');

  const BindingStatus(this.value, this.statusName);

  /// Numeric status transported by the C ABI.
  final int value;

  /// Stable symbolic name used by native error envelopes.
  final String statusName;

  /// Resolves a numeric C ABI status.
  static BindingStatus fromValue(int value) {
    for (final status in values) {
      if (status.value == value) {
        return status;
      }
    }
    throw MdstreamException(
      'unknown mdstream status $value',
      status: 12,
      statusName: 'MDSTREAM_INTERNAL_ERROR',
      detailCode: 'bindings.unknown_status',
    );
  }
}

final _decimalPattern = RegExp(r'^(0|[1-9][0-9]*)$');

/// Validates the canonical unsigned decimal representation used on the wire.
String requireDecimalString(Object? value, String field) {
  if (value is! String || !_decimalPattern.hasMatch(value)) {
    throw invalidBindingPayload(
      '$field must be a canonical unsigned decimal string',
    );
  }
  return value;
}

/// Creates a consistently typed failure for malformed native binding output.
MdstreamException invalidBindingPayload(String message, [Object? cause]) =>
    MdstreamException(
      message,
      status: BindingStatus.internalError.value,
      statusName: BindingStatus.internalError.statusName,
      detailCode: 'bindings.invalid_payload',
      cause: cause,
    );

Uint8List _copyOctets(List<int> bytes) {
  for (final byte in bytes) {
    if (byte < 0 || byte > 255) {
      throw RangeError.range(byte, 0, 255, 'byte');
    }
  }
  return Uint8List.fromList(bytes);
}
