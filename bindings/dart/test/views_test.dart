import 'dart:convert';
import 'dart:typed_data';

import 'package:mdstream/src/engine.dart';
import 'package:mdstream/src/errors.dart';
import 'package:mdstream/src/protocol.dart';
import 'package:mdstream/src/reducer_handle.dart';
import 'package:mdstream/src/views.dart';
import 'package:test/test.dart';

import 'support/fixtures.dart';
import 'support/transition_normalization.dart';

const schema = 'mdstream.bindings/0.4';

void main() {
  group('binding views', () {
    test('decodes every typed binding view', () {
      final cases = <(BindingPayloadKind, Map<String, Object?>, Type)>[
        (BindingPayloadKind.reducerUpdate, _reducerUpdate(), ReducerUpdateView),
        (BindingPayloadKind.nodeView, _nodeView(), NodeView),
        (BindingPayloadKind.resourceView, _resourceView(), ResourceView),
        (
          BindingPayloadKind.processorRequest,
          _processorRequest(),
          ProcessorRequestView,
        ),
        (
          BindingPayloadKind.processorCompletion,
          _processorCompletion(),
          ProcessorCompletionView,
        ),
        (
          BindingPayloadKind.artifactChange,
          _artifactChange(),
          ArtifactChangeView,
        ),
        (BindingPayloadKind.artifactView, _artifactView(), ArtifactView),
        (
          BindingPayloadKind.pendingSourceView,
          _pendingSourceView(),
          PendingSourceView,
        ),
      ];

      for (final (kind, payload, type) in cases) {
        final decoded = decodeBindingView(kind, _bytes(payload), schema);
        expect(decoded.runtimeType, type);
        expect(decoded.schema, schema);
        expect(decoded.kind, kind.viewKind);
        expect(decoded.raw, equals(payload));
      }
    });

    test('exposes decimal identities as strings and typed nested state', () {
      final update =
          decodeBindingView(
                BindingPayloadKind.reducerUpdate,
                _bytes(_reducerUpdate()),
                schema,
              )
              as ReducerUpdateView;
      final node =
          decodeBindingView(
                BindingPayloadKind.nodeView,
                _bytes(_nodeView()),
                schema,
              )
              as NodeView;

      expect(update.outcome.coordinate?.epoch, '1');
      expect(update.document?.projectionCursor, '3');
      expect(update.impact.changedNodeIds, <String>['7']);
      expect(node.node.id, '7');
      expect(node.node.source.start, '0');
      expect(node.node.children.children, <String>['8']);
      expect(node.node.content, isA<HeadingContentView>());
      expect((node.node.content as HeadingContentView).level, 1);
      expect(node.bodyText, 'Title');
    });

    test('recursively freezes raw JSON', () {
      final view =
          decodeBindingView(
                BindingPayloadKind.nodeView,
                _bytes(_nodeView()),
                schema,
              )
              as NodeView;

      expect(() => view.raw['new'] = true, throwsA(isA<UnsupportedError>()));
      final rawNode = view.raw['node']! as Map<String, Object?>;
      final rawContent = rawNode['content']! as Map<String, Object?>;
      expect(
        () => rawContent['kind'] = 'paragraph',
        throwsA(isA<UnsupportedError>()),
      );
      final rawChildren = rawNode['children']! as Map<String, Object?>;
      final children = rawChildren['children']! as List<Object?>;
      expect(() => children.add('mutable'), throwsA(isA<UnsupportedError>()));
    });

    test('decodes pending source with UTF-8 byte cursors', () {
      final view =
          decodeBindingView(
                BindingPayloadKind.pendingSourceView,
                _bytes(_pendingSourceView()),
                schema,
              )
              as PendingSourceView;

      expect(view.range.start, '0');
      expect(view.range.end, '3');
      expect(view.text, 'aé');
      expect(utf8.encode(view.text), hasLength(3));

      final rawRange = view.raw['range']! as Map<String, Object?>;
      expect(() => rawRange['start'] = '1', throwsA(isA<UnsupportedError>()));
    });

    test('rejects malformed pending source ranges', () {
      final numericStart = _pendingSourceView();
      (numericStart['range']! as Map<String, Object?>)['start'] = 0;
      final nonCanonicalEnd = _pendingSourceView();
      (nonCanonicalEnd['range']! as Map<String, Object?>)['end'] = '03';
      final nonRecordRange = _pendingSourceView()..['range'] = '0..3';

      for (final payload in <Map<String, Object?>>[
        numericStart,
        nonCanonicalEnd,
        nonRecordRange,
      ]) {
        expect(
          () => decodeBindingView(
            BindingPayloadKind.pendingSourceView,
            _bytes(payload),
            schema,
          ),
          throwsA(
            isA<MdstreamException>().having(
              (error) => error.detailCode,
              'detailCode',
              'bindings.invalid_payload',
            ),
          ),
        );
      }
    });

    test('validates schema, kind, UTF-8, and decimal fields', () {
      final wrongSchema = _nodeView()..['schema'] = 'mdstream.bindings/0.3';
      final wrongKind = _nodeView()..['kind'] = 'resource_view';
      final numericId = _nodeView();
      (numericId['node']! as Map<String, Object?>)['id'] = 7;

      for (final payload in <Map<String, Object?>>[
        wrongSchema,
        wrongKind,
        numericId,
      ]) {
        expect(
          () => decodeBindingView(
            BindingPayloadKind.nodeView,
            _bytes(payload),
            schema,
          ),
          throwsA(
            isA<MdstreamException>().having(
              (error) => error.detailCode,
              'detailCode',
              'bindings.invalid_payload',
            ),
          ),
        );
      }

      expect(
        () => decodeBindingView(
          BindingPayloadKind.nodeView,
          Uint8List.fromList(<int>[0xff]),
          schema,
        ),
        throwsA(isA<MdstreamException>()),
      );
    });

    test('does not decode canonical change bytes as a binding view', () {
      expect(
        () => decodeBindingView(
          BindingPayloadKind.change,
          _bytes(<String, Object?>{'schema': 'mdstream.content/0.4'}),
          schema,
        ),
        throwsA(isA<MdstreamException>()),
      );
    });
  });

  group('transition binding views', () {
    test('decodes immutable continuous transition facts', () {
      final update = _decodeUpdate(_continuousTransition());
      final transition = update.transition;

      expect(transition?.schema, transitionSchema);
      expect(transition?.facts, isA<ContinuousTransitionFactsView>());
      final facts = transition!.facts as ContinuousTransitionFactsView;
      expect(facts.scope, 'continuous');
      expect(facts.before, isNull);
      expect(facts.after.continuityGeneration, '0');
      expect(facts.nodes.single.key.nodeId, '7');
      expect(facts.nodes.single.after?.stability, 'provisional');
      expect(
        facts.nodes.single.after?.parent,
        isA<DocumentTransitionOwnerView>(),
      );
      expect(facts.nodes.single.text, isA<ProjectionAppendTransitionView>());
      final text = facts.nodes.single.text as ProjectionAppendTransitionView;
      expect(text.range.start, '1');
      expect(text.range.end, '2');
      expect(text.text, 'B');
      expect(facts.structures.single.owner, isA<NodeTransitionOwnerView>());
      expect(facts.structures.single.inserted.single.nodeId, '7');
      expect(facts.resources.single.key.resourceId, '9');
      expect(facts.resources.single.affectedNodes.single.nodeId, '7');

      final replacement = _continuousTransition();
      _map(_mapList(_map(replacement['facts'])['nodes']).first)['text'] =
          <String, Object?>{'kind': 'replacement'};
      final replacementFacts =
          _decodeUpdate(replacement).transition!.facts
              as ContinuousTransitionFactsView;
      expect(
        replacementFacts.nodes.single.text,
        isA<ReplacementTextTransitionView>(),
      );

      expect(() => facts.nodes.add(facts.nodes.single), throwsUnsupportedError);
      expect(
        () => facts.structures.single.inserted.add(
          facts.structures.single.inserted.single,
        ),
        throwsUnsupportedError,
      );
      final rawTransition = update.raw['transition']! as Map<String, Object?>;
      expect(
        () => rawTransition['schema'] = 'mdstream.transitions/2',
        throwsUnsupportedError,
      );
    });

    test('keeps transition optional and decodes full replacement facts', () {
      expect(_decodeUpdate().transition, isNull);

      final transition = _decodeUpdate({
        'schema': transitionSchema,
        'facts': {
          'scope': 'full_replace',
          'before': null,
          'after': _documentStamp('1'),
        },
      }).transition;

      expect(transition?.facts, isA<FullReplaceTransitionFactsView>());
      expect(transition?.facts.scope, 'full_replace');
      expect(transition?.facts.after.continuityGeneration, '1');
    });

    test('rejects draft, future, and unknown nested transition fields', () {
      for (final schema in <String>[
        'mdstream.transitions/draft',
        'mdstream.transitions/2',
      ]) {
        final transition = _continuousTransition()..['schema'] = schema;
        expect(() => _decodeUpdate(transition), _throwsInvalidPayload);
      }

      final mutations = <void Function(Map<String, Object?>)>[
        (transition) => transition['unexpected'] = true,
        (transition) => _map(transition['facts'])['unexpected'] = true,
        (transition) =>
            _map(_map(transition['facts'])['after'])['unexpected'] = true,
        (transition) => _map(
          _map(_map(transition['facts'])['after'])['coordinate'],
        )['unexpected'] = true,
        (transition) => _map(
          _mapList(_map(transition['facts'])['nodes']).first['key'],
        )['unexpected'] = true,
        (transition) => _map(
          _mapList(_map(transition['facts'])['nodes']).first['after'],
        )['unexpected'] = true,
        (transition) => _map(
          _map(
            _mapList(_map(transition['facts'])['nodes']).first['after'],
          )['parent'],
        )['unexpected'] = true,
        (transition) => _map(
          _mapList(_map(transition['facts'])['nodes']).first['text'],
        )['unexpected'] = true,
        (transition) => _map(
          _map(
            _mapList(_map(transition['facts'])['nodes']).first['text'],
          )['range'],
        )['unexpected'] = true,
        (transition) => _mapList(
          _map(transition['facts'])['structures'],
        ).first['unexpected'] = true,
        (transition) => _mapList(
          _map(transition['facts'])['resources'],
        ).first['unexpected'] = true,
        (transition) => _map(
          _mapList(_map(transition['facts'])['resources']).first['key'],
        )['unexpected'] = true,
      ];

      for (final mutate in mutations) {
        final transition = _continuousTransition();
        mutate(transition);
        expect(() => _decodeUpdate(transition), _throwsInvalidPayload);
      }
    });

    test('rejects unknown variants and missing required nullable fields', () {
      final unknownScope = _continuousTransition();
      _map(unknownScope['facts'])['scope'] = 'incremental';
      expect(() => _decodeUpdate(unknownScope), _throwsInvalidPayload);

      final unknownOwner = _continuousTransition();
      _map(
        _mapList(_map(unknownOwner['facts'])['structures']).first['owner'],
      )['kind'] = 'root';
      expect(() => _decodeUpdate(unknownOwner), _throwsInvalidPayload);

      final unknownText = _continuousTransition();
      _map(
        _mapList(_map(unknownText['facts'])['nodes']).first['text'],
      )['kind'] = 'append';
      expect(() => _decodeUpdate(unknownText), _throwsInvalidPayload);

      final missingFactsBefore = _continuousTransition();
      _map(missingFactsBefore['facts']).remove('before');
      expect(() => _decodeUpdate(missingFactsBefore), _throwsInvalidPayload);

      final missingNodeBefore = _continuousTransition();
      _mapList(
        _map(missingNodeBefore['facts'])['nodes'],
      ).first.remove('before');
      expect(() => _decodeUpdate(missingNodeBefore), _throwsInvalidPayload);

      final missingParent = _continuousTransition();
      _map(
        _mapList(_map(missingParent['facts'])['nodes']).first['after'],
      ).remove('parent');
      expect(() => _decodeUpdate(missingParent), _throwsInvalidPayload);

      final missingResourceVersion = _continuousTransition();
      _mapList(
        _map(missingResourceVersion['facts'])['resources'],
      ).first.remove('before_version');
      expect(
        () => _decodeUpdate(missingResourceVersion),
        _throwsInvalidPayload,
      );
    });

    test('enforces transition decimal and opaque identifier domains', () {
      final maximum = _continuousTransition();
      _map(_map(maximum['facts'])['after'])['continuity_generation'] =
          '18446744073709551615';
      _map(_mapList(_map(maximum['facts'])['nodes']).first['key'])['node_id'] =
          '340282366920938463463374607431768211455';
      _map(
        _mapList(_map(maximum['facts'])['resources']).first['key'],
      )['resource_id'] = '340282366920938463463374607431768211455';
      expect(_decodeUpdate(maximum).transition, isNotNull);

      final overflow = _continuousTransition();
      _map(_map(overflow['facts'])['after'])['continuity_generation'] =
          '18446744073709551616';
      expect(() => _decodeUpdate(overflow), _throwsInvalidPayload);

      final nonCanonical = _continuousTransition();
      _map(
        _mapList(_map(nonCanonical['facts'])['resources']).first['key'],
      )['resource_id'] = '09';
      expect(() => _decodeUpdate(nonCanonical), _throwsInvalidPayload);

      for (final value in <String>[
        '',
        'x' * 129,
        '版本',
        'invalid/value',
        'invalid\n',
      ]) {
        final invalidChange = _continuousTransition();
        _map(
          _map(_map(invalidChange['facts'])['after'])['coordinate'],
        )['change_id'] = value;
        expect(() => _decodeUpdate(invalidChange), _throwsInvalidPayload);

        final invalidNode = _continuousTransition();
        _map(
          _mapList(_map(invalidNode['facts'])['nodes']).first['after'],
        )['version'] = value;
        expect(() => _decodeUpdate(invalidNode), _throwsInvalidPayload);

        final invalidStructure = _continuousTransition();
        _mapList(
          _map(invalidStructure['facts'])['structures'],
        ).first['before_version'] = value;
        expect(() => _decodeUpdate(invalidStructure), _throwsInvalidPayload);

        final invalidResource = _continuousTransition();
        _mapList(
          _map(invalidResource['facts'])['resources'],
        ).first['after_version'] = value;
        expect(() => _decodeUpdate(invalidResource), _throwsInvalidPayload);
      }
    });

    test('reducer results preserve ordered A-to-B-to-A transition facts', () {
      final firstTransition = _continuousTransition();
      final firstNode = _mapList(_map(firstTransition['facts'])['nodes']).first;
      firstNode['before'] = <String, Object?>{
        ..._map(firstNode['after']),
        'version': 'A',
      };
      _map(firstNode['after'])['version'] = 'B';

      final secondTransition = _continuousTransition();
      final secondNode = _mapList(
        _map(secondTransition['facts'])['nodes'],
      ).first;
      secondNode['before'] = <String, Object?>{
        ..._map(secondNode['after']),
        'version': 'B',
      };
      _map(secondNode['after'])['version'] = 'A';

      final first = _decodeUpdate(firstTransition);
      final captureOff = _decodeUpdate();
      final second = _decodeUpdate(secondTransition);
      final result = ReducerResult(
        updates: <ReducerUpdateView>[first, captureOff, second],
        processorRequests: const [],
        processorCompletions: const [],
        artifactChanges: const [],
        outputPayloadBytes: '0',
      );

      expect(result.transitionFacts, hasLength(2));
      expect(
        result.transitionFacts.map((facts) {
          final continuous = facts as ContinuousTransitionFactsView;
          return continuous.nodes.single.after?.version;
        }),
        <String?>['B', 'A'],
      );
      expect(
        () => result.transitionFacts.add(result.transitionFacts.first),
        throwsUnsupportedError,
      );

      final disabled = ReducerResult(
        updates: <ReducerUpdateView>[captureOff],
        processorRequests: const [],
        processorCompletions: const [],
        artifactChanges: const [],
        outputPayloadBytes: '0',
      );
      expect(disabled.transitionFacts, isEmpty);
      expect(
        () => disabled.transitionFacts.add(first.transition!.facts),
        throwsUnsupportedError,
      );

      final operation = EngineResult(
        changes: const [],
        reducerResults: <ReducerResult>[disabled, result],
        outputPayloadBytes: '0',
      );
      expect(
        operation.transitionFacts.map((facts) {
          final continuous = facts as ContinuousTransitionFactsView;
          return continuous.nodes.single.after?.version;
        }),
        <String?>['B', 'A'],
      );
      expect(
        () => operation.transitionFacts.add(first.transition!.facts),
        throwsUnsupportedError,
      );
    });

    test('matches every shared Rust transition wire golden', () {
      final fixture = loadFixture('conformance/goldens/transition-v1.json');
      expect(fixture['schema'], 'mdstream.transition-golden/1');
      expect(fixture['binding_schema'], schema);
      expect(fixture['transition_schema'], transitionSchema);

      final goldenCases = list(
        fixture['cases'],
        'fixture.cases',
      ).map((entry) => record(entry, 'fixture case')).toList(growable: false);
      expect(goldenCases, hasLength(5));
      for (final goldenCase in goldenCases) {
        final id = goldenCase['id']! as String;
        final wireJson = goldenCase['wire_json']! as String;
        final decoded =
            decodeBindingView(
                  BindingPayloadKind.reducerUpdate,
                  Uint8List.fromList(utf8.encode(wireJson)),
                  schema,
                )
                as ReducerUpdateView;
        final expected = record(goldenCase['normalized'], '$id.normalized');

        expect(normalizeReducerUpdate(decoded), expected, reason: id);
      }
    });

    test('rejects shared golden draft and future schema negatives', () {
      final fixture = loadFixture('conformance/goldens/transition-v1.json');
      final goldenCases = <String, Map<String, Object?>>{
        for (final entry in list(fixture['cases'], 'fixture.cases'))
          (record(entry, 'fixture case')['id']! as String): record(
            entry,
            'fixture case',
          ),
      };

      for (final entry in list(
        fixture['invalid_transition_schemas'],
        'fixture.invalid_transition_schemas',
      )) {
        final invalid = record(entry, 'invalid transition schema');
        final baseId = invalid['base_case']! as String;
        final base = goldenCases[baseId]!;
        final wire = record(
          jsonDecode(base['wire_json']! as String),
          '$baseId.wire_json',
        );
        final transition = record(wire['transition'], '$baseId.transition')
          ..['schema'] = invalid['schema']! as String;
        wire['transition'] = transition;

        expect(
          () => decodeBindingView(
            BindingPayloadKind.reducerUpdate,
            _bytes(wire),
            schema,
          ),
          _throwsInvalidPayload,
          reason: invalid['id']! as String,
        );
      }
    });
  });
}

