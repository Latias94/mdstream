import 'dart:convert';
import 'dart:io';

import 'package:mdstream/mdstream.dart';

void main(List<String> arguments) {
  final libraryPath = _libraryPath(arguments);
  if (libraryPath == null) {
    stderr.writeln(
      'Usage: dart run example/lossless_batching.dart --library PATH',
    );
    exitCode = 64;
    return;
  }

  final runtime = MdstreamRuntime.openPath(libraryPath);
  final engine = runtime.createEngine();
  final batcher = engine.createBatcher(maxBatchBytes: 32, maxPendingChunks: 8);
  final orderedResults = <EngineResult>[];
  String? callerOwnedChunk;

  try {
    for (final chunk in <String>[
      '# Lossless batching\n\n',
      'AI output ',
      'keeps caller boundaries.',
    ]) {
      callerOwnedChunk = chunk;
      try {
        orderedResults.addAll(batcher.push(chunk));
        callerOwnedChunk = null;
      } on BatchOperationException<EngineResult> catch (error) {
        if (error.operation == BatchOperation.push &&
            error.newInputAccepted == true) {
          callerOwnedChunk = null;
        }
        rethrow;
      }
    }
    orderedResults.addAll(batcher.finish());

    final recovery = batcher.createRecoverySnapshot();
    orderedResults.addAll(recovery.flushed);
    final snapshot = Map<String, Object?>.from(
      jsonDecode(utf8.decode(recovery.snapshot!.bytes)) as Map,
    );
    final reducerResults = orderedResults
        .expand((result) => result.reducerResults)
        .toList(growable: false);

    stdout.writeln('ordered_results=${orderedResults.length}');
    stdout.writeln('coherent_reducer_results=${reducerResults.length}');
    stdout.writeln('final_source=${jsonEncode(snapshot['source'])}');
    stdout.writeln('append_attempts=${batcher.metrics.appendAttempts}');
  } on BatchOperationException<EngineResult> catch (error) {
    orderedResults.addAll(error.completedResults);
    final pending = error.pending;
    stderr.writeln(
      'batch operation ${error.operation.wireValue} failed after '
      '${error.completedResults.length} commit(s); '
      'new_input_accepted=${error.newInputAccepted}; '
      'pending_constituents=${pending?.constituents ?? '0'}',
    );

    // An application can call retryPending() instead. This example transfers
    // the exact accepted suffix and separately retains unaccepted caller input.
    final transferred = batcher.takePending();
    stderr.writeln(
      'transferred_pending=${jsonEncode(transferred?.chunks ?? const [])}',
    );
    stderr.writeln('caller_owned_chunk=${jsonEncode(callerOwnedChunk)}');
    exitCode = 1;
  } on MdstreamException catch (error) {
    stderr.writeln(
      'input rejected before acceptance: ${error.detailCode}; '
      'caller_owned_chunk=${jsonEncode(callerOwnedChunk)}',
    );
    final transferred = batcher.takePending();
    stderr.writeln(
      'transferred_pending=${jsonEncode(transferred?.chunks ?? const [])}',
    );
    exitCode = 1;
  } finally {
    if (!batcher.isReleased) {
      final pending = batcher.inspectPending();
      if (pending != null) {
        final transferred = batcher.takePending();
        stderr.writeln(
          'transferred_pending=${jsonEncode(transferred?.chunks ?? const [])}',
        );
        exitCode = 1;
      }
      batcher.release();
    }
    engine.close();
  }

  if (runtime.nativeAllocations.isZero) {
    stdout.writeln('native_allocations=zero');
  } else {
    stderr.writeln('native allocations leaked');
    exitCode = 1;
  }
}

String? _libraryPath(List<String> arguments) {
  String? selected;
  for (var index = 0; index < arguments.length; index += 1) {
    final argument = arguments[index];
    if (argument == '--library' && index + 1 < arguments.length) {
      selected = arguments[index + 1];
      index += 1;
    } else {
      throw FormatException('unknown or incomplete argument: $argument');
    }
  }
  selected ??=
      Platform.environment['MDSTREAM_NATIVE_LIBRARY'] ??
      Platform.environment['MDSTREAM_FFI_LIBRARY'];
  return selected == null || selected.isEmpty
      ? null
      : File(selected).absolute.path;
}
