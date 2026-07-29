import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'dispose cancels processor work and ignores its late completion',
    () async {
      final runtime = MdstreamRuntime.openPath(libraryPath!);
      final controller = MdstreamController.fromRuntime(runtime);
      final processor = _DeferredProcessor();
      controller.registerProcessor(processor);
      var notifications = 0;
      controller.addListener(() => notifications += 1);

      controller.append('late processor');
      controller.finish();
      await processor.started.future;
      final notificationsBeforeDispose = notifications;
      final idle = controller.whenProcessorsIdle();

      controller.dispose();
      controller.dispose();
      expect(processor.context?.isCancelled, isTrue);
      processor.output.complete(
        const ProcessorTextOutput(
          protocol: 'test.flutter.late/1',
          mediaType: 'text/plain',
          text: 'too late',
        ),
      );
      await idle;

      expect(notifications, notificationsBeforeDispose);
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run ../dart/tool/build_native.dart first'
        : false,
  );
}

final class _DeferredProcessor implements ContentProcessor {
  final Completer<void> started = Completer<void>();
  final Completer<ProcessorOutput> output = Completer<ProcessorOutput>();
  ProcessorContext? context;

  @override
  ContentProcessorDescriptor get descriptor =>
      const ContentProcessorDescriptor(id: 'test.flutter.late', version: 'v1');

  @override
  String get configurationVersion => 'default-v1';

  @override
  bool get allowProvisional => false;

  @override
  bool matches(ContentNodeView node) => node.content is ParagraphContentView;

  @override
  FutureOr<ProcessorOutput> process(
    ProcessorRequestView request,
    ProcessorContext context,
  ) {
    this.context = context;
    started.complete();
    return output.future;
  }
}
