import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';

import 'runtime_smoke_probe.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const SizedBox.shrink());

  final result = await _runSmoke();
  final destination = File(
    '${Directory.systemTemp.path}/$runtimeSmokeResultName',
  );
  final pending = File('${destination.path}.pending');
  await pending.writeAsString(jsonEncode(result), flush: true);
  await pending.rename(destination.path);
}

Future<Map<String, Object?>> _runSmoke() async {
  try {
    final report = runBundledRuntimeSmoke();
    return <String, Object?>{
      'schema': runtimeSmokeSchema,
      'ok': true,
      ...report.toJson(),
    };
  } catch (error, stackTrace) {
    return <String, Object?>{
      'schema': runtimeSmokeSchema,
      'ok': false,
      'error': error.toString(),
      'stack_trace': stackTrace.toString(),
    };
  }
}
