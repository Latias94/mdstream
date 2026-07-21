/// A configured custom block recognized by the mdstream parser.
final class MdstreamCustomBlock {
  /// Creates a custom block descriptor.
  const MdstreamCustomBlock({
    required this.namespace,
    required this.name,
    this.opaque,
    this.caseInsensitive,
  });

  /// Namespace used to avoid collisions between host extensions.
  final String namespace;

  /// Block name within [namespace].
  final String name;

  /// Whether the parser must leave the block body opaque.
  final bool? opaque;

  /// Whether block-name matching is case insensitive.
  final bool? caseInsensitive;

  Map<String, Object> _toJson() {
    if (namespace.isEmpty || name.isEmpty) {
      throw ArgumentError('custom block namespace and name must not be empty');
    }
    return {
      'namespace': namespace,
      'name': name,
      if (opaque != null) 'opaque': opaque!,
      if (caseInsensitive != null) 'case_insensitive': caseInsensitive!,
    };
  }
}

/// Parser-neutral Content IR and reducer resource limits.
///
/// Values use canonical unsigned decimal strings so the complete Rust integer
/// domain is preserved on every supported Dart runtime.
final class MdstreamProtocolLimits {
  /// Creates validated protocol limits.
  MdstreamProtocolLimits({
    String? maxSourceBytes,
    String? maxNodes,
    String? maxResources,
    String? maxOperations,
    String? maxChangeStructuralItems,
    String? maxDocumentStructuralItems,
    String? maxChildrenPerList,
    String? maxAttributesPerNode,
    String? maxMetadataValueBytes,
    String? maxNodeMetadataBytes,
    String? maxChangeMetadataBytes,
    String? maxDocumentMetadataBytes,
    String? maxTreeDepth,
  }) : maxSourceBytes = _validatedOptionalDecimal(
         maxSourceBytes,
         'protocol.max_source_bytes',
       ),
       maxNodes = _validatedOptionalDecimal(maxNodes, 'protocol.max_nodes'),
       maxResources = _validatedOptionalDecimal(
         maxResources,
         'protocol.max_resources',
       ),
       maxOperations = _validatedOptionalDecimal(
         maxOperations,
         'protocol.max_operations',
       ),
       maxChangeStructuralItems = _validatedOptionalDecimal(
         maxChangeStructuralItems,
         'protocol.max_change_structural_items',
       ),
       maxDocumentStructuralItems = _validatedOptionalDecimal(
         maxDocumentStructuralItems,
         'protocol.max_document_structural_items',
       ),
       maxChildrenPerList = _validatedOptionalDecimal(
         maxChildrenPerList,
         'protocol.max_children_per_list',
       ),
       maxAttributesPerNode = _validatedOptionalDecimal(
         maxAttributesPerNode,
         'protocol.max_attributes_per_node',
       ),
       maxMetadataValueBytes = _validatedOptionalDecimal(
         maxMetadataValueBytes,
         'protocol.max_metadata_value_bytes',
       ),
       maxNodeMetadataBytes = _validatedOptionalDecimal(
         maxNodeMetadataBytes,
         'protocol.max_node_metadata_bytes',
       ),
       maxChangeMetadataBytes = _validatedOptionalDecimal(
         maxChangeMetadataBytes,
         'protocol.max_change_metadata_bytes',
       ),
       maxDocumentMetadataBytes = _validatedOptionalDecimal(
         maxDocumentMetadataBytes,
         'protocol.max_document_metadata_bytes',
       ),
       maxTreeDepth = _validatedOptionalDecimal(
         maxTreeDepth,
         'protocol.max_tree_depth',
       );

  /// Maximum UTF-8 source bytes retained by a session.
  final String? maxSourceBytes;

  /// Maximum nodes in a canonical document.
  final String? maxNodes;

  /// Maximum resources in a canonical document.
  final String? maxResources;

  /// Maximum operations in one canonical change.
  final String? maxOperations;