final Matcher _throwsInvalidPayload = throwsA(
  isA<MdstreamException>().having(
    (error) => error.detailCode,
    'detailCode',
    'bindings.invalid_payload',
  ),
);

ReducerUpdateView _decodeUpdate([Map<String, Object?>? transition]) {
  final update = _reducerUpdate();
  if (transition != null) {
    update['transition'] = transition;
  }
  return decodeBindingView(
        BindingPayloadKind.reducerUpdate,
        _bytes(update),
        schema,
      )
      as ReducerUpdateView;
}

Map<String, Object?> _map(Object? value) => value! as Map<String, Object?>;

List<Map<String, Object?>> _mapList(Object? value) =>
    (value! as List<Object?>).cast<Map<String, Object?>>();

Uint8List _bytes(Map<String, Object?> value) =>
    Uint8List.fromList(utf8.encode(jsonEncode(value)));

Map<String, Object?> _coordinate() => <String, Object?>{
  'epoch': '1',
  'sequence': '2',
  'change_id': 'change:2',
  'source_cursor': '3',
};

Map<String, Object?> _impact() => <String, Object?>{
  'changed_node_ids': <Object?>['7'],
  'removed_node_ids': <Object?>[],
  'changed_resource_ids': <Object?>['9'],
  'removed_resource_ids': <Object?>[],
  'source_changed': true,
  'projection_changed': true,
  'lifecycle_changed': false,
  'roots_changed': true,
  'full_replace': false,
};

