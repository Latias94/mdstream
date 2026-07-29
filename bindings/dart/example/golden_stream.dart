import 'dart:convert';
import 'dart:io';

import 'package:mdstream/mdstream.dart';

const _usage = '''
Usage: dart run example/golden_stream.dart [options]

Options:
  --library PATH     Load a trusted mdstream-ffi dynamic library.
  --scenario PATH    Replay a scenario other than the bundled fixture.
  --assert           Fail when canonical state drifts from the scenario.
  -h, --help         Show this help.

The library path may instead come from MDSTREAM_NATIVE_LIBRARY or
MDSTREAM_FFI_LIBRARY. A dynamic library is executable code: use only a path
from a source you trust. ABI and schema checks establish compatibility, not
authenticity.
''';

void main(List<String> arguments) {
  try {
    final options = _CliOptions.parse(arguments, Platform.environment);
    if (options.help) {
      stdout.write(_usage);
      return;
    }

    final scenario = _GoldenScenario.load(options.scenarioPath!);
    final runtime = MdstreamRuntime.openPath(options.libraryPath!);
    final output = _runGoldenStream(
      runtime,
      scenario,
      assertExpected: options.assertExpected,
    );
    stdout.write('${output.join('\n')}\n');
  } on _UsageFailure catch (error) {
    stderr.writeln(error.message);
    stderr.write(_usage);
    exitCode = 64;
  } on _AssertionFailure catch (error) {
    stderr.writeln('assertion failed: ${error.message}');
    exitCode = 1;
  } on FormatException catch (error) {
    stderr.writeln('scenario error: ${error.message}');
    exitCode = 65;
  } on MdstreamException catch (error) {
    stderr.writeln(
      'native error ${error.statusName}/${error.detailCode}: ${error.message}',
    );
    exitCode = 70;
  } on Object catch (error) {
    stderr.writeln('example failed: $error');
    exitCode = 70;
  }
}

List<String> _runGoldenStream(
  MdstreamRuntime runtime,
  _GoldenScenario scenario, {
  required bool assertExpected,
}) {
  MdstreamEngine? engine;
  Object? failure;
  StackTrace? failureStack;
  List<String>? output;

  try {
    engine = runtime.createEngine(options: _transitionOptions());
    output = _replay(engine, runtime, scenario, assertExpected);
  } on Object catch (error, stackTrace) {
    failure = error;
    failureStack = stackTrace;
  } finally {
    try {
      engine?.close();
    } on Object catch (error, stackTrace) {
      failure ??= error;
      failureStack ??= stackTrace;
    }
  }

  final allocations = runtime.nativeAllocations;
  if (!allocations.isZero) {
    throw StateError(
      'native allocations leaked: engines=${allocations.engineHandles}, '
      'reducers=${allocations.reducerHandles}, outputs=${allocations.outputs}, '
      'buffers=${allocations.buffers}, bytes=${allocations.bufferBytes}',
    );
  }
  if (failure != null) {
    Error.throwWithStackTrace(failure, failureStack!);
  }

  return <String>[...output!, 'native_allocations=zero'];
}

