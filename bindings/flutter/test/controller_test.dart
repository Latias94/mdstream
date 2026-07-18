import 'package:flutter_test/flutter_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'pending source is a lazy focused listenable with byte-accurate ranges',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      try {
        controller.append('a *b');
        final pending = controller.pendingSource;
        expect(controller.pendingSource, same(pending));
        expect(pending.value, isNull);
        var notifications = 0;
        pending.addListener(() => notifications += 1);

        controller.append('*');
        expect(notifications, 1);
        final first = pending.value;
        expect(first?.range.start, '4');
        expect(first?.range.end, '5');
        expect(first?.text, '*');
        expect(controller.pendingSourceView(), same(first));

        controller.append('é');
        expect(notifications, 2);
        expect(pending.value, isNot(same(first)));
        expect(pending.value?.range.start, '4');
        expect(pending.value?.range.end, '7');
        expect(pending.value?.text, '*é');

        controller.append('');
        expect(notifications, 2);
        controller.finish();
        expect(notifications, 3);
        expect(pending.value, isNull);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'local controller preserves stable node keys and notifies only changed views',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      var rootNotifications = 0;
      controller.addListener(() => rootNotifications += 1);

      try {
        controller.append('first paragraph\n\nsecond');
        expect(rootNotifications, 1);
        final roots = controller.value.document!.roots!.children;
        expect(roots, hasLength(2));
        final firstId = roots[0];
        final secondId = roots[1];
        final firstKey = controller.nodeKey(firstId);
        final first = controller.node(firstId);
        final second = controller.node(secondId);
        expect(controller.node(firstId), same(first));
        final firstView = first.value;
        final secondView = second.value;
        var firstNotifications = 0;
        var secondNotifications = 0;
        MdstreamControllerState? stateSeenBySecond;
        first.addListener(() => firstNotifications += 1);
        second.addListener(() {
          secondNotifications += 1;
          stateSeenBySecond = controller.value;
        });

        controller.append(' paragraph');

        expect(stateSeenBySecond, same(controller.value));
        expect(controller.value.document!.roots!.children, <NodeId>[
          firstId,
          secondId,
        ]);
        expect(first.value, same(firstView));
        expect(second.value, isNot(same(secondView)));
        expect(firstNotifications, 0);
        expect(secondNotifications, 1);
        expect(rootNotifications, 2);

        final stateBeforeEmpty = controller.value;
        controller.append('');
        expect(controller.value, same(stateBeforeEmpty));
        expect(rootNotifications, 2);

        controller.finish();
        final notificationsAfterFinish = rootNotifications;
        controller.finish();
        expect(rootNotifications, notificationsAfterFinish);

        controller.reset();
        expect(first.value, isNull);
        expect(
          controller.value.document!.coordinate.epoch,
          isNot(firstKey.epoch),
        );
        expect(
          MdstreamNodeKey(
            epoch: controller.value.document!.coordinate.epoch,
            nodeId: firstId,
          ),
          isNot(firstKey),
        );
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'terminal errors are structured without replacing last-good canonical state',
    () {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      try {
        controller.append('done');
        controller.finish();
        final lastGood = controller.value.snapshot;

        expect(
          () => controller.append('late'),
          throwsA(isA<MdstreamException>()),
        );
        expect(controller.value.snapshot, same(lastGood));
        expect(
          controller.value.lastError?.phase,
          MdstreamControllerErrorPhase.append,
        );
        expect(controller.value.lastError?.error, isA<MdstreamException>());
        expect(controller.value.lastError?.error.status, 6);
        expect(
          controller.value.lastError?.error.statusName,
          'MDSTREAM_TERMINAL',
        );
        expect(controller.value.lastError?.error.detailCode, 'engine.finished');
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );

  test(
    'citation updates notify only the targeted resource',
    () {
      const resourceId = '154582791709149689190109869243805354114';
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      try {
        controller.append('# Adoption\n\n');
        final resource = controller.resource(resourceId);
        final unrelated = controller.resource('999');
        var resourceNotifications = 0;
        var unrelatedNotifications = 0;
        resource.addListener(() => resourceNotifications += 1);
        unrelated.addListener(() => unrelatedNotifications += 1);

        final result = controller.append(
          'See [@Engine] while this diagram streams.\n\n'
          '```mermaid\nflowchart LR\n  Token --> IR\n```\n\n'
          '[@engine]: https://mdstream.dev/engine "mdstream"\n',
        );

        expect(result.updates.single.impact.changedResourceIds, <ResourceId>[
          resourceId,
        ]);
        expect(
          resource.value?.resource.content,
          isA<CitationResourceContentView>(),
        );
        expect(resourceNotifications, 1);
        expect(unrelatedNotifications, 0);
      } finally {
        controller.dispose();
      }
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );
}
