import 'dart:convert';
import 'dart:typed_data';

import 'package:mdstream/src/errors.dart';
import 'package:mdstream/src/protocol.dart';
import 'package:mdstream/src/views.dart';
import 'package:test/test.dart';

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
}

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