Map<String, Object?> _reducerUpdate() => <String, Object?>{
  'schema': schema,
  'kind': 'reducer_update',
  'outcome': <String, Object?>{'kind': 'applied', 'coordinate': _coordinate()},
  'status': <String, Object?>{'kind': 'ready'},
  'impact': _impact(),
  'document': <String, Object?>{
    'coordinate': _coordinate(),
    'lifecycle': 'open',
    'projection_cursor': '3',
    'roots': <String, Object?>{
      'version': 'sha256:roots',
      'children': <Object?>['7'],
    },
  },
};

Map<String, Object?> _node() => <String, Object?>{
  'id': '7',
  'version': 'sha256:node',
  'stability': 'stable',
  'source': <String, Object?>{'start': '0', 'end': '7'},
  'body': <String, Object?>{'start': '2', 'end': '7'},
  'children': <String, Object?>{
    'version': 'sha256:children',
    'children': <Object?>['8'],
  },
  'content': <String, Object?>{'kind': 'heading', 'level': 1},
};

Map<String, Object?> _nodeView() => <String, Object?>{
  'schema': schema,
  'kind': 'node_view',
  'node': _node(),
  'body_text': 'Title',
};

Map<String, Object?> _resource() => <String, Object?>{
  'id': '9',
  'version': 'sha256:resource',
  'content': <String, Object?>{
    'kind': 'link',
    'destination': 'https://example.com',
    'title': null,
  },
};

