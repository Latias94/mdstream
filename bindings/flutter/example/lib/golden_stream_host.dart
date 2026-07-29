// ignore_for_file: public_member_api_docs

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

import 'bootstrap.dart';
import 'content_ir_view.dart';

const goldenControlsKey = ValueKey<String>('golden-controls');
const goldenAnswerKey = ValueKey<String>('golden-answer');
const goldenInspectorKey = ValueKey<String>('golden-inspector');
const goldenRetryKey = ValueKey<String>('golden-retry');
const goldenReplayKey = ValueKey<String>('golden-replay');
const goldenInterruptKey = ValueKey<String>('golden-interrupt');
const goldenStatusKey = ValueKey<String>('golden-status');
const goldenTransitionStatusKey = ValueKey<String>('golden-transition-status');

final class GoldenStreamExample extends StatefulWidget {
  const GoldenStreamExample({
    required this.bootstrap,
    this.autoPlay = true,
    this.onNodeBuild,
    super.key,
  });

  final GoldenStreamBootstrap bootstrap;
  final bool autoPlay;
  final GoldenNodeBuildObserver? onNodeBuild;

  @override
  State<GoldenStreamExample> createState() => _GoldenStreamExampleState();
}

final class _GoldenStreamExampleState extends State<GoldenStreamExample> {
  late GoldenStreamBootstrap _bootstrap;
  int _bootstrapGeneration = 0;

  @override
  void initState() {
    super.initState();
    _bootstrap = widget.bootstrap;
    _scheduleInitialization();
  }

  @override
  void didUpdateWidget(GoldenStreamExample oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (identical(_bootstrap, widget.bootstrap)) {
      return;
    }

    final previous = _bootstrap;
    _bootstrap = widget.bootstrap;
    previous.dispose();
    _scheduleInitialization();
  }

  void _scheduleInitialization() {
    final bootstrap = _bootstrap;
    final generation = ++_bootstrapGeneration;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted ||
          generation != _bootstrapGeneration ||
          !identical(_bootstrap, bootstrap)) {
        return;
      }
      unawaited(
        bootstrap.initialize(
          autoPlay: widget.autoPlay,
          reducedMotion: MediaQuery.disableAnimationsOf(context),
        ),
      );
    });
  }

  @override
  void dispose() {
    _bootstrapGeneration += 1;
    _bootstrap.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => MaterialApp(
    debugShowCheckedModeBanner: false,
    title: 'mdstream Flutter host',
    theme: _theme(Brightness.light),
    darkTheme: _theme(Brightness.dark),
    home: GoldenStreamHost(
      bootstrap: _bootstrap,
      onNodeBuild: widget.onNodeBuild,
    ),
  );
}

ThemeData _theme(Brightness brightness) {
  final dark = brightness == Brightness.dark;
  final colors = ColorScheme(
    brightness: brightness,
    primary: dark ? const Color(0xff7dd6b1) : const Color(0xff006b52),
    onPrimary: dark ? const Color(0xff003829) : Colors.white,
    primaryContainer: dark ? const Color(0xff00513d) : const Color(0xff9ff2d1),
    onPrimaryContainer: dark
        ? const Color(0xff9ff2d1)
        : const Color(0xff002117),
    secondary: dark ? const Color(0xffffb59f) : const Color(0xff994527),
    onSecondary: dark ? const Color(0xff581d08) : Colors.white,
    secondaryContainer: dark
        ? const Color(0xff793010)
        : const Color(0xffffdbcf),
    onSecondaryContainer: dark
        ? const Color(0xffffdbcf)
        : const Color(0xff3a0b00),
    tertiary: dark ? const Color(0xffffc869) : const Color(0xff765900),
    onTertiary: dark ? const Color(0xff3e2e00) : Colors.white,
    tertiaryContainer: dark ? const Color(0xff594300) : const Color(0xffffdf91),
    onTertiaryContainer: dark
        ? const Color(0xffffdf91)
        : const Color(0xff251a00),
    error: dark ? const Color(0xffffb4ab) : const Color(0xffba1a1a),
    onError: dark ? const Color(0xff690005) : Colors.white,
    errorContainer: dark ? const Color(0xff93000a) : const Color(0xffffdad6),
    onErrorContainer: dark ? const Color(0xffffdad6) : const Color(0xff410002),
    surface: dark ? const Color(0xff111412) : const Color(0xfffbfdf9),
    onSurface: dark ? const Color(0xffe1e3df) : const Color(0xff191c1a),
    surfaceContainerHighest: dark
        ? const Color(0xff343936)
        : const Color(0xffdfe4e0),
    onSurfaceVariant: dark ? const Color(0xffc0c9c3) : const Color(0xff404944),
    outline: dark ? const Color(0xff8a938e) : const Color(0xff707974),
    outlineVariant: dark ? const Color(0xff404944) : const Color(0xffc0c9c3),
    shadow: Colors.black,
    scrim: Colors.black,
    inverseSurface: dark ? const Color(0xffe1e3df) : const Color(0xff2e312f),
    onInverseSurface: dark ? const Color(0xff2e312f) : const Color(0xfff0f1ee),
    inversePrimary: dark ? const Color(0xff006b52) : const Color(0xff7dd6b1),
    surfaceTint: dark ? const Color(0xff7dd6b1) : const Color(0xff006b52),
  );
  return ThemeData(
    colorScheme: colors,
    useMaterial3: true,
    scaffoldBackgroundColor: colors.surface,
    textTheme: ThemeData(brightness: brightness).textTheme.apply(
      bodyColor: colors.onSurface,
      displayColor: colors.onSurface,
      fontFamily: 'system-ui',
    ),
    cardTheme: const CardThemeData(margin: EdgeInsets.zero),
  );
}