  /// Maximum structural items carried by one change.
  final String? maxChangeStructuralItems;

  /// Maximum structural items retained by one document.
  final String? maxDocumentStructuralItems;

  /// Maximum children in one ordered child list.
  final String? maxChildrenPerList;

  /// Maximum attributes on one node.
  final String? maxAttributesPerNode;

  /// Maximum UTF-8 bytes in one metadata value.
  final String? maxMetadataValueBytes;

  /// Maximum metadata bytes on one node.
  final String? maxNodeMetadataBytes;

  /// Maximum metadata bytes carried by one change.
  final String? maxChangeMetadataBytes;

  /// Maximum metadata bytes retained by one document.
  final String? maxDocumentMetadataBytes;

  /// Maximum canonical tree depth.
  final String? maxTreeDepth;

  Map<String, String> _toJson() => {
    if (maxSourceBytes != null) 'max_source_bytes': maxSourceBytes!,
    if (maxNodes != null) 'max_nodes': maxNodes!,
    if (maxResources != null) 'max_resources': maxResources!,
    if (maxOperations != null) 'max_operations': maxOperations!,
    if (maxChangeStructuralItems != null)
      'max_change_structural_items': maxChangeStructuralItems!,
    if (maxDocumentStructuralItems != null)
      'max_document_structural_items': maxDocumentStructuralItems!,
    if (maxChildrenPerList != null)
      'max_children_per_list': maxChildrenPerList!,
    if (maxAttributesPerNode != null)
      'max_attributes_per_node': maxAttributesPerNode!,
    if (maxMetadataValueBytes != null)
      'max_metadata_value_bytes': maxMetadataValueBytes!,
    if (maxNodeMetadataBytes != null)
      'max_node_metadata_bytes': maxNodeMetadataBytes!,
    if (maxChangeMetadataBytes != null)
      'max_change_metadata_bytes': maxChangeMetadataBytes!,
    if (maxDocumentMetadataBytes != null)
      'max_document_metadata_bytes': maxDocumentMetadataBytes!,
    if (maxTreeDepth != null) 'max_tree_depth': maxTreeDepth!,
  };
}

/// Compiler work and semantic-state limits.
final class MdstreamCompilerLimits {
  /// Creates validated compiler limits.
  MdstreamCompilerLimits({
    String? maxMarkdownEvents,
    String? maxMarkdownOverlapWork,
    String? maxDefinitions,
    String? maxDefinitionEdges,
    String? maxDefinitionMetadataBytes,
  }) : maxMarkdownEvents = _validatedOptionalDecimal(
         maxMarkdownEvents,
         'compiler.max_markdown_events',
       ),
       maxMarkdownOverlapWork = _validatedOptionalDecimal(
         maxMarkdownOverlapWork,
         'compiler.max_markdown_overlap_work',
       ),
       maxDefinitions = _validatedOptionalDecimal(
         maxDefinitions,
         'compiler.max_definitions',
       ),
       maxDefinitionEdges = _validatedOptionalDecimal(
         maxDefinitionEdges,
         'compiler.max_definition_edges',
       ),
       maxDefinitionMetadataBytes = _validatedOptionalDecimal(
         maxDefinitionMetadataBytes,
         'compiler.max_definition_metadata_bytes',
       );

  /// Maximum retained parser classification events for one compilation.
  ///
  /// This bounds the event set used to classify unresolved footnotes; it is
  /// not a limit on every parser event emitted by the Markdown compiler.
  final String? maxMarkdownEvents;

  /// Maximum overlap comparisons performed while overlaying unresolved
  /// footnotes.
  ///
  /// This is a bounded work counter, not a byte or parser-event budget.
  final String? maxMarkdownOverlapWork;

  /// Maximum definitions retained by the compiler's semantic registry.
  final String? maxDefinitions;

  /// Maximum reverse dependency edges retained for semantic correction.
  final String? maxDefinitionEdges;

