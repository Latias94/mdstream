import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:yaml/yaml.dart';

Future<void> main() async {
  final packageRoot = File.fromUri(Platform.script).parent.parent;
  final repositoryRoot = packageRoot.parent.parent;
  final archiveDirectory = Directory(
    '${repositoryRoot.path}${Platform.pathSeparator}target'
    '${Platform.pathSeparator}dart-package',
  )..createSync(recursive: true);
  final archive = File(
    '${archiveDirectory.path}${Platform.pathSeparator}mdstream-0.4.0.tar.gz',
  );

  final publish = await Process.run(Platform.resolvedExecutable, [
    'pub',
    'publish',
    '--to-archive=${archive.path}',
  ],
    workingDirectory: packageRoot.path,
    environment: {
      ...Platform.environment,
      'PUB_HOSTED_URL': 'https://pub.dev',
    },
  );
  if (publish.exitCode != 0) {
    stderr.write(publish.stdout);
    stderr.write(publish.stderr);
    exitCode = publish.exitCode;
    return;
  }

  final budgetDocument =
      jsonDecode(
            File(
              '${repositoryRoot.path}${Platform.pathSeparator}bindings'
              '${Platform.pathSeparator}budgets.json',
            ).readAsStringSync(),
          )
          as Map<String, Object?>;
  final artifacts = budgetDocument['artifacts']! as List<Object?>;
  final dartBudget = artifacts.cast<Map<String, Object?>>().singleWhere(
    (artifact) => artifact['artifact'] == 'dart_packed',
  );
  final ceiling = dartBudget['ceiling_bytes']! as int;
  final measurement = dartBudget['measurement']! as Map<String, Object?>;
  final baselineBytes = measurement['measured_bytes']! as int;
  final regressionPercent = dartBudget['regression_percent']! as int;
  final bytes = archive.readAsBytesSync();
  final digest = sha256.convert(bytes).toString();
  if (bytes.length > ceiling) {
    throw StateError(
      'Dart package is ${bytes.length} bytes; ceiling is $ceiling bytes',
    );
  }
  final advisoryBytes = baselineBytes * (100 + regressionPercent) ~/ 100;
  if (bytes.length > advisoryBytes) {
    stderr.writeln(
      'warning: Dart package ${bytes.length} bytes exceeds advisory baseline '
      '$advisoryBytes bytes',
    );
  }

  final pubspec =
      loadYaml(
            File(
              '${packageRoot.path}${Platform.pathSeparator}pubspec.yaml',
            ).readAsStringSync(),
          )
          as YamlMap;
  final dependencies = (pubspec['dependencies'] as YamlMap).keys
      .cast<String>()
      .toSet();
  if (dependencies.length != 1 || !dependencies.contains('ffi')) {
    throw StateError('standalone Dart production dependency must be only ffi');
  }
  final policy = budgetDocument['policy']! as Map<String, Object?>;
  final forbidden = (policy['forbidden_default_dependencies']! as List<Object?>)
      .cast<String>();
  final presentForbidden = dependencies.intersection(forbidden.toSet());
  if (presentForbidden.isNotEmpty) {
    throw StateError('forbidden Dart dependencies: $presentForbidden');
  }

  for (final entity in packageRoot.listSync(recursive: true)) {
    if (entity is! File ||
        entity.path.contains(
          '${Platform.pathSeparator}.dart_tool${Platform.pathSeparator}',
        )) {
      continue;
    }
    final extension = entity.path.toLowerCase();
    if (_nativeExtensions.any(extension.endsWith)) {
      throw StateError(
        'native binary must not ship in Dart package: ${entity.path}',
      );
    }
    final header = entity.openSync()..setPositionSync(0);
    try {
      final prefix = header.readSync(8);
      if (_hasNativeMagic(prefix)) {
        throw StateError(
          'native binary magic found in Dart package: ${entity.path}',
        );
      }
    } finally {
      header.closeSync();
    }
  }

  stdout.writeln(
    'dart package verified: ${bytes.length}/$ceiling bytes, sha256=$digest',
  );
}

const _nativeExtensions = <String>[
  '.a',
  '.dylib',
  '.dll',
  '.framework',
  '.lib',
  '.so',
];

bool _hasNativeMagic(List<int> bytes) {
  if (bytes.length >= 4) {
    final prefix = bytes.take(4).toList(growable: false);
    if (_fourByteMagics.any((magic) => _equals(prefix, magic))) {
      return true;
    }
  }
  return bytes.length >= 2 && bytes[0] == 0x4d && bytes[1] == 0x5a ||
      bytes.length >= 8 &&
          utf8.decode(bytes, allowMalformed: true) == '!<arch>\n';
}

const _fourByteMagics = <List<int>>[
  [0x7f, 0x45, 0x4c, 0x46],
  [0xfe, 0xed, 0xfa, 0xce],
  [0xfe, 0xed, 0xfa, 0xcf],
  [0xce, 0xfa, 0xed, 0xfe],
  [0xcf, 0xfa, 0xed, 0xfe],
  [0xca, 0xfe, 0xba, 0xbe],
  [0xbe, 0xba, 0xfe, 0xca],
];

bool _equals(List<int> left, List<int> right) {
  for (var index = 0; index < left.length; index += 1) {
    if (left[index] != right[index]) {
      return false;
    }
  }
  return true;
}