final class GoldenStreamHost extends StatefulWidget {
  const GoldenStreamHost({
    required this.bootstrap,
    this.onNodeBuild,
    super.key,
  });

  final GoldenStreamBootstrap bootstrap;
  final GoldenNodeBuildObserver? onNodeBuild;

  @override
  State<GoldenStreamHost> createState() => _GoldenStreamHostState();
}

final class _GoldenStreamHostState extends State<GoldenStreamHost> {
  final FocusNode _retryFocus = FocusNode(debugLabel: 'Retry stream');

  @override
  void initState() {
    super.initState();
    widget.bootstrap.addListener(_handleBootstrapChange);
  }

  @override
  void didUpdateWidget(GoldenStreamHost oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!identical(oldWidget.bootstrap, widget.bootstrap)) {
      oldWidget.bootstrap.removeListener(_handleBootstrapChange);
      widget.bootstrap.addListener(_handleBootstrapChange);
    }
  }

  @override
  void dispose() {
    widget.bootstrap.removeListener(_handleBootstrapChange);
    _retryFocus.dispose();
    super.dispose();
  }

  void _handleBootstrapChange() {
    if (widget.bootstrap.phase == GoldenHostPhase.error) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) {
          _retryFocus.requestFocus();
        }
      });
    }
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
    animation: widget.bootstrap,
    builder: (context, _) {
      final bootstrap = widget.bootstrap;
      final reducedMotion = MediaQuery.disableAnimationsOf(context);
      return Scaffold(
        appBar: AppBar(
          title: const Text('mdstream'),
          actions: <Widget>[
            Padding(
              padding: const EdgeInsets.only(right: 16),
              child: Center(
                child: Text(
                  bootstrap.runtime?.packageVersion ?? 'Flutter host',
                  style: Theme.of(context).textTheme.labelMedium,
                ),
              ),
            ),
          ],
        ),
        body: SafeArea(
          child: SingleChildScrollView(
            padding: const EdgeInsets.fromLTRB(20, 16, 20, 40),
            child: Align(
              alignment: Alignment.topCenter,
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 1180),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: <Widget>[
                    _PlaybackControls(
                      key: goldenControlsKey,
                      bootstrap: bootstrap,
                      reducedMotion: reducedMotion,
                    ),
                    const SizedBox(height: 24),
                    _buildBody(bootstrap, reducedMotion),
                  ],
                ),
              ),
            ),
          ),
        ),
      );
    },
  );

  Widget _buildBody(GoldenStreamBootstrap bootstrap, bool reducedMotion) {
    switch (bootstrap.phase) {
      case GoldenHostPhase.booting:
        return const SizedBox(
          height: 300,
          child: Center(child: CircularProgressIndicator()),
        );
      case GoldenHostPhase.error:
        return _ErrorState(
          message: bootstrap.errorLabel,
          retryFocus: _retryFocus,
          onRetry: () =>
              unawaited(bootstrap.retry(reducedMotion: reducedMotion)),
        );
      case GoldenHostPhase.readyEmpty ||
          GoldenHostPhase.streaming ||
          GoldenHostPhase.draining ||
          GoldenHostPhase.settled ||
          GoldenHostPhase.interrupted:
        final controller = bootstrap.controller;
        if (controller == null) {
          return const SizedBox.shrink();
        }
        final duration =
            bootstrap.mode == GoldenPlaybackMode.paced && !reducedMotion
            ? const Duration(milliseconds: 180)
            : Duration.zero;
        return LayoutBuilder(
          builder: (context, constraints) {
            final answer = _AnswerPane(
              key: goldenAnswerKey,
              controller: controller,
              motionDuration: duration,
              onNodeBuild: widget.onNodeBuild,
            );
            final inspector = _StreamInspector(
              key: goldenInspectorKey,
              controller: controller,
              checkpoint: bootstrap.finalCheckpoint,
            );
            if (constraints.maxWidth >= 900) {
              return Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: <Widget>[
                  Expanded(flex: 3, child: answer),
                  const SizedBox(width: 28),
                  SizedBox(width: 330, child: inspector),
                ],
              );
            }
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: <Widget>[answer, const SizedBox(height: 28), inspector],
            );
          },
        );
    }
  }
}

