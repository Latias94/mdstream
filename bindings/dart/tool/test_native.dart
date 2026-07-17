import 'dart:io';

Future<void> main() async {
  final packageRoot = File.fromUri(Platform.script).parent.parent;
  final build = await Process.start(
    Platform.resolvedExecutable,
    const ['run', 'tool/build_native.dart'],
    workingDirectory: packageRoot.path,
    mode: ProcessStartMode.inheritStdio,
  );
  final buildStatus = await build.exitCode;
  if (buildStatus != 0) {
    exitCode = buildStatus;
    return;
  }

  final tests = await Process.start(
    Platform.resolvedExecutable,
    const ['test'],
    workingDirectory: packageRoot.path,
    environment: {...Platform.environment, 'MDSTREAM_REQUIRE_NATIVE': '1'},
    mode: ProcessStartMode.inheritStdio,
  );
  exitCode = await tests.exitCode;
}
