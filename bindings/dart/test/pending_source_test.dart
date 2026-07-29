import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/fixtures.dart';
import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'pending source views are lazy, nullable, and precisely invalidated',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final changes = _characterChanges();
      final reducer = runtime.createReducer();
      try {
        final commandsBeforeEmptyQuery = _commands(reducer);
        expect(reducer.pendingSourceView(), isNull);
        expect(reducer.state.pendingSourceView(), isNull);
        expect(_commands(reducer), commandsBeforeEmptyQuery + BigInt.one);
        expect(reducer.metrics.pendingSourceViewPayloads, '0');

        final firstResult = reducer.applyChange(changes[0]);
        _expectNoPendingText(firstResult);
        final commandsBeforeFirstView = _commands(reducer);
        final first = reducer.state.pendingSourceView();
        expect(first?.range.start, '0');
        expect(first?.range.end, '1');
        expect(first?.text, 'a');
        expect(reducer.pendingSourceView(), same(first));
        expect(_commands(reducer), commandsBeforeFirstView + BigInt.one);

        reducer.applyChange(changes[1]);
        final second = reducer.pendingSourceView();
        expect(second, isNot(same(first)));
        expect(second?.range.start, '0');
        expect(second?.range.end, '2');
        expect(second?.text, 'ab');
        expect(reducer.state.pendingSourceView(), same(second));

        reducer.applyChange(changes[2]);
        final third = reducer.state.pendingSourceView();
        expect(third, isNot(same(second)));
        expect(third?.range.start, '0');
        expect(third?.range.end, '3');
        expect(third?.text, 'abc');
        expect(reducer.metrics.pendingSourceViewPayloads, '3');

        final finishResult = reducer.applyChange(changes[3]);
        _expectNoPendingText(finishResult);
        final commandsBeforeFinalQuery = _commands(reducer);
        expect(reducer.pendingSourceView(), isNull);
        expect(reducer.state.pendingSourceView(), isNull);
        expect(_commands(reducer), commandsBeforeFinalQuery + BigInt.one);
        expect(reducer.metrics.pendingSourceViewPayloads, '3');
      } finally {
        reducer.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    'recovery-required retains last-good pending source until replacement',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final changes = _characterChanges();
      final source = runtime.createReducer();
      final replica = runtime.createReducer();
      try {
        source.applyChange(changes[0]);
        source.applyChange(changes[1]);
        final replacement = source.createRecoverySnapshot()!;

        replica.applyChange(changes[0]);
        final lastGood = replica.pendingSourceView();
        expect(lastGood?.text, 'a');

        final gap = replica.applyChange(changes[2]);
        expect(gap.updates.single.outcome, isA<RecoveryRequiredOutcomeView>());
        expect(replica.pendingSourceView(), same(lastGood));

        final recovered = replica.recoverSnapshot(replacement);
        expect(recovered.updates.single.impact.fullReplace, isTrue);
        final current = replica.pendingSourceView();
        expect(current, isNot(same(lastGood)));
        expect(current?.range.end, '2');
        expect(current?.text, 'ab');
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
    'engine state exposes pending source without widening engine results',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final engine = runtime.createEngine();
      try {
        final prefix = engine.append('a *b');
        expect(prefix.updates.single.raw.keys, isNot(contains('text')));
        expect(engine.state.pendingSourceView(), isNull);

        final append = engine.append('*');
        expect(append.updates.single.raw.keys, isNot(contains('text')));
        final pending = engine.state.pendingSourceView();
        expect(pending?.range.start, '4');
        expect(pending?.range.end, '5');
        expect(pending?.text, '*');
        expect(engine.state.pendingSourceView(), same(pending));

        final finish = engine.finish();
        expect(finish.updates.single.raw.keys, isNot(contains('text')));
        expect(engine.state.pendingSourceView(), isNull);
      } finally {
        engine.close();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}

List<CanonicalChangeBytes> _characterChanges() {
  final fixture = loadFixture(
    'conformance/fixtures/protocol-linear-source.json',
  );
  final trace = list(fixture['traces'], 'traces')
      .map((value) => record(value, 'trace'))
      .singleWhere((value) => value['id'] == 'characters');
  return list(
    trace['changes'],
    'changes',
  ).map(encodeChange).toList(growable: false);
}

BigInt _commands(MdstreamReducer reducer) =>
    BigInt.parse(reducer.metrics.commands);

void _expectNoPendingText(ReducerResult result) {
  expect(result.updates, hasLength(1));
  expect(result.updates.single.kind, 'reducer_update');
  expect(result.updates.single.raw.keys, isNot(contains('text')));
  expect(result.updates.single.raw.keys, isNot(contains('range')));
}