Map<String, Object?> _resourceView() => <String, Object?>{
  'schema': schema,
  'kind': 'resource_view',
  'resource': _resource(),
};

Map<String, Object?> _processorKey() => <String, Object?>{
  'epoch': '1',
  'node_id': '7',
  'processor_id': 'mdstream.mermaid',
  'node_version': 'sha256:node',
  'input_version': 'sha256:input',
  'processor_version': '1',
  'configuration_version': '1',
  'generation': '2',
};

Map<String, Object?> _processorRequest() => <String, Object?>{
  'schema': schema,
  'kind': 'processor_request',
  'request_id': '2',
  'key': _processorKey(),
  'input': <String, Object?>{
    'node': _node(),
    'body': 'flowchart LR',
    'resource': _resource(),
  },
};

Map<String, Object?> _processorCompletion() => <String, Object?>{
  'schema': schema,
  'kind': 'processor_completion',
  'request_id': '2',
  'outcome': 'applied',
};

Map<String, Object?> _artifactChange() => <String, Object?>{
  'schema': schema,
  'kind': 'artifact_change',
  'key': _processorKey(),
  'change': <String, Object?>{'kind': 'ready', 'artifact_bytes': '42'},
};

Map<String, Object?> _artifactView() => <String, Object?>{
  'schema': schema,
  'kind': 'artifact_view',
  'key': _processorKey(),
  'state': 'ready',
  'artifact': <String, Object?>{
    'protocol': 'mdstream.processor-artifact/0.4',
    'media_type': 'image/svg+xml',
    'payload': <String, Object?>{'kind': 'text', 'text': '<svg />'},
  },
  'failure': null,
};

