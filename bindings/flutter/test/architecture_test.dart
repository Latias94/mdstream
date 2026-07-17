import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'public package remains a framework state adapter without render policy',
    () {
      final manifest = File('pubspec.yaml').readAsStringSync();
      expect(manifest, contains('flutter:'));
      expect(manifest, contains('mdstream: ^0.4.0'));
      for (final forbidden in <String>[
        'merman:',
        'react:',
        'streamdown:',
        'incremark:',
      ]) {
        expect(manifest.toLowerCase(), isNot(contains(forbidden)));
      }

      final source = Directory('lib')
          .listSync(recursive: true)
          .whereType<File>()
          .where((file) => file.path.endsWith('.dart'))
          .map((file) => file.readAsStringSync())
          .join('\n');
      for (final forbidden in <String>[
        'extends Widget',
        'extends StatelessWidget',
        'extends StatefulWidget',
        'insert_node',
        'replace_node',
        'splice_children',
        'finish_document',
        'package:merman',
        'package:react',
        'package:streamdown',
        'package:incremark',
      ]) {
        expect(source, isNot(contains(forbidden)));
      }

      expect(source, contains("DynamicLibrary.open('libmdstream_ffi.so')"));
      expect(source, contains("DynamicLibrary.open('mdstream_ffi.dll')"));
      expect(source, contains('DynamicLibrary.process()'));
    },
  );
}