List<String> _replay(
  MdstreamEngine engine,
  MdstreamRuntime runtime,
  _GoldenScenario scenario,
  bool assertExpected,
) {
  final output = <String>[
    'scenario=${scenario.id}',
    'runtime_package=${runtime.packageVersion}',
    'binding_schema=${runtime.bindingSchema}',
  ];
  final checkpointIds = <String>[];
  var transitionIndex = 0;
  var pendingQueries = 0;

  for (final action in scenario.actions) {
    switch (action) {
      case _AppendAction(:final chunk):
        transitionIndex = _recordTransitions(
          output,
          engine.append(chunk),
          transitionIndex,
        );
      case _CheckpointAction(
        :final id,
        :final sourceCursor,
        :final observations,
      ):
        final document = engine.state.currentState.document;
        if (document == null) {
          throw _AssertionFailure('checkpoint $id has no document state');
        }
        final actualCursor = document.coordinate.sourceCursor;
        final rootIds = document.roots?.children ?? const <NodeId>[];
        output.add(
          'checkpoint=$id cursor=$actualCursor expected_cursor=$sourceCursor '
          'root_ids=${rootIds.join(',')}',
        );
        checkpointIds.add(id);
        if (assertExpected && actualCursor != sourceCursor.toString()) {
          throw _AssertionFailure(
            'checkpoint $id cursor is $actualCursor, expected $sourceCursor',
          );
        }
        if (observations.contains('pending_source')) {
          pendingQueries += 1;
          final pending = engine.state.pendingSourceView();
          output.add(
            'pending_source=$id range='
            '${pending?.range.start ?? 'none'}..${pending?.range.end ?? 'none'} '
            'text=${jsonEncode(pending?.text)}',
          );
          if (assertExpected && pending == null) {
            throw _AssertionFailure('checkpoint $id expected pending source');
          }
        }
      case _FinishAction():
        transitionIndex = _recordTransitions(
          output,
          engine.finish(),
          transitionIndex,
        );
    }
  }

  final document = engine.state.currentState.document;
  if (document == null) {
    throw _AssertionFailure('final document state is missing');
  }
  final finalNodes = _readFinalNodes(engine, document);
  final stableIds =
      finalNodes
          .where((view) => view.node.stability == 'stable')
          .map((view) => view.node.id)
          .toList(growable: false)
        ..sort(_compareDecimalIds);
  final allNodesStable = stableIds.length == finalNodes.length;
  final snapshot = engine.createRecoverySnapshot();
  if (snapshot == null) {
    throw _AssertionFailure('final recovery snapshot is missing');
  }
  final snapshotRecord = _record(
    jsonDecode(utf8.decode(snapshot.bytes)),
    'recovery snapshot',
  );
  final finalSource = _string(snapshotRecord['source'], 'snapshot.source');

  output
    ..add('checkpoints=${checkpointIds.join(',')}')
    ..add('pending_queries=$pendingQueries')
    ..add('stable_node_ids=${stableIds.join(',')}')
    ..add('final_lifecycle=${document.lifecycle}')
    ..add('final_source=${jsonEncode(finalSource)}');

  if (assertExpected) {
    if (finalSource != scenario.finalSource) {
      throw _AssertionFailure('final canonical source drifted');
    }
    if (document.lifecycle != scenario.lifecycle) {
      throw _AssertionFailure(
        'final lifecycle is ${document.lifecycle}, '
        'expected ${scenario.lifecycle}',
      );
    }
    if (allNodesStable != scenario.allNodesStable) {
      throw _AssertionFailure(
        'all_nodes_stable is $allNodesStable, '
        'expected ${scenario.allNodesStable}',
      );
    }
    if (transitionIndex == 0) {
      throw _AssertionFailure('transition capture produced no facts');
    }
    if (stableIds.isEmpty) {
      throw _AssertionFailure('final document has no stable node identities');
    }
    output.add('assertions=passed');
  } else {
    output.add('assertions=not_requested');
  }
  return output;
}

int _recordTransitions(
  List<String> output,
  EngineResult result,
  int startIndex,
) {
  var index = startIndex;
  for (final facts in result.transitionFacts) {
    output.add(
      'transition=$index scope=${facts.scope} '
      'categories=${_transitionCategories(facts).join(',')}',
    );
    index += 1;
  }
  return index;
}

List<String> _transitionCategories(TransitionFactsView facts) {
  if (facts is FullReplaceTransitionFactsView) {
    return const <String>['full_replace'];
  }
  final continuous = facts as ContinuousTransitionFactsView;
  final categories = <String>[
    ...continuous.nodes.map(_nodeTransitionCategory),
    ...continuous.structures.map((_) => 'structure'),
    ...continuous.resources.map(_resourceTransitionCategory),
  ];
  final before = continuous.before;
  if (before == null) {
    categories.add('document_initialize');
  } else if (before.lifecycle != continuous.after.lifecycle) {
    categories.add('lifecycle');
  }
  return categories.isEmpty ? const <String>['none'] : categories;
}

String _nodeTransitionCategory(NodeTransitionView transition) {
  if (transition.before == null) {
    return 'node_insert';
  }
  if (transition.after == null) {
    return 'node_remove';
  }
  return switch (transition.text) {
    ProjectionAppendTransitionView() => 'text_append',
    ReplacementTextTransitionView() => 'text_replace',
    _ when transition.before!.stability != transition.after!.stability =>
      'stability',
    _ => 'node_update',
  };
}