  /// Maximum UTF-8 bytes retained by definition keys and values.
  final String? maxDefinitionMetadataBytes;

  Map<String, String> _toJson() => {
    if (maxMarkdownEvents != null) 'max_markdown_events': maxMarkdownEvents!,
    if (maxMarkdownOverlapWork != null)
      'max_markdown_overlap_work': maxMarkdownOverlapWork!,
    if (maxDefinitions != null) 'max_definitions': maxDefinitions!,
    if (maxDefinitionEdges != null) 'max_definition_edges': maxDefinitionEdges!,
    if (maxDefinitionMetadataBytes != null)
      'max_definition_metadata_bytes': maxDefinitionMetadataBytes!,
  };
}

/// Canonical change construction limits for the streaming engine.
final class MdstreamEngineLimits {
  /// Creates validated engine limits.
  MdstreamEngineLimits({String? maxChangeBytes, String? maxTransactionBytes})
    : maxChangeBytes = _validatedOptionalDecimal(
        maxChangeBytes,
        'engine.max_change_bytes',
      ),
      maxTransactionBytes = _validatedOptionalDecimal(
        maxTransactionBytes,
        'engine.max_transaction_bytes',
      );

  /// Maximum encoded bytes represented by one canonical change.
  final String? maxChangeBytes;

  /// Maximum bytes accepted by one engine transaction.
  final String? maxTransactionBytes;

  Map<String, String> _toJson() => {
    if (maxChangeBytes != null) 'max_change_bytes': maxChangeBytes!,
    if (maxTransactionBytes != null)
      'max_transaction_bytes': maxTransactionBytes!,
  };
}

/// Processor scheduling, input, and retained-artifact limits.
final class MdstreamProcessorLimits {
  /// Creates validated processor limits.
  MdstreamProcessorLimits({
    String? maxInputBytes,
    String? maxArtifactBytes,
    String? maxInFlightJobs,
    String? maxInFlightInputBytes,
    String? maxSlots,
    String? maxRetainedArtifacts,
    String? maxRetainedArtifactBytes,
    String? maxErrorBytes,
    String? maxPendingChanges,
    String? maxPendingChangeBytes,
  }) : maxInputBytes = _validatedOptionalDecimal(
         maxInputBytes,
         'processor.max_input_bytes',
       ),
       maxArtifactBytes = _validatedOptionalDecimal(
         maxArtifactBytes,
         'processor.max_artifact_bytes',
       ),
       maxInFlightJobs = _validatedOptionalDecimal(
         maxInFlightJobs,
         'processor.max_in_flight_jobs',
       ),
       maxInFlightInputBytes = _validatedOptionalDecimal(
         maxInFlightInputBytes,
         'processor.max_in_flight_input_bytes',
       ),
       maxSlots = _validatedOptionalDecimal(maxSlots, 'processor.max_slots'),
       maxRetainedArtifacts = _validatedOptionalDecimal(
         maxRetainedArtifacts,
         'processor.max_retained_artifacts',
       ),
       maxRetainedArtifactBytes = _validatedOptionalDecimal(
         maxRetainedArtifactBytes,
         'processor.max_retained_artifact_bytes',
       ),
       maxErrorBytes = _validatedOptionalDecimal(
         maxErrorBytes,
         'processor.max_error_bytes',
       ),
       maxPendingChanges = _validatedOptionalDecimal(
         maxPendingChanges,
         'processor.max_pending_changes',
       ),
       maxPendingChangeBytes = _validatedOptionalDecimal(
         maxPendingChangeBytes,
         'processor.max_pending_change_bytes',
       );

  /// Maximum bytes in one processor input.
  final String? maxInputBytes;

  /// Maximum bytes in one successful artifact.
  final String? maxArtifactBytes;

  /// Maximum concurrently leased processor jobs.
  final String? maxInFlightJobs;

  /// Maximum aggregate input bytes leased concurrently.
  final String? maxInFlightInputBytes;

