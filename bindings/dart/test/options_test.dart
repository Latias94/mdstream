import 'package:mdstream/src/options.dart';
import 'package:test/test.dart';

void main() {
  test(
    'omits defaulted custom-block booleans and preserves explicit false',
    () {
      const defaulted = MdstreamCustomBlock(
        namespace: 'app',
        name: 'defaulted',
      );
      const explicit = MdstreamCustomBlock(
        namespace: 'app',
        name: 'explicit',
        opaque: false,
        caseInsensitive: false,
      );
      final options = MdstreamSessionOptions(
        customBlocks: const [defaulted, explicit],
      );

      expect(defaulted.opaque, isNull);
      expect(defaulted.caseInsensitive, isNull);
      expect(options.toJson('mdstream.bindings-options/0.4'), {
        'schema': 'mdstream.bindings-options/0.4',
        'custom_blocks': [
          {'namespace': 'app', 'name': 'defaulted'},
          {
            'namespace': 'app',
            'name': 'explicit',
            'opaque': false,
            'case_insensitive': false,
          },
        ],
      });
    },
  );

  test('encodes every typed limit group with native snake-case keys', () {
    final options = MdstreamSessionOptions(
      protocol: MdstreamProtocolLimits(
        maxSourceBytes: '1',
        maxNodes: '2',
        maxResources: '3',
        maxOperations: '6',
        maxChangeStructuralItems: '7',
        maxDocumentStructuralItems: '8',
        maxChildrenPerList: '9',
        maxAttributesPerNode: '10',
        maxMetadataValueBytes: '11',
        maxNodeMetadataBytes: '12',
        maxChangeMetadataBytes: '13',
        maxDocumentMetadataBytes: '14',
        maxTreeDepth: '16',
      ),
      compiler: MdstreamCompilerLimits(
        maxMarkdownEvents: '17',
        maxMarkdownOverlapWork: '18',
        maxDefinitions: '4',
        maxDefinitionEdges: '5',
        maxDefinitionMetadataBytes: '15',
      ),
      engine: MdstreamEngineLimits(
        maxChangeBytes: '19',
        maxTransactionBytes: '20',
      ),
      processor: MdstreamProcessorLimits(
        maxInputBytes: '21',
        maxArtifactBytes: '22',
        maxInFlightJobs: '23',
        maxInFlightInputBytes: '24',
        maxSlots: '25',
        maxRetainedArtifacts: '26',
        maxRetainedArtifactBytes: '27',
        maxErrorBytes: '28',
        maxPendingChanges: '29',
        maxPendingChangeBytes: '30',
      ),
      wire: MdstreamWireLimits(
        maxCommandBytes: '31',
        maxEncodedChangeBytes: '32',
        maxEncodedSnapshotBytes: '33',
        maxReducerUpdateBytes: '34',
        maxProcessorPayloadBytes: '35',
        maxArtifactEventBytes: '36',
        maxViewBytes: '37',
      ),
    );

    expect(options.toJson('mdstream.bindings-options/0.4'), {
      'schema': 'mdstream.bindings-options/0.4',
      'protocol': {
        'max_source_bytes': '1',
        'max_nodes': '2',
        'max_resources': '3',
        'max_operations': '6',
        'max_change_structural_items': '7',
        'max_document_structural_items': '8',
        'max_children_per_list': '9',
        'max_attributes_per_node': '10',
        'max_metadata_value_bytes': '11',
        'max_node_metadata_bytes': '12',
        'max_change_metadata_bytes': '13',
        'max_document_metadata_bytes': '14',
        'max_tree_depth': '16',
      },
      'compiler': {
        'max_markdown_events': '17',
        'max_markdown_overlap_work': '18',
        'max_definitions': '4',
        'max_definition_edges': '5',
        'max_definition_metadata_bytes': '15',
      },
      'engine': {'max_change_bytes': '19', 'max_transaction_bytes': '20'},
      'processor': {
        'max_input_bytes': '21',
        'max_artifact_bytes': '22',
        'max_in_flight_jobs': '23',
        'max_in_flight_input_bytes': '24',
        'max_slots': '25',
        'max_retained_artifacts': '26',
        'max_retained_artifact_bytes': '27',
        'max_error_bytes': '28',
        'max_pending_changes': '29',
        'max_pending_change_bytes': '30',
      },
      'wire': {
        'max_command_bytes': '31',
        'max_encoded_change_bytes': '32',
        'max_encoded_snapshot_bytes': '33',
        'max_reducer_update_bytes': '34',
        'max_processor_payload_bytes': '35',
        'max_artifact_event_bytes': '36',
        'max_view_bytes': '37',
      },
    });
  });

  test('omits empty typed limit groups', () {
    final options = MdstreamSessionOptions(
      protocol: MdstreamProtocolLimits(),
      compiler: MdstreamCompilerLimits(),
      engine: MdstreamEngineLimits(),
      processor: MdstreamProcessorLimits(),
      wire: MdstreamWireLimits(),
    );

    expect(options.protocol, isA<MdstreamProtocolLimits>());
    expect(options.compiler, isA<MdstreamCompilerLimits>());
    expect(options.engine, isA<MdstreamEngineLimits>());
    expect(options.processor, isA<MdstreamProcessorLimits>());
    expect(options.wire, isA<MdstreamWireLimits>());
    expect(options.toJson('mdstream.bindings-options/0.4'), {
      'schema': 'mdstream.bindings-options/0.4',
    });
  });

  test('rejects non-canonical decimal values in every limit group', () {
    final invalidFactories = <Object? Function()>[
      () => MdstreamProtocolLimits(maxNodes: '01'),
      () => MdstreamCompilerLimits(maxMarkdownEvents: '-1'),
      () => MdstreamEngineLimits(maxChangeBytes: '1.0'),
      () => MdstreamProcessorLimits(maxSlots: ''),
      () => MdstreamWireLimits(maxViewBytes: '+1'),
    ];

    for (final factory in invalidFactories) {
      expect(factory, throwsArgumentError);
    }
  });

  test('maps transition capture and reducer-update budget options', () {
    final options = MdstreamSessionOptions(
      captureTransitions: true,
      wire: MdstreamWireLimits(maxReducerUpdateBytes: '32768'),
    );

    expect(options.captureTransitions, isTrue);
    expect(options.toJson('mdstream.bindings-options/0.4'), {
      'schema': 'mdstream.bindings-options/0.4',
      'capture_transitions': true,
      'wire': {'max_reducer_update_bytes': '32768'},
    });
  });

  test('encodes compiler budgets outside the protocol group', () {
    final options = MdstreamSessionOptions(
      compiler: MdstreamCompilerLimits(
        maxMarkdownEvents: '8',
        maxMarkdownOverlapWork: '16',
        maxDefinitions: '32',
        maxDefinitionEdges: '64',
        maxDefinitionMetadataBytes: '128',
      ),
    );

    expect(options.toJson('mdstream.bindings-options/0.4'), {
      'schema': 'mdstream.bindings-options/0.4',
      'compiler': {
        'max_markdown_events': '8',
        'max_markdown_overlap_work': '16',
        'max_definitions': '32',
        'max_definition_edges': '64',
        'max_definition_metadata_bytes': '128',
      },
    });
  });
}
