import 'dart:typed_data';

import 'errors.dart';

/// Current JSON schema emitted by the binding facade.
const bindingSchema = 'mdstream.bindings/0.4';

/// Current JSON schema accepted for session options and commands.
const bindingOptionsSchema = 'mdstream.bindings-options/0.4';

/// Frozen transition-facts schema carried inside reducer updates.
const transitionSchema = 'mdstream.transitions/1';

/// Stable C ABI version implemented by this package.
const mdstreamAbiVersion = 1;

/// An exact unsigned 64-bit counter transported as canonical decimal text.
extension type const DecimalCounter._(String value) implements String {
  /// Parses and validates a public counter value.
  factory DecimalCounter.parse(String value) => DecimalCounter._(
    _validateDecimalInput(value, 'decimal counter', _maxU64),
  );
}

/// Converts a trusted package-internal non-negative counter without reparsing.
///
/// Native transport and Dart-side metrics call this only after counting values
/// in non-negative integer fields. Public decimal text must use
/// [DecimalCounter.parse] instead.
DecimalCounter decimalCounterFromTrustedInt(int value) {
  if (value < 0) {
    throw RangeError.value(value, 'value', 'must be non-negative');
  }
  return DecimalCounter._(value.toString());
}

/// A document epoch transported as canonical unsigned 64-bit decimal text.
extension type const Epoch._(String value) implements String {
  /// Parses and validates a public epoch value.
  factory Epoch.parse(String value) =>
      Epoch._(_validateDecimalInput(value, 'epoch', _maxU64));
}

/// A change sequence transported as canonical unsigned 64-bit decimal text.
extension type const Sequence._(String value) implements String {
  /// Parses and validates a public sequence value.
  factory Sequence.parse(String value) =>
      Sequence._(_validateDecimalInput(value, 'sequence', _maxU64));
}

/// A source byte cursor transported as canonical unsigned decimal text.
extension type const SourceCursor._(String value) implements String {
  /// Parses and validates a public source cursor.
  factory SourceCursor.parse(String value) =>
      SourceCursor._(_validateDecimalInput(value, 'source cursor', _maxU64));
}

/// A processor request generation transported as canonical decimal text.
extension type const RequestGeneration._(String value) implements String {
  /// Parses and validates a public request generation.
  factory RequestGeneration.parse(String value) => RequestGeneration._(
    _validateDecimalInput(value, 'request generation', _maxU64),
  );
}

/// A reducer continuity generation transported as canonical decimal text.
extension type const ContinuityGeneration._(String value) implements String {
  /// Parses and validates a public continuity generation.
  factory ContinuityGeneration.parse(String value) => ContinuityGeneration._(
    _validateDecimalInput(value, 'continuity generation', _maxU64),
  );
}

/// A content-node identity transported as canonical unsigned 128-bit text.
extension type const NodeId._(String value) implements String {
  /// Parses and validates a public node identity.
  factory NodeId.parse(String value) =>
      NodeId._(_validateDecimalInput(value, 'node id', _maxU128));
}

/// A semantic-resource identity transported as unsigned 128-bit text.
extension type const ResourceId._(String value) implements String {
  /// Parses and validates a public resource identity.
  factory ResourceId.parse(String value) =>
      ResourceId._(_validateDecimalInput(value, 'resource id', _maxU128));
}

/// An opaque protocol change identity.
extension type const ChangeId._(String value) implements String {
  /// Parses and validates a public change identity.
  factory ChangeId.parse(String value) =>
      ChangeId._(_validateOpaqueInput(value, 'change id'));
}

/// An opaque semantic node version.
extension type const NodeVersion._(String value) implements String {
  /// Parses and validates a public node version.
  factory NodeVersion.parse(String value) =>
      NodeVersion._(_validateOpaqueInput(value, 'node version'));
}

/// An opaque semantic resource version.
extension type const ResourceVersion._(String value) implements String {
  /// Parses and validates a public resource version.
  factory ResourceVersion.parse(String value) =>
      ResourceVersion._(_validateOpaqueInput(value, 'resource version'));
}

/// An opaque ordered-child-list version.
extension type const StructureVersion._(String value) implements String {
  /// Parses and validates a public structure version.
  factory StructureVersion.parse(String value) =>
      StructureVersion._(_validateOpaqueInput(value, 'structure version'));
}

/// An opaque version of a processor's canonical input.
extension type const ProcessorInputVersion._(String value) implements String {
  /// Parses and validates a public processor input version.
  factory ProcessorInputVersion.parse(String value) => ProcessorInputVersion._(
    _validateOpaqueInput(value, 'processor input version'),
  );
}

/// Opaque canonical change bytes produced by the native engine.
///
/// The Dart binding can copy and transport this value but deliberately exposes
/// no canonical reducer operations.
final class CanonicalChangeBytes {
  /// Copies [bytes] into an immutable canonical change container.
  CanonicalChangeBytes(List<int> bytes)
    : _bytes = _copyOctets(bytes).asUnmodifiableView();

