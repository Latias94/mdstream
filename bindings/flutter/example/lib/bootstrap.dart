// ignore_for_file: public_member_api_docs

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

enum GoldenPlaybackMode { immediate, paced }

enum GoldenHostPhase {
  booting,
  readyEmpty,
  streaming,
  draining,
  settled,
  interrupted,
  error,
}

abstract interface class GoldenPlaybackClock {
  Future<void> wait(Duration duration);

  void cancel();
}

final class GoldenStreamCheckpoint {
  const GoldenStreamCheckpoint({
    required this.id,
    required this.sourceCursor,
    required this.lifecycle,
    required this.rootIds,
  });

  final String id;
  final int sourceCursor;
  final String lifecycle;
  final List<NodeId> rootIds;
}

typedef GoldenScenarioLoader = Future<String> Function();
typedef GoldenRuntimeOpener = MdstreamRuntime Function();
typedef GoldenControllerFactory =
    MdstreamController Function(MdstreamRuntime runtime);

final class GoldenStreamBootstrap extends ChangeNotifier {
  GoldenStreamBootstrap({
    required GoldenScenarioLoader loadScenario,
    required GoldenRuntimeOpener openRuntime,
    GoldenControllerFactory? createController,
    GoldenPlaybackClock? clock,
    this.pacedDelay = const Duration(milliseconds: 420),
  }) : _loadScenario = loadScenario,
       _openRuntime = openRuntime,
       _createController = createController ?? _defaultControllerFactory,
       _clock = clock ?? _TimerPlaybackClock();

  factory GoldenStreamBootstrap.bundled() => GoldenStreamBootstrap(
    loadScenario: () => rootBundle.loadString('assets/golden_ai_stream.json'),
    openRuntime: MdstreamFlutterRuntime.open,
  );

  static MdstreamController _defaultControllerFactory(
    MdstreamRuntime runtime,
  ) => MdstreamController.fromRuntime(runtime, options: _sessionOptions);