final class _PlaybackControls extends StatelessWidget {
  const _PlaybackControls({
    required this.bootstrap,
    required this.reducedMotion,
    super.key,
  });

  final GoldenStreamBootstrap bootstrap;
  final bool reducedMotion;

  @override
  Widget build(BuildContext context) {
    final enabled =
        bootstrap.phase != GoldenHostPhase.booting &&
        bootstrap.phase != GoldenHostPhase.error;
    return Wrap(
      spacing: 12,
      runSpacing: 12,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: <Widget>[
        Semantics(
          key: goldenReplayKey,
          button: true,
          enabled: enabled,
          label: 'Replay stream',
          child: ExcludeSemantics(
            child: IconButton.filledTonal(
              tooltip: 'Replay stream',
              onPressed: enabled
                  ? () => unawaited(
                      bootstrap.replay(
                        mode: bootstrap.mode,
                        reducedMotion: reducedMotion,
                      ),
                    )
                  : null,
              icon: const Icon(Icons.replay),
            ),
          ),
        ),
        Semantics(
          key: goldenInterruptKey,
          button: true,
          enabled:
              bootstrap.phase == GoldenHostPhase.streaming ||
              bootstrap.phase == GoldenHostPhase.draining,
          label: 'Interrupt stream',
          child: ExcludeSemantics(
            child: IconButton(
              tooltip: 'Interrupt stream',
              onPressed:
                  bootstrap.phase == GoldenHostPhase.streaming ||
                      bootstrap.phase == GoldenHostPhase.draining
                  ? bootstrap.interrupt
                  : null,
              icon: const Icon(Icons.stop_circle_outlined),
            ),
          ),
        ),
        SegmentedButton<GoldenPlaybackMode>(
          segments: const <ButtonSegment<GoldenPlaybackMode>>[
            ButtonSegment<GoldenPlaybackMode>(
              value: GoldenPlaybackMode.immediate,
              icon: Icon(Icons.flash_on_outlined),
              label: Text('Immediate'),
            ),
            ButtonSegment<GoldenPlaybackMode>(
              value: GoldenPlaybackMode.paced,
              icon: Icon(Icons.motion_photos_on_outlined),
              label: Text('Paced'),
            ),
          ],
          selected: <GoldenPlaybackMode>{bootstrap.mode},
          onSelectionChanged: enabled
              ? (selection) => unawaited(
                  bootstrap.replay(
                    mode: selection.single,
                    reducedMotion: reducedMotion,
                  ),
                )
              : null,
        ),
        Semantics(
          key: goldenStatusKey,
          container: true,
          liveRegion: true,
          label: 'Stream status: ${bootstrap.statusLabel}',
          child: ExcludeSemantics(
            child: Chip(
              avatar: Icon(_phaseIcon(bootstrap.phase), size: 16),
              label: Text(bootstrap.statusLabel),
            ),
          ),
        ),
      ],
    );
  }
}

final class _AnswerPane extends StatelessWidget {
  const _AnswerPane({
    required this.controller,
    required this.motionDuration,
    this.onNodeBuild,
    super.key,
  });

  final MdstreamController controller;
  final Duration motionDuration;
  final GoldenNodeBuildObserver? onNodeBuild;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: <Widget>[
      Text('Golden AI answer', style: Theme.of(context).textTheme.titleLarge),
      const SizedBox(height: 16),
      ContentIrView(
        controller: controller,
        motionDuration: motionDuration,
        onNodeBuild: onNodeBuild,
      ),
    ],
  );
}

final class _StreamInspector extends StatelessWidget {
  const _StreamInspector({
    required this.controller,
    required this.checkpoint,
    super.key,
  });

