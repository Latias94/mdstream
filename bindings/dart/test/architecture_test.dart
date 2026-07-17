import 'dart:io';

import 'package:test/test.dart';
import 'package:yaml/yaml.dart';

void main() {
  test('standalone package stays framework and parser neutral', () {
    final root = Directory.current;
    final sources = Directory('${root.path}${Platform.pathSeparator}lib')
        .listSync(recursive: true)
        .whereType<File>()
        .where((file) => file.path.endsWith('.dart'))
        .map((file) => file.readAsStringSync())
        .join('\n');

    for (final forbidden in <String>[
      'package:flutter',
      'package:react',
      'merman',
      'pulldown',
      'streamdown',
      'incremark',
      'insert_node',
      'replace_node',
      'remove_node',
      'finish_document',
      'class ChangeSet',
      'class ProjectionOp',
    ]) {
      expect(sources.toLowerCase(), isNot(contains(forbidden.toLowerCase())));
    }

    final entrypoint = File(
      '${root.path}${Platform.pathSeparator}lib'
      '${Platform.pathSeparator}mdstream.dart',
    ).readAsStringSync();
    expect(entrypoint, isNot(contains("export 'src/ffi.dart'")));

    final pubspec =
        loadYaml(
              File(
                '${root.path}${Platform.pathSeparator}pubspec.yaml',
              ).readAsStringSync(),
            )
            as YamlMap;
    expect((pubspec['dependencies'] as YamlMap).keys, <Object?>['ffi']);
    expect(pubspec.containsKey('flutter'), isFalse);
  });
}
