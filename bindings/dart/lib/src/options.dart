/// A configured custom block recognized by the mdstream parser.
final class MdstreamCustomBlock {
  /// Creates a custom block descriptor.
  const MdstreamCustomBlock({
    required this.namespace,
    required this.name,
    this.opaque = false,
    this.caseInsensitive = false,
  });

  /// Namespace used to avoid collisions between host extensions.
  final String namespace;

  /// Block name within [namespace].
  final String name;

  /// Whether the parser must leave the block body opaque.
  final bool opaque;

  /// Whether block-name matching is case insensitive.
  final bool caseInsensitive;

  Map<String, Object> _toJson() {
    if (namespace.isEmpty || name.isEmpty) {
      throw ArgumentError('custom block namespace and name must not be empty');
    }
    return {
      'namespace': namespace,
      'name': name,
      'opaque': opaque,
      'case_insensitive': caseInsensitive,
    };
  }
}

/// Resource limits and parser extensions used by one native session.
///
/// Limit values are unsigned decimal strings so the complete Rust integer
/// domain is preserved on every supported Dart runtime.
final class MdstreamSessionOptions {
  /// Creates validated session options.
  MdstreamSessionOptions({
    Map<String, String> protocol = const {},
    Map<String, String> engine = const {},
    Map<String, String> processor = const {},
    Map<String, String> wire = const {},
    List<MdstreamCustomBlock> customBlocks = const [],
  }) : protocol = _validatedLimits(protocol, 'protocol'),
       engine = _validatedLimits(engine, 'engine'),
       processor = _validatedLimits(processor, 'processor'),
       wire = _validatedLimits(wire, 'wire'),
       customBlocks = List.unmodifiable(customBlocks) {
    for (final block in this.customBlocks) {
      block._toJson();
    }
  }

  /// Protocol-level limits encoded with snake_case option names.
  final Map<String, String> protocol;

  /// Engine-level limits encoded with snake_case option names.
  final Map<String, String> engine;

  /// Processor-host limits encoded with snake_case option names.
  final Map<String, String> processor;

  /// Binding-wire limits encoded with snake_case option names.
  final Map<String, String> wire;

  /// Custom blocks sealed before the first input is appended.
  final List<MdstreamCustomBlock> customBlocks;

  /// Encodes these options using the native binding-options [schema].
  Map<String, Object> toJson(String schema) {
    if (schema.isEmpty) {
      throw ArgumentError.value(schema, 'schema', 'must not be empty');
    }
    return {
      'schema': schema,
      if (protocol.isNotEmpty) 'protocol': protocol,
      if (engine.isNotEmpty) 'engine': engine,
      if (processor.isNotEmpty) 'processor': processor,
      if (wire.isNotEmpty) 'wire': wire,
      if (customBlocks.isNotEmpty)
        'custom_blocks': customBlocks.map((block) => block._toJson()).toList(),
    };
  }
}

final RegExp _decimalPattern = RegExp(r'^(0|[1-9][0-9]*)$');
final RegExp _optionNamePattern = RegExp(r'^[a-z][a-z0-9_]*$');

Map<String, String> _validatedLimits(Map<String, String> source, String group) {
  final result = <String, String>{};
  for (final MapEntry(:key, :value) in source.entries) {
    if (!_optionNamePattern.hasMatch(key)) {
      throw ArgumentError.value(key, '$group option', 'must be snake_case');
    }
    if (!_decimalPattern.hasMatch(value)) {
      throw ArgumentError.value(
        value,
        '$group.$key',
        'must be an unsigned canonical decimal string',
      );
    }
    result[key] = value;
  }
  return Map.unmodifiable(result);
}
