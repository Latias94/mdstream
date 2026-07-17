import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('bundled runtime loads and completes the shared smoke trace', (
    tester,
  ) async {
    expect(Platform.environment['MDSTREAM_NATIVE_LIBRARY'], isNull);
    expect(Platform.environment['MDSTREAM_FFI_LIBRARY'], isNull);

    final runtime = MdstreamFlutterRuntime.open();
    expect(runtime.abiVersion, 1);
    expect(runtime.packageVersion, '0.4.0');
    expect(runtime.bindingSchema, 'mdstream.bindings/0.4');

    final controller = MdstreamController.fromRuntime(runtime);
    try {
      controller.append('# Bundled runtime\n\nstreamed content');
      controller.finish();

      expect(controller.value.isFinalized, isTrue);
      final roots = controller.value.document?.roots?.children;
      expect(roots, isNotNull);
      expect(roots, isNotEmpty);
      expect(controller.node(roots!.first).value, isNotNull);
    } finally {
      controller.dispose();
    }
    expect(runtime.nativeAllocations.isZero, isTrue);
  });
}
