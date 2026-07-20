// ignore_for_file: public_member_api_docs

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

typedef GoldenNodeBuildObserver = void Function(NodeId id);

final class ContentIrView extends StatefulWidget {
  const ContentIrView({
    required this.controller,
    required this.motionDuration,
    this.onNodeBuild,
    super.key,
  });

  final MdstreamController controller;
  final Duration motionDuration;
  final GoldenNodeBuildObserver? onNodeBuild;

  @override
  State<ContentIrView> createState() => _ContentIrViewState();
}

final class _ContentIrViewState extends State<ContentIrView> {
  late List<NodeId> _rootIds;
  final Map<MdstreamNodeKey, Widget> _rootWidgets = <MdstreamNodeKey, Widget>{};

  @override
  void initState() {
    super.initState();
    _rootIds = _readRoots();
    widget.controller.addListener(_handleControllerChange);
  }

  @override
  void didUpdateWidget(ContentIrView oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!identical(oldWidget.controller, widget.controller)) {
      oldWidget.controller.removeListener(_handleControllerChange);
      _rootWidgets.clear();
      _rootIds = _readRoots();
      widget.controller.addListener(_handleControllerChange);
    } else if (oldWidget.motionDuration != widget.motionDuration ||
        oldWidget.onNodeBuild != widget.onNodeBuild) {
      _rootWidgets.clear();
    }
  }

  @override
  void dispose() {
    widget.controller.removeListener(_handleControllerChange);
    super.dispose();
  }

  void _handleControllerChange() {
    final impact = widget.controller.value.impact;
    if (!impact.rootsChanged && !impact.fullReplace) {
      return;
    }
    final next = _readRoots();
    if (!impact.fullReplace && listEquals(next, _rootIds)) {
      return;
    }
    if (impact.fullReplace) {
      _rootWidgets.clear();
    }
    final retainedKeys = next.map(widget.controller.nodeKey).toSet();
    _rootWidgets.removeWhere((key, _) => !retainedKeys.contains(key));
    setState(() => _rootIds = next);
  }

  List<NodeId> _readRoots() => List<NodeId>.of(
    widget.controller.value.document?.roots?.children ?? const <NodeId>[],
  );

  @override
  Widget build(BuildContext context) {
    if (_rootIds.isEmpty) {
      return const SizedBox(
        height: 180,
        child: Center(child: Text('Ready for content')),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: <Widget>[
        for (final id in _rootIds) _rootWidget(id),
        ValueListenableBuilder<PendingSourceView?>(
          valueListenable: widget.controller.pendingSource,
          builder: (context, pending, _) {
            if (pending == null) {
              return const SizedBox.shrink();
            }
            return Semantics(
              label: 'Pending source',
              liveRegion: false,
              child: AnimatedDefaultTextStyle(
                duration: widget.motionDuration,
                curve: Curves.easeOutCubic,
                style: Theme.of(context).textTheme.bodyLarge!.copyWith(
                  color: Theme.of(context).colorScheme.tertiary,
                  height: 1.55,
                ),
                child: Text(pending.text),
              ),
            );
          },
        ),
      ],
    );
  }

  Widget _rootWidget(NodeId id) {
    final key = widget.controller.nodeKey(id);
    return _rootWidgets.putIfAbsent(
      key,
      () => Padding(
        padding: const EdgeInsets.only(bottom: 18),
        child: _FocusedNode(
          key: key,
          controller: widget.controller,
          nodeId: id,
          motionDuration: widget.motionDuration,
          onNodeBuild: widget.onNodeBuild,
        ),
      ),
    );
  }
}

final class _FocusedNode extends StatefulWidget {
  const _FocusedNode({
    required this.controller,
    required this.nodeId,
    required this.motionDuration,
    this.onNodeBuild,
    super.key,
  });

  final MdstreamController controller;
  final NodeId nodeId;
  final Duration motionDuration;
  final GoldenNodeBuildObserver? onNodeBuild;

  @override
  State<_FocusedNode> createState() => _FocusedNodeState();
}

final class _FocusedNodeState extends State<_FocusedNode> {
  final Map<MdstreamNodeKey, Widget> _children = <MdstreamNodeKey, Widget>{};

