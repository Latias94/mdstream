import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/fixtures.dart';
import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'gap retains last-good state until explicit snapshot recovery',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final fixture = loadFixture(
        'conformance/fixtures/protocol-linear-source.json',
      );
      final traces = list(
        fixture['traces'],
        'traces',
      ).map((value) => record(value, 'trace'));
      final trace = traces.singleWhere((value) => value['id'] == 'characters');
      final changes = list(
        trace['changes'],
        'changes',
      ).map(encodeChange).toList(growable: false);

      final source = runtime.createReducer();
      final replica = runtime.createReducer();
      try {
        for (final change in changes.take(3)) {
          source.applyChange(change);
        }
        final recovery = source.createRecoverySnapshot()!;

        replica.applyChange(changes[0]);
        final lastGoodDocument = replica.currentState.document;
        final lastGood = replica.currentState.document?.coordinate;
        final gap = replica.applyChange(changes[2]);
        expect(gap.updates.single.outcome.kind, 'recovery_required');
        expect(replica.currentState.status.kind, 'needs_snapshot');
        expect(replica.currentState.document, same(lastGoodDocument));
        expect(
          replica.currentState.document?.coordinate.changeId,
          lastGood?.changeId,
        );
        expect(
          () => replica.applyChange(changes[3]),
          throwsA(
            isA<MdstreamException>()
                .having((error) => error.status, 'status', 9)
                .having(
                  (error) => error.statusName,
                  'statusName',
                  'MDSTREAM_NEEDS_SNAPSHOT',
                ),
          ),
        );

        final recovered = replica.recoverSnapshot(recovery);
        expect(recovered.updates.single.impact.fullReplace, isTrue);
        expect(replica.currentState.status.kind, 'ready');
        replica.applyChange(changes[3]);
        expect(replica.currentState.document?.lifecycle, 'finalized');
      } finally {
        replica.close();
        source.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    'exact retry is idempotent while a same-sequence fork needs recovery',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final fixture = loadFixture(
        'conformance/fixtures/protocol-linear-source.json',
      );
      final trace = list(fixture['traces'], 'traces')
          .map((value) => record(value, 'trace'))
          .singleWhere((value) => value['id'] == 'whole');
      final firstRecord = record(
        list(trace['changes'], 'changes').first,
        'first change',
      );
      final first = encodeChange(firstRecord);

      final retryReducer = runtime.createReducer();
      final forkReducer = runtime.createReducer();
      try {
        retryReducer.applyChange(first);
        final stateBeforeRetry = retryReducer.currentState;
        final impactBeforeRetry = stateBeforeRetry.impact;
        final coordinate = retryReducer.currentState.document?.coordinate;
        final retry = retryReducer.applyChange(first);
        expect(retry.updates.single.outcome.kind, 'idempotent');
        expect(retryReducer.currentState, same(stateBeforeRetry));
        expect(retryReducer.currentState.impact, same(impactBeforeRetry));
        expect(
          retryReducer.currentState.document?.coordinate.changeId,
          coordinate?.changeId,
        );

        final second = encodeChange(list(trace['changes'], 'changes')[1]);
        retryReducer.applyChange(second);
        final stateBeforeStale = retryReducer.currentState;
        final stale = retryReducer.applyChange(first);
        expect(stale.updates.single.outcome.kind, 'stale');
        expect(retryReducer.currentState, same(stateBeforeStale));

        forkReducer.applyChange(first);
        final forkRecord = Map<String, Object?>.from(firstRecord)
          ..['change_id'] = 'dart:conflicting-current-change';
        final fork = forkReducer.applyChange(encodeChange(forkRecord));
        expect(fork.updates.single.outcome.kind, 'recovery_required');
        expect(forkReducer.currentState.status.kind, 'needs_snapshot');
      } finally {
        forkReducer.close();
        retryReducer.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    'node views retain identity until a precise invalidation',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final engine = runtime.createEngine();
      try {
        engine.append('first paragraph\n\nsecond paragraph');
        engine.finish();
        final roots = engine.state.currentState.document?.roots?.children;
        expect(roots, hasLength(2));
        final first = engine.state.nodeView(roots![0]);
        final second = engine.state.nodeView(roots[1]);
        expect(engine.state.nodeView(roots[0]), same(first));
        expect(engine.state.nodeView(roots[1]), same(second));

        final reset = engine.reset();
        expect(reset.updates.single.impact.fullReplace, isTrue);
        expect(engine.state.nodeView(roots[0]), isNull);
        expect(engine.state.nodeView(roots[1]), isNull);
      } finally {
        engine.close();
        engine.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    'captured recovery facts distinguish continuous, empty, and full replace',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final fixture = loadFixture(
        'conformance/fixtures/protocol-linear-source.json',
      );
      final trace = list(fixture['traces'], 'traces')
          .map((value) => record(value, 'trace'))
          .singleWhere((value) => value['id'] == 'characters');
      final changes = list(
        trace['changes'],
        'changes',
      ).map(encodeChange).toList(growable: false);
      final options = MdstreamSessionOptions(
        captureTransitions: true,
        protocol: const {
          'max_source_bytes': '1024',
          'max_nodes': '16',
          'max_resources': '16',
          'max_operations': '64',
          'max_change_structural_items': '64',
          'max_children_per_list': '16',
        },
        wire: const {'max_reducer_update_bytes': '1048576'},
      );
      final producer = runtime.createReducer(options: options);
      final consumer = runtime.createReducer(options: options);
      try {
        for (final change in changes.take(3)) {
          producer.applyChange(change);
        }
        final advanced = producer.createRecoverySnapshot()!;

        final initial = consumer.applyChange(changes[0]);
        expect(initial.transitionFacts, hasLength(1));
        expect(initial.transitionFacts.single.scope, 'continuous');
        expect(initial.transitionFacts.single.after.continuityGeneration, '0');

        final retry = consumer.applyChange(changes[0]);
        expect(retry.updates.single.outcome.kind, 'idempotent');
        expect(retry.transitionFacts, isEmpty);

        final gap = consumer.applyChange(changes[2]);
        expect(gap.updates.single.outcome.kind, 'recovery_required');
        expect(gap.transitionFacts, isEmpty);

        final recovered = consumer.recoverSnapshot(advanced);
        expect(recovered.transitionFacts, hasLength(1));
        expect(recovered.transitionFacts.single.scope, 'full_replace');
        expect(
          recovered.transitionFacts.single.after.continuityGeneration,
          '1',
        );
      } finally {
        consumer.close();
        producer.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}
