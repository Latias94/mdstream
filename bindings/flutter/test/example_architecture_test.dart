import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'example owns presentation while package lib stays renderer-neutral',
    () {
      final exampleFiles = <String>[
        'example/lib/main.dart',
        'example/lib/bootstrap.dart',
        'example/lib/golden_stream_host.dart',
        'example/lib/content_ir_view.dart',
        'example/integration_test/golden_stream_smoke_test.dart',
        'example/assets/golden_ai_stream.json',
      ];
      for (final path in exampleFiles) {
        expect(File(path).existsSync(), isTrue, reason: '$path must ship');
      }

      final packageSource = Directory('lib')
          .listSync(recursive: true)
          .whereType<File>()
          .where((file) => file.path.endsWith('.dart'))
          .map((file) => file.readAsStringSync())
          .join('\n');
      for (final forbidden in <String>[
        'StatelessWidget',
        'StatefulWidget',
        'GoldenStream',
        'Mermaid source',
        'package:markdown',
        'package:merman',
      ]) {
        expect(packageSource, isNot(contains(forbidden)));
      }

      final exampleSource = exampleFiles
          .where((path) => path.endsWith('.dart'))
          .map((path) => File(path).readAsStringSync())
          .join('\n');
      expect(exampleSource, contains('MdstreamController'));
      expect(exampleSource, contains('controller.node('));
      expect(exampleSource, contains('controller.nodeKey'));
      expect(exampleSource, contains('controller.pendingSource'));
      expect(exampleSource, contains('controller.transitions'));
      expect(exampleSource, isNot(contains('SvgPicture')));
      expect(exampleSource, isNot(contains('MarkdownBody')));
      expect(exampleSource, isNot(contains('package:markdown')));
      expect(exampleSource, isNot(contains('package:merman')));

      final mainSource = File('example/lib/main.dart').readAsStringSync();
      final integrationSource = File(
        'example/integration_test/golden_stream_smoke_test.dart',
      ).readAsStringSync();
      expect(mainSource, contains('GoldenStreamBootstrap.bundled()'));
      expect(integrationSource, contains('GoldenStreamBootstrap.bundled()'));
    },
  );

  test(
    'repository-only dependency override cannot enter the package archive',
    () {
      final manifest = File('example/pubspec.yaml').readAsStringSync();
      expect(manifest, contains('assets/golden_ai_stream.json'));
      expect(manifest, isNot(contains('dependency_overrides:')));
      expect(manifest, isNot(contains('markdown:')));
      expect(manifest, isNot(contains('merman:')));

      final overrides = File(
        'example/pubspec_overrides.yaml',
      ).readAsStringSync();
      expect(overrides, contains('path: ../../dart'));

      final publishIgnore = File('.pubignore').readAsStringSync();
      expect(publishIgnore, isNot(contains('\nexample/\n')));
      expect(publishIgnore, contains('example/pubspec_overrides.yaml'));
      expect(publishIgnore, contains('example/pubspec.lock'));
      expect(publishIgnore, contains('example/.flutter-plugins-dependencies'));
    },
  );
}