  final MdstreamController controller;
  final GoldenStreamCheckpoint? checkpoint;

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    return DecoratedBox(
      decoration: BoxDecoration(
        border: Border.all(color: colors.outlineVariant),
        borderRadius: BorderRadius.circular(8),
      ),
      child: ExpansionTile(
        initiallyExpanded: true,
        leading: const Icon(Icons.data_object),
        title: const Text('Content IR'),
        shape: const Border(),
        collapsedShape: const Border(),
        childrenPadding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
        children: <Widget>[
          ValueListenableBuilder<MdstreamControllerState>(
            valueListenable: controller,
            builder: (context, state, _) {
              final roots = state.document?.roots?.children ?? const <NodeId>[];
              return _InspectorRow(
                label: 'Root identities',
                value: roots.isEmpty ? 'None' : roots.join('\n'),
              );
            },
          ),
          const SizedBox(height: 12),
          ValueListenableBuilder<PendingSourceView?>(
            valueListenable: controller.pendingSource,
            builder: (context, pending, _) => _InspectorRow(
              label: 'Pending source',
              value: pending == null
                  ? 'None'
                  : '${pending.range.start}..${pending.range.end}',
            ),
          ),
          const SizedBox(height: 12),
          ValueListenableBuilder<MdstreamTransitionBatch>(
            valueListenable: controller.transitions,
            builder: (context, batch, _) => _InspectorRow(
              valueKey: goldenTransitionStatusKey,
              label: 'Latest transition',
              value: _transitionLabel(batch),
            ),
          ),
          if (checkpoint != null) ...<Widget>[
            const SizedBox(height: 12),
            _InspectorRow(
              label: 'Final checkpoint',
              value: '${checkpoint!.id} · ${checkpoint!.sourceCursor}',
            ),
          ],
        ],
      ),
    );
  }
}

final class _InspectorRow extends StatelessWidget {
  const _InspectorRow({
    required this.label,
    required this.value,
    this.valueKey,
  });

  final String label;
  final String value;
  final Key? valueKey;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: <Widget>[
      Text(label, style: Theme.of(context).textTheme.labelMedium),
      const SizedBox(height: 4),
      Semantics(
        key: valueKey,
        label: value,
        child: ExcludeSemantics(
          child: SelectableText(
            value,
            style: Theme.of(
              context,
            ).textTheme.bodySmall?.copyWith(fontFamily: 'monospace'),
          ),
        ),
      ),
    ],
  );
}

final class _ErrorState extends StatelessWidget {
  const _ErrorState({
    required this.message,
    required this.retryFocus,
    required this.onRetry,
  });

  final String message;
  final FocusNode retryFocus;
  final VoidCallback onRetry;

  @override
  Widget build(BuildContext context) => Semantics(
    container: true,
    label: 'Unable to start the stream. $message',
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: 64),
      child: Column(
        children: <Widget>[
          Icon(
            Icons.error_outline,
            size: 36,
            color: Theme.of(context).colorScheme.error,
          ),
          const SizedBox(height: 16),
          Text(
            'Unable to start the stream',
            style: Theme.of(context).textTheme.titleLarge,
          ),
          const SizedBox(height: 8),
          Text(message, textAlign: TextAlign.center),
          const SizedBox(height: 20),
          FilledButton.icon(
            key: goldenRetryKey,
            focusNode: retryFocus,
            onPressed: onRetry,
            icon: const Icon(Icons.refresh),
            label: const Text('Retry'),
          ),
        ],
      ),
    ),
  );
}

String _transitionLabel(MdstreamTransitionBatch batch) {
  if (batch.revision == 0) {
    return 'None';
  }
  var replacement = false;
  var semanticCorrection = false;
  var appended = false;
  var fullReplace = false;
  for (final facts in batch.facts) {
    switch (facts) {
      case FullReplaceTransitionFactsView():
        fullReplace = true;
      case ContinuousTransitionFactsView(:final nodes, :final resources):
        for (final node in nodes) {
          replacement =
              replacement || node.text is ReplacementTextTransitionView;
          appended = appended || node.text is ProjectionAppendTransitionView;
        }
        semanticCorrection =
            semanticCorrection ||
            resources.any((resource) => resource.affectedNodes.isNotEmpty);
    }
  }
  final kind = replacement || semanticCorrection
      ? 'Correction / replacement'
      : fullReplace
      ? 'Full replacement'
      : appended
      ? 'Text append'
      : 'Structural update';
  return 'r${batch.revision} · $kind';
}

IconData _phaseIcon(GoldenHostPhase phase) => switch (phase) {
  GoldenHostPhase.booting => Icons.hourglass_top,
  GoldenHostPhase.readyEmpty => Icons.pause_circle_outline,
  GoldenHostPhase.streaming => Icons.stream,
  GoldenHostPhase.draining => Icons.pending_outlined,
  GoldenHostPhase.settled => Icons.check_circle_outline,
  GoldenHostPhase.interrupted => Icons.stop_circle_outlined,
  GoldenHostPhase.error => Icons.error_outline,
};
