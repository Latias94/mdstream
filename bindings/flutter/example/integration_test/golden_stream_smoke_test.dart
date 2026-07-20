import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mdstream_flutter_example/bootstrap.dart';
import 'package:mdstream_flutter_example/golden_stream_host.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('bundled bootstrap reaches the golden final checkpoint', (
    tester,
  ) async {
    final bootstrap = GoldenStreamBootstrap.bundled();
    await tester.runAsync(() async {
      await bootstrap.initialize(autoPlay: false);
      await bootstrap.replay(mode: GoldenPlaybackMode.immediate);
    });

    await tester.pumpWidget(
      GoldenStreamExample(bootstrap: bootstrap, autoPlay: false),
    );
    await tester.pumpAndSettle();

    expect(bootstrap.runtime?.abiVersion, 1);
    expect(bootstrap.runtime?.bindingSchema, 'mdstream.bindings/0.4');
    expect(bootstrap.phase, GoldenHostPhase.settled);
    expect(bootstrap.finalCheckpoint?.id, 'finalized');
    expect(bootstrap.finalCheckpoint?.lifecycle, 'finalized');
    expect(bootstrap.finalCheckpoint?.rootIds, isNotEmpty);
    expect(bootstrap.controller?.pendingSource.value, isNull);
    expect(bootstrap.controller?.transitions.value.revision, greaterThan(0));
    expect(find.text('Why stable streaming matters'), findsOneWidget);
    expect(find.text('Mermaid source'), findsOneWidget);
    expect(find.byKey(goldenInspectorKey), findsOneWidget);

    await tester.pumpWidget(const SizedBox.shrink());
    expect(bootstrap.runtime?.nativeAllocations.isZero, isTrue);
  });
}