String _resourceTransitionCategory(ResourceTransitionView transition) {
  if (transition.beforeVersion == null) {
    return 'resource_insert';
  }
  if (transition.afterVersion == null) {
    return 'resource_remove';
  }
  return 'resource_update';
}

List<NodeView> _readFinalNodes(
  MdstreamEngine engine,
  DocumentSummaryView document,
) {
  final pending = <NodeId>[...?document.roots?.children];
  final visited = <NodeId>{};
  final nodes = <NodeView>[];
  for (var index = 0; index < pending.length; index += 1) {
    final id = pending[index];
    if (!visited.add(id)) {
      continue;
    }
    final view = engine.state.nodeView(id);
    if (view == null) {
      throw _AssertionFailure('final node $id is not materializable');
    }
    nodes.add(view);
    pending.addAll(view.node.children.children);
  }
  return nodes;
}

MdstreamSessionOptions _transitionOptions() => MdstreamSessionOptions(
  captureTransitions: true,
  protocol: MdstreamProtocolLimits(
    maxSourceBytes: '4096',
    maxNodes: '128',
    maxResources: '32',
    maxOperations: '1024',
    maxChangeStructuralItems: '1024',
    maxChildrenPerList: '128',
  ),
  wire: MdstreamWireLimits(maxReducerUpdateBytes: '4194304'),
);

int _compareDecimalIds(String left, String right) =>
    BigInt.parse(left).compareTo(BigInt.parse(right));

final class _CliOptions {
  const _CliOptions({
    required this.libraryPath,
    required this.scenarioPath,
    required this.assertExpected,
    required this.help,
  });

  factory _CliOptions.parse(
    List<String> arguments,
    Map<String, String> environment,
  ) {
    String? libraryPath;
    String? scenarioPath;
    var assertExpected = false;
    var help = false;

    for (var index = 0; index < arguments.length; index += 1) {
      final argument = arguments[index];
      if (argument == '--library' || argument == '--scenario') {
        if (index + 1 >= arguments.length) {
          throw _UsageFailure('$argument requires a path.');
        }
        final value = arguments[index + 1];
        index += 1;
        if (argument == '--library') {
          if (libraryPath != null) {
            throw const _UsageFailure('--library may be provided only once.');
          }
          libraryPath = value;
        } else {
          if (scenarioPath != null) {
            throw const _UsageFailure('--scenario may be provided only once.');
          }
          scenarioPath = value;
        }
      } else if (argument.startsWith('--library=')) {
        if (libraryPath != null) {
          throw const _UsageFailure('--library may be provided only once.');
        }
        libraryPath = argument.substring('--library='.length);
      } else if (argument.startsWith('--scenario=')) {
        if (scenarioPath != null) {
          throw const _UsageFailure('--scenario may be provided only once.');
        }
        scenarioPath = argument.substring('--scenario='.length);
      } else if (argument == '--assert') {
        if (assertExpected) {
          throw const _UsageFailure('--assert may be provided only once.');
        }
        assertExpected = true;
      } else if (argument == '--help' || argument == '-h') {
        help = true;
      } else {
        throw _UsageFailure('Unknown argument: $argument');
      }
    }

    if (help) {
      return _CliOptions(
        libraryPath: libraryPath,
        scenarioPath: scenarioPath,
        assertExpected: assertExpected,
        help: true,
      );
    }

    libraryPath ??=
        environment['MDSTREAM_NATIVE_LIBRARY'] ??
        environment['MDSTREAM_FFI_LIBRARY'];
    if (libraryPath == null) {
      throw const _UsageFailure('No native library was provided.');
    }
    if (libraryPath.isEmpty) {
      throw const _UsageFailure('The native library path must not be empty.');
    }
    if (scenarioPath != null && scenarioPath.isEmpty) {
      throw const _UsageFailure('The scenario path must not be empty.');
    }

    final bundledScenario = File.fromUri(
      File.fromUri(
        Platform.script,
      ).parent.uri.resolve('fixtures/golden_ai_stream.json'),
    );
    return _CliOptions(
      libraryPath: File(libraryPath).absolute.path,
      scenarioPath: scenarioPath == null
          ? bundledScenario.absolute.path
          : File(scenarioPath).absolute.path,
      assertExpected: assertExpected,
      help: false,
    );
  }

  final String? libraryPath;
  final String? scenarioPath;
  final bool assertExpected;
  final bool help;
}

