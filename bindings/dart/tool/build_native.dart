import 'dart:convert';
import 'dart:io';

import 'package:mdstream/mdstream.dart';

Future<void> main(List<String> arguments) async {
  final explicitPath = _explicitLibraryPath(arguments);
  final packageRoot = File.fromUri(Platform.script).parent.parent;
  final repositoryRoot = packageRoot.parent.parent;
  final library = explicitPath == null
      ? await _buildLibrary(repositoryRoot)
      : File(explicitPath);

  if (!library.existsSync()) {
    stderr.writeln('mdstream native library does not exist: ${library.path}');
    exitCode = 2;
    return;
  }
  final resolvedLibrary = File(library.resolveSymbolicLinksSync());
  final runtime = MdstreamRuntime.openPath(resolvedLibrary.path);

  final rustc = await Process.run('rustc', const ['+1.85.0', '-vV']);
  if (rustc.exitCode != 0) {
    stderr.write(rustc.stderr);
    exitCode = rustc.exitCode;
    return;
  }
  final targetTriple = const LineSplitter()
      .convert(rustc.stdout as String)
      .where((line) => line.startsWith('host: '))
      .map((line) => line.substring('host: '.length))
      .single;

  final configuration = <String, Object>{
    'schema': 'mdstream.dart-native-library/1',
    'library': resolvedLibrary.path,
    'source': explicitPath == null ? 'cargo' : 'host-supplied',
    'profile': explicitPath == null ? 'debug' : 'unknown',
    'target': targetTriple,
    'abi_version': runtime.abiVersion,
    'package_version': runtime.packageVersion,
    'binding_schema': runtime.bindingSchema,
    'binding_options_schema': runtime.bindingOptionsSchema,
  };
  final outputDirectory = Directory(
    '${packageRoot.path}${Platform.pathSeparator}.dart_tool'
    '${Platform.pathSeparator}mdstream',
  )..createSync(recursive: true);
  final output = File(
    '${outputDirectory.path}${Platform.pathSeparator}native-library.json',
  );
  final temporary = File('${output.path}.tmp');
  temporary.writeAsStringSync('${jsonEncode(configuration)}\n', flush: true);
  temporary.renameSync(output.path);
  stdout.writeln(resolvedLibrary.path);
}

String? _explicitLibraryPath(List<String> arguments) {
  String? fromArguments;
  for (var index = 0; index < arguments.length; index += 1) {
    final argument = arguments[index];
    if (argument == '--library') {
      if (index + 1 >= arguments.length) {
        throw const FormatException('--library requires a path');
      }
      fromArguments = arguments[index + 1];
      index += 1;
    } else if (argument.startsWith('--library=')) {
      fromArguments = argument.substring('--library='.length);
    } else {
      throw FormatException('unknown argument: $argument');
    }
  }
  final environment =
      Platform.environment['MDSTREAM_NATIVE_LIBRARY'] ??
      Platform.environment['MDSTREAM_FFI_LIBRARY'];
  final selected = fromArguments ?? environment;
  if (selected == null) {
    return null;
  }
  if (selected.isEmpty) {
    throw const FormatException('native library path must not be empty');
  }
  return File(selected).absolute.path;
}

Future<File> _buildLibrary(Directory repositoryRoot) async {
  final process = await Process.start('cargo', [
    '+1.85.0',
    'build',
    '--locked',
    '--manifest-path',
    '${repositoryRoot.path}${Platform.pathSeparator}Cargo.toml',
    '-p',
    'mdstream-ffi',
    '--message-format=json-render-diagnostics',
  ], workingDirectory: repositoryRoot.path);

  String? artifact;
  final stdoutDone = process.stdout
      .transform(utf8.decoder)
      .transform(const LineSplitter())
      .forEach((line) {
        final Object? decoded;
        try {
          decoded = jsonDecode(line);
        } on FormatException {
          stderr.writeln(line);
          return;
        }
        if (decoded is! Map<String, Object?> ||
            decoded['reason'] != 'compiler-artifact') {
          return;
        }
        final target = decoded['target'];
        final filenames = decoded['filenames'];
        if (target is! Map<String, Object?> ||
            target['name'] != 'mdstream_ffi' ||
            target['kind'] is! List ||
            !(target['kind'] as List<Object?>).contains('cdylib') ||
            filenames is! List) {
          return;
        }
        for (final filename in filenames.whereType<String>()) {
          if (_isHostDynamicLibrary(filename)) {
            artifact = filename;
          }
        }
      });
  final stderrDone = process.stderr
      .transform(utf8.decoder)
      .forEach(stderr.write);
  await Future.wait<void>([stdoutDone, stderrDone]);
  final status = await process.exitCode;
  if (status != 0) {
    throw ProcessException('cargo', const [], 'native build failed', status);
  }
  final path = artifact;
  if (path == null) {
    throw StateError('cargo did not emit the mdstream-ffi cdylib artifact');
  }
  return File(path);
}

bool _isHostDynamicLibrary(String path) {
  if (Platform.isMacOS || Platform.isIOS) {
    return path.endsWith('.dylib');
  }
  if (Platform.isWindows) {
    return path.endsWith('.dll');
  }
  return path.endsWith('.so');
}