  CanonicalChangeBytes._fromOwned(Uint8List bytes)
    : _bytes = bytes.asUnmodifiableView();

  final Uint8List _bytes;

  /// Returns a defensive copy suitable for an FFI call.
  Uint8List get bytes => Uint8List.fromList(_bytes);

  /// Number of encoded bytes.
  int get byteLength => _bytes.length;
}

/// Opaque canonical recovery snapshot produced by native mdstream.
final class CanonicalSnapshotBytes {
  /// Copies [bytes] into an immutable canonical snapshot container.
  CanonicalSnapshotBytes(List<int> bytes)
    : _bytes = _copyOctets(bytes).asUnmodifiableView();

  CanonicalSnapshotBytes._fromOwned(Uint8List bytes)
    : _bytes = bytes.asUnmodifiableView();

  final Uint8List _bytes;

  /// Returns a defensive copy suitable for an FFI call.
  Uint8List get bytes => Uint8List.fromList(_bytes);

  /// Number of encoded bytes.
  int get byteLength => _bytes.length;
}

/// Package-internal ownership transfer for native binding payloads.
CanonicalChangeBytes canonicalChangeBytesFromOwned(Uint8List bytes) =>
    CanonicalChangeBytes._fromOwned(bytes);

/// Package-internal readonly view used by synchronous FFI calls.
Uint8List canonicalChangeBytesView(CanonicalChangeBytes value) => value._bytes;

/// Package-internal ownership transfer for native snapshot payloads.
CanonicalSnapshotBytes canonicalSnapshotBytesFromOwned(Uint8List bytes) =>
    CanonicalSnapshotBytes._fromOwned(bytes);

/// Package-internal readonly view used by synchronous FFI calls.
Uint8List canonicalSnapshotBytesView(CanonicalSnapshotBytes value) =>
    value._bytes;

/// Payload discriminants frozen by `mdstream.h` ABI version 1.
enum BindingPayloadKind {
  /// A canonical document change emitted by the engine.
  change(1, 'change'),

  /// A canonical document snapshot used for reducer recovery.
  snapshot(2, 'snapshot'),

  /// A canonical reducer state update.
  reducerUpdate(3, 'reducer_update'),

  /// A point-in-time node projection.
  nodeView(4, 'node_view'),

  /// A point-in-time semantic resource projection.
  resourceView(5, 'resource_view'),

  /// A request to run a registered content processor.
  processorRequest(6, 'processor_request'),

  /// A terminal processor request outcome.
  processorCompletion(7, 'processor_completion'),

  /// A change to a retained processor artifact.
  artifactChange(8, 'artifact_change'),

  /// A point-in-time retained artifact projection.
  artifactView(9, 'artifact_view'),

  /// A bounded view of source not yet committed to stable content.
  pendingSourceView(10, 'pending_source_view');

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
  /// The native operation completed successfully.
  ok(0, 'MDSTREAM_OK'),

  /// A public argument failed boundary validation.
  invalidArgument(1, 'MDSTREAM_INVALID_ARGUMENT'),

  /// Input bytes were not valid UTF-8.
  utf8Error(2, 'MDSTREAM_UTF8_ERROR'),

  /// Session options were invalid or unsupported.
  optionsError(3, 'MDSTREAM_OPTIONS_ERROR'),

  /// A command payload was malformed or invalid for the current state.
  commandError(4, 'MDSTREAM_COMMAND_ERROR'),

  /// A payload declared an unsupported schema version.
  unsupportedSchema(5, 'MDSTREAM_UNSUPPORTED_SCHEMA'),

  /// The stream has reached a terminal state for this operation.
  terminal(6, 'MDSTREAM_TERMINAL'),

  /// The native engine rejected or failed the operation.
  engineError(7, 'MDSTREAM_ENGINE_ERROR'),

  /// Canonical protocol validation failed.
  protocolError(8, 'MDSTREAM_PROTOCOL_ERROR'),

  /// Incremental continuity was lost and a snapshot is required.
  needsSnapshot(9, 'MDSTREAM_NEEDS_SNAPSHOT'),

  /// Processor lifecycle validation or execution failed.
  processorError(10, 'MDSTREAM_PROCESSOR_ERROR'),

  /// A configured resource or payload budget was exceeded.
  resourceLimitExceeded(11, 'MDSTREAM_RESOURCE_LIMIT_EXCEEDED'),

  /// An invariant failed inside the native binding implementation.
  internalError(12, 'MDSTREAM_INTERNAL_ERROR'),