  @override
  void didUpdateWidget(_FocusedNode oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!identical(oldWidget.controller, widget.controller) ||
        oldWidget.motionDuration != widget.motionDuration ||
        oldWidget.onNodeBuild != widget.onNodeBuild) {
      _children.clear();
    }
  }

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<NodeView?>(
    valueListenable: widget.controller.node(widget.nodeId),
    builder: (context, view, _) {
      widget.onNodeBuild?.call(widget.nodeId);
      if (view == null) {
        _children.clear();
        return const SizedBox.shrink();
      }
      final retainedKeys = view.node.children.children
          .map(widget.controller.nodeKey)
          .toSet();
      _children.removeWhere((key, _) => !retainedKeys.contains(key));
      return _NodePresentation(
        controller: widget.controller,
        view: view,
        motionDuration: widget.motionDuration,
        childBuilder: _child,
      );
    },
  );

  Widget _child(NodeId id) {
    final key = widget.controller.nodeKey(id);
    return _children.putIfAbsent(
      key,
      () => _FocusedNode(
        key: key,
        controller: widget.controller,
        nodeId: id,
        motionDuration: widget.motionDuration,
        onNodeBuild: widget.onNodeBuild,
      ),
    );
  }
}

final class _NodePresentation extends StatelessWidget {
  const _NodePresentation({
    required this.controller,
    required this.view,
    required this.motionDuration,
    required this.childBuilder,
  });

  final MdstreamController controller;
  final NodeView view;
  final Duration motionDuration;
  final Widget Function(NodeId id) childBuilder;

  @override
  Widget build(BuildContext context) {
    final node = view.node;
    final content = node.content;
    final stable = node.stability == 'stable';
    final colorScheme = Theme.of(context).colorScheme;
    final foreground = stable ? colorScheme.onSurface : colorScheme.tertiary;
    final body = switch (content) {
      HeadingContentView(:final level) => _AnimatedContentText(
        text: view.bodyText,
        style: _headingStyle(context, level).copyWith(color: foreground),
        duration: motionDuration,
      ),
      ParagraphContentView() => _childrenOrText(
        context,
        fallback: view.bodyText,
        inline: true,
      ),
      TextContentView() => _AnimatedContentText(
        text: view.bodyText,
        style: Theme.of(
          context,
        ).textTheme.bodyLarge!.copyWith(color: foreground, height: 1.55),
        duration: motionDuration,
      ),
      CodeBlockContentView(:final info) => _CodeSourceBlock(
        nodeId: node.id,
        language: info,
        source: view.bodyText,
        stable: stable,
        duration: motionDuration,
      ),
      CitationReferenceContentView(:final key, :final target) =>
        _CitationReference(
          controller: controller,
          citationKey: key,
          target: target,
        ),
      CitationDefinitionContentView(:final key, :final target) =>
        _CitationDefinition(
          controller: controller,
          citationKey: key,
          target: target,
        ),
      SoftBreakContentView() => const SizedBox(width: 4),
      HardBreakContentView() => const SizedBox(width: double.infinity),
      _ => _childrenOrText(context, fallback: view.bodyText),
    };
    return Semantics(container: content is CodeBlockContentView, child: body);
  }

  Widget _childrenOrText(
    BuildContext context, {
    required String fallback,
    bool inline = false,
  }) {
    final children = view.node.children.children;
    if (children.isEmpty) {
      return _AnimatedContentText(
        text: fallback,
        style: Theme.of(context).textTheme.bodyLarge!.copyWith(height: 1.55),
        duration: motionDuration,
      );
    }
    final widgets = <Widget>[
      for (final childId in children) childBuilder(childId),
    ];
    if (inline) {
      return Wrap(
        spacing: 0,
        runSpacing: 2,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: widgets,
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: widgets,
    );
  }
}

final class _AnimatedContentText extends StatelessWidget {
  const _AnimatedContentText({
    required this.text,
    required this.style,
    required this.duration,
  });

  final String text;
  final TextStyle style;
  final Duration duration;

  @override
  Widget build(BuildContext context) => AnimatedDefaultTextStyle(
    duration: duration,
    curve: Curves.easeOutCubic,
    style: style,
    child: Text(text, softWrap: true),
  );
}

final class _CodeSourceBlock extends StatelessWidget {
  const _CodeSourceBlock({
    required this.nodeId,
    required this.language,
    required this.source,
    required this.stable,
    required this.duration,
  });