  static final MdstreamSessionOptions _sessionOptions = MdstreamSessionOptions(
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

  final GoldenScenarioLoader _loadScenario;
  final GoldenRuntimeOpener _openRuntime;
  final GoldenControllerFactory _createController;
  final GoldenPlaybackClock _clock;
  final Duration pacedDelay;

  GoldenHostPhase _phase = GoldenHostPhase.booting;
  GoldenPlaybackMode _mode = GoldenPlaybackMode.paced;
  _GoldenScenario? _scenario;
  MdstreamRuntime? _runtime;
  MdstreamController? _controller;
  GoldenStreamCheckpoint? _finalCheckpoint;
  Object? _error;
  String? _currentActionId;
  int _generation = 0;
  bool _disposed = false;

  GoldenHostPhase get phase => _phase;
  GoldenPlaybackMode get mode => _mode;
  MdstreamRuntime? get runtime => _runtime;
  MdstreamController? get controller => _controller;
  GoldenStreamCheckpoint? get finalCheckpoint => _finalCheckpoint;
  Object? get error => _error;
  String? get currentActionId => _currentActionId;

  String get statusLabel => switch (_phase) {
    GoldenHostPhase.booting => 'Opening runtime',
    GoldenHostPhase.readyEmpty => 'Ready',
    GoldenHostPhase.streaming => 'Streaming',
    GoldenHostPhase.draining => 'Finalizing',
    GoldenHostPhase.settled => 'Settled',
    GoldenHostPhase.interrupted => 'Interrupted',
    GoldenHostPhase.error => 'Initialization failed',
  };

  String get errorLabel {
    final value = _error;
    if (value is FormatException) {
      return value.message;
    }
    if (value is MdstreamException) {
      return '${value.statusName}: ${value.message}';
    }
    return value?.toString() ?? 'Unknown initialization failure';
  }

  Future<void> initialize({
    bool autoPlay = true,
    GoldenPlaybackMode? mode,
    bool reducedMotion = false,
  }) async {
    if (_disposed) {
      return;
    }
    if (_scenario != null && _runtime != null && _controller != null) {
      if (autoPlay && _phase == GoldenHostPhase.readyEmpty) {
        await _runPlayback(reducedMotion: reducedMotion);
      }
      return;
    }

    final generation = _beginGeneration();
    _replaceController(null);
    _scenario = null;
    _runtime = null;
    _error = null;
    _finalCheckpoint = null;
    _currentActionId = null;
    _mode = mode ?? _mode;
    _setPhase(GoldenHostPhase.booting, force: true);

    try {
      final source = await _loadScenario();
      if (!_isCurrent(generation)) {
        return;
      }
      final scenario = _GoldenScenario.decode(source);
      final runtime = _openRuntime();
      if (!_isCurrent(generation)) {
        return;
      }
      _scenario = scenario;
      _runtime = runtime;
      _replaceController(_createController(runtime));
      _setPhase(GoldenHostPhase.readyEmpty);
      if (autoPlay) {
        await _runPlayback(reducedMotion: reducedMotion);
      }
    } catch (error) {
      if (!_isCurrent(generation)) {
        return;
      }
      _replaceController(null);
      _error = error;
      _setPhase(GoldenHostPhase.error);
    }
  }

  Future<void> replay({
    required GoldenPlaybackMode mode,
    bool reducedMotion = false,
  }) async {
    if (_disposed) {
      return;
    }
    final runtime = _runtime;
    final scenario = _scenario;
    if (runtime == null || scenario == null) {
      await initialize(
        autoPlay: true,
        mode: mode,
        reducedMotion: reducedMotion,
      );
      return;
    }

    final wasActive =
        _phase == GoldenHostPhase.streaming ||
        _phase == GoldenHostPhase.draining;
    final generation = _beginGeneration();
    if (wasActive) {
      _setPhase(GoldenHostPhase.interrupted);
      await Future<void>.delayed(Duration.zero);
      if (!_isCurrent(generation)) {
        return;
      }
    }
    _mode = mode;
    _error = null;
    _finalCheckpoint = null;
    _currentActionId = null;
    try {
      _replaceController(_createController(runtime));
      _setPhase(GoldenHostPhase.readyEmpty);
      await _runPlayback(reducedMotion: reducedMotion);
    } catch (error) {
      if (!_isCurrent(generation)) {
        return;
      }
      _replaceController(null);
      _error = error;
      _setPhase(GoldenHostPhase.error);
    }
  }

  Future<void> retry({bool reducedMotion = false}) async {
    _beginGeneration();
    _replaceController(null);
    _scenario = null;
    _runtime = null;
    await initialize(autoPlay: true, reducedMotion: reducedMotion);
  }

  void interrupt() {
    if (_disposed ||
        (_phase != GoldenHostPhase.streaming &&
            _phase != GoldenHostPhase.draining)) {
      return;
    }
    _beginGeneration();
    _setPhase(GoldenHostPhase.interrupted);
  }

  Future<void> _runPlayback({required bool reducedMotion}) async {
    final scenario = _scenario;
    final controller = _controller;
    if (scenario == null || controller == null || _disposed) {
      return;
    }
    final generation = _generation;
    final source = StringBuffer();
    final checkpointIds = <String>[];
    _setPhase(GoldenHostPhase.streaming);

    try {
      for (final action in scenario.actions) {
        if (!_isCurrent(generation)) {
          return;
        }
        _currentActionId = action.id;
        switch (action) {
          case _AppendAction(:final chunk):
            controller.append(chunk);
            source.write(chunk);
            if (_mode == GoldenPlaybackMode.paced && !reducedMotion) {
              await _clock.wait(pacedDelay);
            }
          case _CheckpointAction(:final sourceCursor):
            final actualCursor = int.parse(
              controller.value.document!.coordinate.sourceCursor,
            );
            if (actualCursor != sourceCursor) {
              throw StateError(
                'Checkpoint ${action.id} expected cursor $sourceCursor, '
                'received $actualCursor.',
              );
            }
            checkpointIds.add(action.id);
          case _FinishAction():
            _setPhase(GoldenHostPhase.draining);
            controller.finish();
            _verifyFinalState(
              scenario: scenario,
              controller: controller,
              source: source.toString(),
              checkpointIds: checkpointIds,
              finishId: action.id,
            );
        }
      }
      if (!_isCurrent(generation)) {
        return;
      }
      _setPhase(GoldenHostPhase.settled);
    } catch (error) {
      if (!_isCurrent(generation)) {
        return;
      }
      _error = error;
      _setPhase(GoldenHostPhase.error);
    }
  }

  void _verifyFinalState({
    required _GoldenScenario scenario,
    required MdstreamController controller,
    required String source,
    required List<String> checkpointIds,
    required String finishId,
  }) {
    if (source != scenario.finalSource) {
      throw StateError('Golden stream final source drifted.');
    }
    if (checkpointIds.length != scenario.checkpointCount) {
      throw StateError('Golden stream checkpoint coverage drifted.');
    }
    final document = controller.value.document;
    if (document == null || document.lifecycle != scenario.finalLifecycle) {
      throw StateError('Golden stream final lifecycle drifted.');
    }
    final rootIds = List<NodeId>.unmodifiable(
      document.roots?.children ?? const <NodeId>[],
    );
    if (rootIds.isEmpty) {
      throw StateError('Golden stream produced no root nodes.');
    }
    final sourceCursor = int.parse(document.coordinate.sourceCursor);
    _finalCheckpoint = GoldenStreamCheckpoint(
      id: finishId,
      sourceCursor: sourceCursor,
      lifecycle: document.lifecycle,
      rootIds: rootIds,
    );
  }

  int _beginGeneration() {
    _generation += 1;
    _clock.cancel();
    return _generation;
  }

  bool _isCurrent(int generation) => !_disposed && generation == _generation;

  void _replaceController(MdstreamController? next) {
    final previous = _controller;
    if (identical(previous, next)) {
      return;
    }
    _controller = next;
    previous?.dispose();
  }

  void _setPhase(GoldenHostPhase next, {bool force = false}) {
    if (_disposed || (!force && next == _phase)) {
      return;
    }
    _phase = next;
    notifyListeners();
  }

  @override
  void dispose() {
    if (_disposed) {
      return;
    }
    _disposed = true;
    _generation += 1;
    _clock.cancel();
    _replaceController(null);
    super.dispose();
  }
}

sealed class _GoldenAction {
  const _GoldenAction(this.id);

