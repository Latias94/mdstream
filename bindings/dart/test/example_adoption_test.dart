import 'dart:convert';
import 'dart:io';

import 'package:mdstream/mdstream.dart';
import 'package:test/test.dart';

import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test('example requires an explicit or environment native library', () async {
    final result = await _runExample(
      const <String>[],
      environment: _environmentWithoutNativeLibrary(),
    );

    expect(result.exitCode, 64);
    expect(result.stdout, isEmpty);
    expect(result.stderr, contains('No native library was provided.'));
    expect(result.stderr, contains('Usage:'));
    expect(result.stderr, isNot(contains('.dart_tool')));
  });

  test('example help does not load a native library', () async {
    final result = await _runExample(const <String>[
      '--help',
    ], environment: _environmentWithoutNativeLibrary());

    expect(result.exitCode, 0);
    expect(result.stdout, contains('Usage:'));
    expect(result.stdout, contains('MDSTREAM_NATIVE_LIBRARY'));
    expect(result.stderr, isEmpty);
  });

  test(
    'explicit and environment library inputs replay the same golden stream',
    () async {
      final explicit = await _runExample(<String>[
        '--library',
        libraryPath!,
        '--assert',
      ]);
      final environment = await _runExample(
        const <String>['--assert'],
        environment: <String, String>{
          ..._environmentWithoutNativeLibrary(),
          'MDSTREAM_NATIVE_LIBRARY': libraryPath,
        },
      );

      expect(explicit.exitCode, 0, reason: explicit.stderr as String);
      expect(environment.exitCode, 0, reason: environment.stderr as String);
      expect(explicit.stderr, isEmpty);
      expect(environment.stderr, isEmpty);
      expect(environment.stdout, explicit.stdout);

      final output = explicit.stdout;
      expect(output, contains('scenario=golden-ai-stream'));
      expect(output, contains('checkpoint=rust-fence-pending'));
      expect(output, contains('pending_source=rust-fence-pending'));
      expect(output, contains('transition=0 scope=continuous'));
      expect(output, contains('stable_node_ids='));
      expect(output, contains('final_lifecycle=finalized'));
      expect(output, contains('native_allocations=zero'));
      expect(output, contains('assertions=passed'));
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );

  test(
    'assertion drift exits nonzero after releasing native allocations',
    () async {
      final temporary = await Directory.systemTemp.createTemp(
        'mdstream-dart-example-',
      );
      try {
        final fixture =
            jsonDecode(
                  File(
                    'example/fixtures/golden_ai_stream.json',
                  ).readAsStringSync(),
                )
                as Map<String, Object?>;
        final expected = fixture['expected']! as Map<String, Object?>;
        expected['final_source'] = '${expected['final_source']}drift';
        final scenario = File('${temporary.path}/drift.json')
          ..writeAsStringSync(jsonEncode(fixture));

        final result = await _runExample(<String>[
          '--library',
          libraryPath!,
          '--scenario',
          scenario.path,
          '--assert',
        ]);

        expect(result.exitCode, 1);
        expect(
          result.stderr,
          contains('assertion failed: final canonical source drifted'),
        );
        expect(result.stderr, isNot(contains('native allocations leaked')));
      } finally {
        await temporary.delete(recursive: true);
      }

      final runtime = MdstreamRuntime.openPath(libraryPath);
      final engine = runtime.createEngine();
      engine.close();
      engine.close();
      expect(runtime.nativeAllocations.isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}

Future<ProcessResult> _runExample(
  List<String> arguments, {
  Map<String, String>? environment,
}) => Process.run(
  Platform.resolvedExecutable,
  <String>['run', 'example/golden_stream.dart', ...arguments],
  workingDirectory: Directory.current.path,
  environment: environment,
  includeParentEnvironment: environment == null,
);

Map<String, String> _environmentWithoutNativeLibrary() =>
    Map<String, String>.from(Platform.environment)
      ..remove('MDSTREAM_NATIVE_LIBRARY')
      ..remove('MDSTREAM_FFI_LIBRARY');
