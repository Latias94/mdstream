import 'dart:convert';
import 'dart:typed_data';

/// Whether replaying a rejected joined append at retained input boundaries is safe.
enum SplitSafety {
  /// The error is local to an append transaction and may admit a replay.
  retryAtOriginalBoundaries('retry_at_original_boundaries'),

  /// Splitting cannot make the failure admissible or would change semantics.
  notSafe('not_safe');

  const SplitSafety(this.value);

  /// Stable wire representation owned by the Rust core.
  final String value;

  /// Decodes an untrusted wire value conservatively.
  static SplitSafety fromWire(Object? value) {
    for (final splitSafety in values) {
      if (splitSafety.value == value) {
        return splitSafety;
      }
    }
    return notSafe;
  }
}

/// A structured error returned by mdstream or synthesized by the Dart binding.
final class MdstreamException implements Exception {
  /// Creates a structured mdstream error.
  MdstreamException(
    this.message, {
    this.status = 12,
    this.statusName = 'MDSTREAM_INTERNAL_ERROR',
    this.detailCode = 'bindings.dart_error',
    this.schema,
    this.splitSafety = SplitSafety.notSafe,
    this.cause,
  });

  /// Decodes a native JSON error envelope.
  ///
  /// [value] may be an already-decoded object, a JSON string, or UTF-8 bytes.
  /// Invalid envelopes become a typed internal error so callers never have to
  /// handle JSON or UTF-8 exceptions at the FFI boundary.
  factory MdstreamException.fromJson(Object? value, {int? fallbackStatus}) {
    try {
      final Object? decoded;
      if (value is String) {
        decoded = jsonDecode(value);
      } else if (value is Uint8List) {
        decoded = jsonDecode(utf8.decode(value, allowMalformed: false));
      } else if (value is List<int>) {
        decoded = jsonDecode(utf8.decode(value, allowMalformed: false));
      } else {
        decoded = value;
      }
      if (decoded is! Map) {
        throw const FormatException('mdstream error payload must be an object');
      }

      final envelope = <String, Object?>{};
      for (final entry in decoded.entries) {
        if (entry.key is! String) {
          throw const FormatException(
            'mdstream error payload keys must be strings',
          );
        }
        envelope[entry.key as String] = entry.value;
      }

      final message = envelope['message'];
      final status = envelope['status'];
      final statusName = envelope['status_name'];
      final detailCode = envelope['detail_code'];
      final schema = envelope['schema'];
      final splitSafety = envelope['split_safety'];
      return MdstreamException(
        message is String ? message : 'mdstream operation failed',
        status: status is int ? status : (fallbackStatus ?? 12),
        statusName: statusName is String
            ? statusName
            : 'MDSTREAM_INTERNAL_ERROR',
        detailCode: detailCode is String
            ? detailCode
            : 'bindings.invalid_error_payload',
        schema: schema is String ? schema : null,
        splitSafety: SplitSafety.fromWire(splitSafety),
        cause: value,
      );
    } catch (error) {
      return MdstreamException(
        'failed to decode mdstream error payload',
        status: fallbackStatus ?? 12,
        detailCode: 'bindings.invalid_error_payload',
        cause: error,
      );
    }
  }

  /// Decodes fatal UTF-8 JSON bytes returned by the C ABI.
  factory MdstreamException.fromJsonBytes(
    Uint8List bytes, {
    int? fallbackStatus,
  }) => MdstreamException.fromJson(bytes, fallbackStatus: fallbackStatus);

  /// Normalizes an arbitrary Dart or native error into a structured exception.
  factory MdstreamException.fromObject(
    Object? value, {
    int status = 12,
    String statusName = 'MDSTREAM_INTERNAL_ERROR',
    String detailCode = 'bindings.dart_error',
  }) {
    if (value is MdstreamException) {
      return value;
    }
    if (value is Map || value is Uint8List || value is List<int>) {
      return MdstreamException.fromJson(value, fallbackStatus: status);
    }
    return MdstreamException(
      value?.toString() ?? 'mdstream operation failed',
      status: status,
      statusName: statusName,
      detailCode: detailCode,
      cause: value,
    );
  }

  /// Human-readable error text.
  final String message;

  /// Stable numeric status from the C ABI.
  final int status;

  /// Stable symbolic status from the native error envelope.
  final String statusName;

  /// Machine-readable error detail owned by the failing mdstream layer.
  final String detailCode;

  /// Binding schema that encoded the error, when supplied by native code.
  final String? schema;

  /// Rust-owned replay classification, conservatively defaulted for unknown values.
  final SplitSafety splitSafety;

  /// Original error or payload retained for diagnostics.
  final Object? cause;

  @override
  String toString() => 'MdstreamException($statusName/$detailCode): $message';
}