Map<String, Object?> _pendingSourceView() => <String, Object?>{
  'schema': schema,
  'kind': 'pending_source_view',
  'range': <String, Object?>{'start': '0', 'end': '3'},
  'text': 'aé',
};

Map<String, Object?> _documentStamp(String continuityGeneration) =>
    <String, Object?>{
      'continuity_generation': continuityGeneration,
      'coordinate': <String, Object?>{
        'epoch': '1',
        'sequence': '1',
        'change_id': 'transition:test',
        'source_cursor': '2',
      },
      'lifecycle': 'open',
      'projection_cursor': '2',
      'roots_version': 'sha256:roots-after',
    };

Map<String, Object?> _transitionNodeKey() => <String, Object?>{
  'continuity_generation': '0',
  'epoch': '1',
  'node_id': '7',
};

Map<String, Object?> _continuousTransition() => <String, Object?>{
  'schema': transitionSchema,
  'facts': <String, Object?>{
    'scope': 'continuous',
    'before': null,
    'after': _documentStamp('0'),
    'nodes': <Object?>[
      <String, Object?>{
        'key': _transitionNodeKey(),
        'before': null,
        'after': <String, Object?>{
          'version': 'sha256:node-after',
          'stability': 'provisional',
          'parent': <String, Object?>{'kind': 'document'},
          'children_version': 'sha256:children-after',
        },
        'text': <String, Object?>{
          'kind': 'projection_append',
          'range': <String, Object?>{'start': '1', 'end': '2'},
          'text': 'B',
        },
      },
    ],
    'structures': <Object?>[
      <String, Object?>{
        'owner': <String, Object?>{'kind': 'node', 'key': _transitionNodeKey()},
        'before_version': 'sha256:children-before',
        'after_version': 'sha256:children-after',
        'start': 0,
        'removed': <Object?>[],
        'inserted': <Object?>[_transitionNodeKey()],
      },
    ],
    'resources': <Object?>[
      <String, Object?>{
        'key': <String, Object?>{
          'continuity_generation': '0',
          'epoch': '1',
          'resource_id': '9',
        },
        'before_version': null,
        'after_version': 'sha256:resource-after',
        'affected_nodes': <Object?>[_transitionNodeKey()],
      },
    ],
  },
};
