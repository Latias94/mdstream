import 'dart:io';

import 'package:mdstream_flutter_example/configure_host.dart';

void main(List<String> arguments) {
  if (arguments.length != 1) {
    stderr.writeln('usage: dart run configure_host.dart <ios|macos>');
    exitCode = 2;
    return;
  }

  try {
    configureHost(projectRoot: Directory.current, platform: arguments.single);
  } on ConfigureHostException catch (error) {
    stderr.writeln(error.message);
    exitCode = 2;
    return;
  }

  stdout.writeln('configured ${arguments.single} deployment target');
}