  /// Maximum processor slots retained by a session.
  final String? maxSlots;

  /// Maximum successful artifacts retained by a session.
  final String? maxRetainedArtifacts;

  /// Maximum aggregate bytes across retained artifacts.
  final String? maxRetainedArtifactBytes;

  /// Maximum bytes in one processor failure message.
  final String? maxErrorBytes;

  /// Maximum queued artifact changes awaiting host delivery.
  final String? maxPendingChanges;

  /// Maximum aggregate bytes across pending artifact changes.
  final String? maxPendingChangeBytes;

  Map<String, String> _toJson() => {
    if (maxInputBytes != null) 'max_input_bytes': maxInputBytes!,
    if (maxArtifactBytes != null) 'max_artifact_bytes': maxArtifactBytes!,
    if (maxInFlightJobs != null) 'max_in_flight_jobs': maxInFlightJobs!,
    if (maxInFlightInputBytes != null)
      'max_in_flight_input_bytes': maxInFlightInputBytes!,
    if (maxSlots != null) 'max_slots': maxSlots!,
    if (maxRetainedArtifacts != null)
      'max_retained_artifacts': maxRetainedArtifacts!,
    if (maxRetainedArtifactBytes != null)
      'max_retained_artifact_bytes': maxRetainedArtifactBytes!,
    if (maxErrorBytes != null) 'max_error_bytes': maxErrorBytes!,
    if (maxPendingChanges != null) 'max_pending_changes': maxPendingChanges!,
    if (maxPendingChangeBytes != null)
      'max_pending_change_bytes': maxPendingChangeBytes!,
  };
}

/// Effective native capacity for a framework adapter's processor scheduler.
final class MdstreamProcessorSchedulerLimits {
  /// Creates effective scheduler limits returned by the native reducer.
  const MdstreamProcessorSchedulerLimits({
    required this.maxInFlightJobs,
    required this.maxQueuedCandidates,
  });

  /// Maximum processor jobs an adapter may dispatch concurrently.
  final int maxInFlightJobs;

  /// Maximum processor candidates an adapter may retain before dispatch.
  final int maxQueuedCandidates;
}

/// Serialization and FFI payload limits at the native binding boundary.
final class MdstreamWireLimits {
  /// Creates validated wire limits.
  MdstreamWireLimits({
    String? maxCommandBytes,
    String? maxEncodedChangeBytes,
    String? maxEncodedSnapshotBytes,
    String? maxReducerUpdateBytes,
    String? maxProcessorPayloadBytes,
    String? maxArtifactEventBytes,
    String? maxViewBytes,
  }) : maxCommandBytes = _validatedOptionalDecimal(
         maxCommandBytes,
         'wire.max_command_bytes',
       ),
       maxEncodedChangeBytes = _validatedOptionalDecimal(
         maxEncodedChangeBytes,
         'wire.max_encoded_change_bytes',
       ),
       maxEncodedSnapshotBytes = _validatedOptionalDecimal(
         maxEncodedSnapshotBytes,
         'wire.max_encoded_snapshot_bytes',
       ),
       maxReducerUpdateBytes = _validatedOptionalDecimal(
         maxReducerUpdateBytes,
         'wire.max_reducer_update_bytes',
       ),
       maxProcessorPayloadBytes = _validatedOptionalDecimal(
         maxProcessorPayloadBytes,
         'wire.max_processor_payload_bytes',
       ),
       maxArtifactEventBytes = _validatedOptionalDecimal(
         maxArtifactEventBytes,
         'wire.max_artifact_event_bytes',
       ),
       maxViewBytes = _validatedOptionalDecimal(
         maxViewBytes,
         'wire.max_view_bytes',
       );

  /// Maximum bytes in one binding command.
  final String? maxCommandBytes;

  /// Maximum bytes in one encoded canonical change.
  final String? maxEncodedChangeBytes;

  /// Maximum bytes in one encoded canonical snapshot.
  final String? maxEncodedSnapshotBytes;