  final String id;
}

final class _AppendAction extends _GoldenAction {
  const _AppendAction(super.id, this.chunk);

  final String chunk;
}

final class _CheckpointAction extends _GoldenAction {
  const _CheckpointAction(super.id, this.sourceCursor);

  final int sourceCursor;
}

final class _FinishAction extends _GoldenAction {
  const _FinishAction(super.id);
}

final class _GoldenScenario {
  const _GoldenScenario({
    required this.actions,
    required this.finalSource,
    required this.finalLifecycle,
    required this.checkpointCount,
  });

  final List<_GoldenAction> actions;
  final String finalSource;
  final String finalLifecycle;
  final int checkpointCount;

  static _GoldenScenario decode(String source) {
    final Object? decoded;
    try {
      decoded = jsonDecode(source);
    } on FormatException catch (error) {
      throw FormatException('Scenario JSON is invalid: ${error.message}');
    }
    final root = _record(decoded, 'scenario');
    if (root['schema'] != 'mdstream.example-scenario/1') {
      throw const FormatException('Scenario schema is unsupported.');
    }
    if (root['id'] != 'golden-ai-stream') {
      throw const FormatException('Scenario ID is unsupported.');
    }
    final episodes = _record(root['episodes'], 'episodes');
    final mainline = _record(episodes['mainline'], 'episodes.mainline');
    final rawActions = _list(mainline['actions'], 'episodes.mainline.actions');
    final actions = <_GoldenAction>[];
    var checkpointCount = 0;
    for (final (index, value) in rawActions.indexed) {
      final action = _record(value, 'actions[$index]');
      final id = _string(action['id'], 'actions[$index].id');
      switch (_string(action['kind'], 'actions[$index].kind')) {
        case 'append':
          actions.add(
            _AppendAction(
              id,
              _string(action['chunk'], 'actions[$index].chunk'),
            ),
          );
        case 'checkpoint':
          checkpointCount += 1;
          actions.add(
            _CheckpointAction(
              id,
              _integer(
                action['source_cursor'],
                'actions[$index].source_cursor',
              ),
            ),
          );
        case 'finish':
          actions.add(_FinishAction(id));
        case final kind:
          throw FormatException('Unsupported scenario action: $kind.');
      }
    }
    if (actions.isEmpty || actions.last is! _FinishAction) {
      throw const FormatException('Scenario must end with finish.');
    }
    final expected = _record(root['expected'], 'expected');
    final finalSource = _string(
      expected['final_source'],
      'expected.final_source',
    );
    final concatenated = actions
        .whereType<_AppendAction>()
        .map((action) => action.chunk)
        .join();
    if (concatenated != finalSource) {
      throw const FormatException('Scenario chunks do not match final source.');
    }
    return _GoldenScenario(
      actions: List<_GoldenAction>.unmodifiable(actions),
      finalSource: finalSource,
      finalLifecycle: _string(expected['lifecycle'], 'expected.lifecycle'),
      checkpointCount: checkpointCount,
    );
  }
}

final class _TimerPlaybackClock implements GoldenPlaybackClock {
  Timer? _timer;
  Completer<void>? _pending;

  @override
  Future<void> wait(Duration duration) {
    cancel();
    final pending = Completer<void>();
    _pending = pending;
    _timer = Timer(duration, () {
      _timer = null;
      _pending = null;
      pending.complete();
    });
    return pending.future;
  }

  @override
  void cancel() {
    _timer?.cancel();
    _timer = null;
    final pending = _pending;
    _pending = null;
    if (pending != null && !pending.isCompleted) {
      pending.complete();
    }
  }
}

Map<String, Object?> _record(Object? value, String field) {
  if (value is! Map) {
    throw FormatException('$field must be an object.');
  }
  return Map<String, Object?>.from(value);
}

List<Object?> _list(Object? value, String field) {
  if (value is! List<Object?>) {
    throw FormatException('$field must be an array.');
  }
  return value;
}

String _string(Object? value, String field) {
  if (value is! String || value.isEmpty) {
    throw FormatException('$field must be a non-empty string.');
  }
  return value;
}

int _integer(Object? value, String field) {
  if (value is! int || value < 0) {
    throw FormatException('$field must be a non-negative integer.');
  }
  return value;
}
