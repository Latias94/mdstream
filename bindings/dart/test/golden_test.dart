import 'dart:convert';

import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/fixtures.dart';
import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'every shared protocol trace reaches the Rust normalized snapshot',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      for (final fixtureName in _canonicalFixtures) {
        final fixture = loadFixture('conformance/fixtures/$fixtureName');
        final expectedValue = record(
          fixture['expected'],
          'expected',
        )['normalized_snapshot'];
        final expected = expectedValue is Map
            ? Map<String, Object?>.from(expectedValue)
            : null;
        Map<String, Object?>? fixtureBaseline;

        for (final traceValue in list(fixture['traces'], 'traces')) {
          final trace = record(traceValue, 'trace');
          final reducer = runtime.createReducer();
          try {
            for (final change in list(trace['changes'], 'changes')) {
              final result = reducer.applyChange(encodeChange(change));
              expect(result.updates, hasLength(1));
              expect(
                result.outputPayloadBytes,
                matches(RegExp(r'^[1-9][0-9]*$')),
              );
            }
            expect(reducer.metrics.snapshotPayloads, '0');
            final snapshot = reducer.createRecoverySnapshot();
            expect(snapshot, isNotNull);
            final normalized = normalizeSnapshot(decodeSnapshot(snapshot!));
            if (expected != null) {
              expect(
                normalized,
                expected,
                reason: '$fixtureName/${trace['id']}',
              );
            } else if (fixtureBaseline == null) {
              fixtureBaseline = normalized;
            } else {
              expect(
                normalized,
                fixtureBaseline,
                reason: '$fixtureName/${trace['id']}',
              );
            }
            expect(normalized['source'], fixture['source']);
            expect(reducer.metrics.snapshotPayloads, '1');
          } finally {
            reducer.close();
          }
        }
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    'whole and adversarial engine schedules preserve final identity',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final fixture = loadFixture(
        'conformance/fixtures/adoption/headless-rich-content.json',
      );
      final expected = record(
        record(fixture['expected'], 'expected')['normalized_snapshot'],
        'normalized_snapshot',
      );
      final results = <String, _EngineTraceResult>{};
      for (final traceValue in list(fixture['traces'], 'traces')) {
        final trace = record(traceValue, 'trace');
        final id = trace['id']! as String;
        final engine = runtime.createEngine();
        final nodeIds = <NodeId>{};
        try {
          for (final eventValue in list(
            trace['input_events'],
            'input_events',
          )) {
            final event = record(eventValue, 'input event');
            final result = switch (event['kind']) {
              'append' => engine.append(event['chunk']! as String),
              'finish' => engine.finish(),
              _ => throw FormatException(
                'unknown input event ${event['kind']}',
              ),
            };
            for (final update in result.updates) {
              for (final removed in update.impact.removedNodeIds) {
                nodeIds.remove(removed);
              }
              for (final changed in update.impact.changedNodeIds) {
                final view = engine.state.nodeView(changed);
                if (view == null) {
                  nodeIds.remove(changed);
                } else {
                  nodeIds.add(changed);
                }
              }
            }
          }
          expect(engine.metrics.snapshotPayloads, '0');
          expect(engine.reducerMetrics.snapshotPayloads, '0');
          final snapshot = engine.createRecoverySnapshot();
          expect(snapshot, isNotNull);
          expect(engine.metrics.snapshotPayloads, '1');
          results[id] = _EngineTraceResult(
            normalized: normalizeSnapshot(decodeSnapshot(snapshot!)),
            nodeIds: nodeIds.toList()..sort(),
          );
          expect(utf8.decode(snapshot.bytes), isNot(contains('artifact_view')));
        } finally {
          engine.close();
        }
      }

      expect(results['whole']?.normalized, expected);
      expect(results['adversarial']?.normalized, expected);
      expect(results['adversarial']?.nodeIds, results['whole']?.nodeIds);
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}

final class _EngineTraceResult {
  const _EngineTraceResult({required this.normalized, required this.nodeIds});

  final Map<String, Object?> normalized;
  final List<NodeId> nodeIds;
}

const _canonicalFixtures = <String>[
  'protocol-linear-source.json',
  'protocol-epoch-reset.json',
  'adoption/headless-rich-content.json',
];