  /// Maximum bytes in one reducer update payload.
  final String? maxReducerUpdateBytes;

  /// Maximum bytes in one processor request or completion payload.
  final String? maxProcessorPayloadBytes;

  /// Maximum bytes in one artifact event payload.
  final String? maxArtifactEventBytes;

  /// Maximum bytes in one focused view payload.
  final String? maxViewBytes;

  Map<String, String> _toJson() => {
    if (maxCommandBytes != null) 'max_command_bytes': maxCommandBytes!,
    if (maxEncodedChangeBytes != null)
      'max_encoded_change_bytes': maxEncodedChangeBytes!,
    if (maxEncodedSnapshotBytes != null)
      'max_encoded_snapshot_bytes': maxEncodedSnapshotBytes!,
    if (maxReducerUpdateBytes != null)
      'max_reducer_update_bytes': maxReducerUpdateBytes!,
    if (maxProcessorPayloadBytes != null)
      'max_processor_payload_bytes': maxProcessorPayloadBytes!,
    if (maxArtifactEventBytes != null)
      'max_artifact_event_bytes': maxArtifactEventBytes!,
    if (maxViewBytes != null) 'max_view_bytes': maxViewBytes!,
  };
}

/// Resource limits and parser extensions used by one native session.
final class MdstreamSessionOptions {
  /// Creates validated session options.
  MdstreamSessionOptions({
    this.protocol,
    this.compiler,
    this.engine,
    this.processor,
    this.wire,
    List<MdstreamCustomBlock> customBlocks = const [],
    this.captureTransitions = false,
  }) : customBlocks = List.unmodifiable(customBlocks) {
    for (final block in this.customBlocks) {
      block._toJson();
    }
  }

  /// Parser-neutral Content IR and reducer limits.
  final MdstreamProtocolLimits? protocol;

  /// Compiler work and retained semantic-state limits.
  final MdstreamCompilerLimits? compiler;

  /// Canonical change construction limits.
  final MdstreamEngineLimits? engine;

  /// Processor scheduling and artifact limits.
  final MdstreamProcessorLimits? processor;

  /// Native binding payload limits.
  final MdstreamWireLimits? wire;

  /// Custom blocks sealed before the first input is appended.
  final List<MdstreamCustomBlock> customBlocks;

  /// Whether reducer updates include ordered transition facts.
  final bool captureTransitions;

  /// Encodes these options using the native binding-options [schema].
  Map<String, Object> toJson(String schema) {
    if (schema.isEmpty) {
      throw ArgumentError.value(schema, 'schema', 'must not be empty');
    }
    final protocolJson = protocol?._toJson();
    final compilerJson = compiler?._toJson();
    final engineJson = engine?._toJson();
    final processorJson = processor?._toJson();
    final wireJson = wire?._toJson();
    return {
      'schema': schema,
      if (protocolJson?.isNotEmpty ?? false) 'protocol': protocolJson!,
      if (compilerJson?.isNotEmpty ?? false) 'compiler': compilerJson!,
      if (engineJson?.isNotEmpty ?? false) 'engine': engineJson!,
      if (processorJson?.isNotEmpty ?? false) 'processor': processorJson!,
      if (wireJson?.isNotEmpty ?? false) 'wire': wireJson!,
      if (captureTransitions) 'capture_transitions': true,
      if (customBlocks.isNotEmpty)
        'custom_blocks': customBlocks.map((block) => block._toJson()).toList(),
    };
  }
}

final RegExp _decimalPattern = RegExp(r'^(0|[1-9][0-9]*)$');

String? _validatedOptionalDecimal(String? value, String field) =>
    value == null ? null : _validatedDecimal(value, field);

String _validatedDecimal(String value, String field) {
  if (!_decimalPattern.hasMatch(value)) {
    throw ArgumentError.value(
      value,
      field,
      'must be an unsigned canonical decimal string',
    );
  }
  return value;
}