  final NodeId nodeId;
  final String? language;
  final String source;
  final bool stable;
  final Duration duration;

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final normalizedLanguage = language?.trim().toLowerCase();
    final label = normalizedLanguage == 'mermaid'
        ? 'Mermaid source'
        : (normalizedLanguage?.isNotEmpty ?? false)
        ? normalizedLanguage!
        : 'Code';
    return DecoratedBox(
      decoration: BoxDecoration(
        color: colorScheme.surfaceContainerHighest,
        border: Border.all(color: colorScheme.outlineVariant),
        borderRadius: BorderRadius.circular(6),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: <Widget>[
          Padding(
            padding: const EdgeInsets.fromLTRB(14, 10, 14, 8),
            child: Row(
              children: <Widget>[
                Icon(
                  normalizedLanguage == 'mermaid'
                      ? Icons.account_tree_outlined
                      : Icons.code,
                  size: 17,
                  color: colorScheme.secondary,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    label,
                    style: Theme.of(context).textTheme.labelLarge,
                  ),
                ),
                AnimatedContainer(
                  duration: duration,
                  width: 7,
                  height: 7,
                  decoration: BoxDecoration(
                    color: stable ? colorScheme.primary : colorScheme.tertiary,
                    shape: BoxShape.circle,
                  ),
                ),
              ],
            ),
          ),
          Divider(height: 1, color: colorScheme.outlineVariant),
          SingleChildScrollView(
            key: ValueKey<String>('golden-code-scroll-$nodeId'),
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.all(14),
            child: AnimatedDefaultTextStyle(
              duration: duration,
              curve: Curves.easeOutCubic,
              style: Theme.of(context).textTheme.bodyMedium!.copyWith(
                color: stable ? colorScheme.onSurface : colorScheme.tertiary,
                fontFamily: 'monospace',
                height: 1.5,
              ),
              child: Text(source),
            ),
          ),
        ],
      ),
    );
  }
}

final class _CitationReference extends StatelessWidget {
  const _CitationReference({
    required this.controller,
    required this.citationKey,
    required this.target,
  });

  final MdstreamController controller;
  final String citationKey;
  final ResourceRefView? target;

  @override
  Widget build(BuildContext context) {
    final reference = target;
    if (reference == null) {
      return _CitationLabel(label: citationKey, destination: null);
    }
    return ValueListenableBuilder<ResourceView?>(
      valueListenable: controller.resource(reference.id),
      builder: (context, resource, _) {
        final content = resource?.resource.content;
        final destination = content is CitationResourceContentView
            ? _allowedDestination(content.destination)
            : null;
        return _CitationLabel(label: citationKey, destination: destination);
      },
    );
  }
}

final class _CitationDefinition extends StatelessWidget {
  const _CitationDefinition({
    required this.controller,
    required this.citationKey,
    required this.target,
  });

  final MdstreamController controller;
  final String citationKey;
  final ResourceRefView target;

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<ResourceView?>(
    valueListenable: controller.resource(target.id),
    builder: (context, resource, _) {
      final content = resource?.resource.content;
      final destination = content is CitationResourceContentView
          ? _allowedDestination(content.destination)
          : null;
      return Text(
        destination == null ? '[$citationKey]' : '[$citationKey] $destination',
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: Theme.of(context).colorScheme.onSurfaceVariant,
        ),
      );
    },
  );
}

final class _CitationLabel extends StatelessWidget {
  const _CitationLabel({required this.label, required this.destination});

  final String label;
  final String? destination;

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    return Semantics(
      label: destination == null
          ? 'Unresolved citation $label'
          : 'Citation $label, $destination',
      child: Tooltip(
        message: destination ?? 'Citation pending',
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: colorScheme.secondaryContainer,
            borderRadius: BorderRadius.circular(4),
          ),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 3),
            child: Text(
              '[$label]',
              style: Theme.of(context).textTheme.labelMedium?.copyWith(
                color: colorScheme.onSecondaryContainer,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

String? _allowedDestination(String value) {
  final uri = Uri.tryParse(value);
  if (uri == null || !uri.hasAuthority) {
    return null;
  }
  return switch (uri.scheme.toLowerCase()) {
    'http' || 'https' => uri.toString(),
    _ => null,
  };
}

TextStyle _headingStyle(BuildContext context, int level) {
  final textTheme = Theme.of(context).textTheme;
  return switch (level) {
    1 => textTheme.headlineMedium!,
    2 => textTheme.headlineSmall!,
    _ => textTheme.titleLarge!,
  };
}