  /// Native code panicked across a protected ABI boundary.
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
final _opaqueIdentifierPattern = RegExp(r'^[A-Za-z0-9._:-]{1,128}$');
const _maxU64 = '18446744073709551615';
const _maxU128 = '340282366920938463463374607431768211455';

/// Validates a native-output decimal whose Rust domain is `u64`.
String requireDecimalString(Object? value, String field) =>
    decodeDecimalU64(value, field);

/// Decodes a package-internal unsigned 64-bit decimal wire value.
String decodeDecimalU64(Object? value, String field) =>
    _validateDecimal(value, field, _maxU64, invalidBindingPayload);

/// Decodes a package-internal unsigned 128-bit decimal wire value.
String decodeDecimalU128(Object? value, String field) =>
    _validateDecimal(value, field, _maxU128, invalidBindingPayload);

/// Validates an unsigned 64-bit decimal at a public command boundary.
String validateDecimalU64Input(Object? value, String field) =>
    _validateDecimal(value, field, _maxU64, _invalidDecimalInput);

/// Validates an unsigned 128-bit decimal at a public command boundary.
String validateDecimalU128Input(Object? value, String field) =>
    _validateDecimal(value, field, _maxU128, _invalidDecimalInput);

/// Validates an opaque identifier at a public command boundary.
String validateOpaqueIdentifierInput(Object? value, String field) {
  if (value is! String) {
    throw _invalidOpaqueInput(
      '$field must be a 1-128 byte ASCII opaque identifier',
    );
  }
  return _validateOpaqueInput(value, field);
}

/// Decodes a package-internal exact counter from native output.
DecimalCounter decodeDecimalCounter(Object? value, String field) =>
    DecimalCounter._(decodeDecimalU64(value, field));

/// Decodes a package-internal epoch from native output.
Epoch decodeEpoch(Object? value, String field) =>
    Epoch._(decodeDecimalU64(value, field));

/// Decodes a package-internal sequence from native output.
Sequence decodeSequence(Object? value, String field) =>
    Sequence._(decodeDecimalU64(value, field));

/// Decodes a package-internal source cursor from native output.
SourceCursor decodeSourceCursor(Object? value, String field) =>
    SourceCursor._(decodeDecimalU64(value, field));

/// Decodes a package-internal processor request generation.
RequestGeneration decodeRequestGeneration(Object? value, String field) =>
    RequestGeneration._(decodeDecimalU64(value, field));

/// Decodes a package-internal continuity generation.
ContinuityGeneration decodeContinuityGeneration(Object? value, String field) =>
    ContinuityGeneration._(decodeDecimalU64(value, field));

/// Decodes a package-internal node identity from native output.
NodeId decodeNodeId(Object? value, String field) =>
    NodeId._(decodeDecimalU128(value, field));

/// Decodes a package-internal resource identity from native output.
ResourceId decodeResourceId(Object? value, String field) =>
    ResourceId._(decodeDecimalU128(value, field));

/// Decodes a package-internal change identity from native output.
ChangeId decodeChangeId(Object? value, String field) =>
    ChangeId._(_decodeOpaque(value, field));

/// Decodes a package-internal node version from native output.
NodeVersion decodeNodeVersion(Object? value, String field) =>
    NodeVersion._(_decodeOpaque(value, field));

/// Decodes a package-internal resource version from native output.
ResourceVersion decodeResourceVersion(Object? value, String field) =>
    ResourceVersion._(_decodeOpaque(value, field));

/// Decodes a package-internal child-list version from native output.
StructureVersion decodeStructureVersion(Object? value, String field) =>
    StructureVersion._(_decodeOpaque(value, field));

/// Decodes a package-internal processor input version from native output.
ProcessorInputVersion decodeProcessorInputVersion(
  Object? value,
  String field,
) => ProcessorInputVersion._(_decodeOpaque(value, field));

String _validateDecimal(
  Object? value,
  String field,
  String maximum,
  MdstreamException Function(String message) error,
) {
  if (value is! String ||
      !_decimalPattern.hasMatch(value) ||
      _decimalExceeds(value, maximum)) {
    throw error(
      '$field must be a canonical unsigned decimal string within its supported range',
    );
  }
  return value;
}

bool _decimalExceeds(String value, String maximum) =>
    value.length > maximum.length ||
    (value.length == maximum.length && value.compareTo(maximum) > 0);

MdstreamException _invalidDecimalInput(String message) => MdstreamException(
  message,
  status: BindingStatus.invalidArgument.value,
  statusName: BindingStatus.invalidArgument.statusName,
  detailCode: 'bindings.decimal_id',
);

String _validateDecimalInput(String value, String field, String maximum) =>
    _validateDecimal(value, field, maximum, _invalidDecimalInput);

String _decodeOpaque(Object? value, String field) {
  if (value is! String || !_opaqueIdentifierPattern.hasMatch(value)) {
    throw invalidBindingPayload(
      '$field must be a 1-128 byte ASCII opaque identifier',
    );
  }
  return value;
}

String _validateOpaqueInput(String value, String field) {
  if (!_opaqueIdentifierPattern.hasMatch(value)) {
    throw _invalidOpaqueInput(
      '$field must be a 1-128 byte ASCII opaque identifier',
    );
  }
  return value;
}

MdstreamException _invalidOpaqueInput(String message) => MdstreamException(
  message,
  status: BindingStatus.invalidArgument.value,
  statusName: BindingStatus.invalidArgument.statusName,
  detailCode: 'bindings.opaque_id',
);

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