sealed class _ScenarioAction {
  const _ScenarioAction();
}

final class _AppendAction extends _ScenarioAction {
  const _AppendAction(this.chunk);
  final String chunk;
}

final class _CheckpointAction extends _ScenarioAction {
  const _CheckpointAction({
    required this.id,
    required this.sourceCursor,
    required this.observations,
  });

  final String id;
  final int sourceCursor;
  final Set<String> observations;
}

final class _FinishAction extends _ScenarioAction {
  const _FinishAction();
}

final class _GoldenScenario {
  const _GoldenScenario({
    required this.id,
    required this.actions,
    required this.finalSource,
    required this.lifecycle,
    required this.allNodesStable,
  });

  factory _GoldenScenario.load(String path) {
    final file = File(path);
    if (!file.existsSync()) {
      throw FormatException('scenario does not exist: ${file.path}');
    }
    final root = _record(jsonDecode(file.readAsStringSync()), 'scenario');
    final schema = _string(root['schema'], 'scenario.schema');
    if (schema != 'mdstream.example-scenario/1') {
      throw FormatException('unsupported scenario schema: $schema');
    }
    final mainline = _record(
      _record(root['episodes'], 'scenario.episodes')['mainline'],
      'scenario.episodes.mainline',
    );
    final actions = <_ScenarioAction>[];
    final checkpointIds = <String>{};
    for (final value in _list(mainline['actions'], 'mainline.actions')) {
      final action = _record(value, 'mainline action');
      switch (_string(action['kind'], 'action.kind')) {
        case 'append':
          actions.add(_AppendAction(_string(action['chunk'], 'append.chunk')));
        case 'checkpoint':
          final id = _string(action['id'], 'checkpoint.id');
          if (!checkpointIds.add(id)) {
            throw FormatException('duplicate checkpoint id: $id');
          }
          actions.add(
            _CheckpointAction(
              id: id,
              sourceCursor: _integer(
                action['source_cursor'],
                'checkpoint.source_cursor',
              ),
              observations: _list(
                action['observations'],
                'checkpoint.observations',
              ).map((value) => _string(value, 'observation')).toSet(),
            ),
          );
        case 'finish':
          actions.add(const _FinishAction());
        case final kind:
          throw FormatException('unsupported mainline action: $kind');
      }
    }
    if (actions.isEmpty || actions.last is! _FinishAction) {
      throw const FormatException('mainline must end with finish');
    }
    if (actions.whereType<_FinishAction>().length != 1) {
      throw const FormatException('mainline must contain exactly one finish');
    }

    final expected = _record(root['expected'], 'scenario.expected');
    return _GoldenScenario(
      id: _string(root['id'], 'scenario.id'),
      actions: List<_ScenarioAction>.unmodifiable(actions),
      finalSource: _string(expected['final_source'], 'expected.final_source'),
      lifecycle: _string(expected['lifecycle'], 'expected.lifecycle'),
      allNodesStable: _boolean(
        expected['all_nodes_stable'],
        'expected.all_nodes_stable',
      ),
    );
  }

  final String id;
  final List<_ScenarioAction> actions;
  final String finalSource;
  final String lifecycle;
  final bool allNodesStable;
}

Map<String, Object?> _record(Object? value, String field) {
  if (value is! Map) {
    throw FormatException('$field must be an object');
  }
  final result = <String, Object?>{};
  for (final entry in value.entries) {
    if (entry.key is! String) {
      throw FormatException('$field keys must be strings');
    }
    result[entry.key as String] = entry.value;
  }
  return result;
}

List<Object?> _list(Object? value, String field) {
  if (value is! List) {
    throw FormatException('$field must be an array');
  }
  return List<Object?>.from(value);
}

String _string(Object? value, String field) {
  if (value is! String) {
    throw FormatException('$field must be a string');
  }
  return value;
}

int _integer(Object? value, String field) {
  if (value is! int || value < 0) {
    throw FormatException('$field must be a non-negative integer');
  }
  return value;
}

bool _boolean(Object? value, String field) {
  if (value is! bool) {
    throw FormatException('$field must be a boolean');
  }
  return value;
}

final class _UsageFailure implements Exception {
  const _UsageFailure(this.message);
  final String message;
}

final class _AssertionFailure implements Exception {
  const _AssertionFailure(this.message);
  final String message;
}
