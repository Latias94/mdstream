import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';

import '../tool/runtime_smoke_probe.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('bundled runtime loads and completes the shared smoke trace', (
    tester,
  ) async {
    final report = runBundledRuntimeSmoke();
    expect(report.abiVersion, runtimeSmokeAbiVersion);
    expect(report.packageVersion, runtimeSmokePackageVersion);
    expect(report.bindingSchema, runtimeSmokeBindingSchema);
    expect(report.isFinalized, isTrue);
    expect(report.hasRootNode, isTrue);
    expect(report.nativeAllocationsZero, isTrue);
  });
}
