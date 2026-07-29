import 'dart:io';

import 'package:mdstream_flutter/mdstream_flutter.dart';

const runtimeSmokeResultName = 'mdstream-flutter-runtime-smoke.json';
const runtimeSmokeSchema = 'mdstream.flutter-runtime-smoke/1';
const runtimeSmokeAbiVersion = 1;
const runtimeSmokePackageVersion = '0.4.0';
const runtimeSmokeBindingSchema = 'mdstream.bindings/0.4';

final class RuntimeSmokeReport {
  const RuntimeSmokeReport({
    required this.abiVersion,
    required this.packageVersion,
    required this.bindingSchema,
    required this.isFinalized,
    required this.hasRootNode,
    required this.nativeAllocationsZero,
  });

  final int abiVersion;
  final String packageVersion;
  final String bindingSchema;
  final bool isFinalized;
  final bool hasRootNode;
  final bool nativeAllocationsZero;

  Map<String, Object?> toJson() => <String, Object?>{
    'abi_version': abiVersion,
    'package_version': packageVersion,
    'binding_schema': bindingSchema,
    'is_finalized': isFinalized,
    'has_root_node': hasRootNode,
    'native_allocations_zero': nativeAllocationsZero,
  };
}

RuntimeSmokeReport runBundledRuntimeSmoke() {
  if (Platform.environment['MDSTREAM_NATIVE_LIBRARY'] != null ||
      Platform.environment['MDSTREAM_FFI_LIBRARY'] != null) {
    throw StateError('native library override leaked into the smoke app');
  }

  final runtime = MdstreamFlutterRuntime.open();
  if (runtime.abiVersion != runtimeSmokeAbiVersion ||
      runtime.packageVersion != runtimeSmokePackageVersion ||
      runtime.bindingSchema != runtimeSmokeBindingSchema) {
    throw StateError('bundled runtime metadata does not match the package');
  }

  final controller = MdstreamController.fromRuntime(runtime);
  late final bool isFinalized;
  late final bool hasRootNode;
  try {
    controller.append('# Bundled runtime\n\nstreamed content');
    controller.finish();
    final roots = controller.value.document?.roots?.children;
    isFinalized = controller.value.isFinalized;
    hasRootNode =
        roots != null &&
        roots.isNotEmpty &&
        controller.node(roots.first).value != null;
  } finally {
    controller.dispose();
  }

  final nativeAllocationsZero = runtime.nativeAllocations.isZero;
  if (!isFinalized || !hasRootNode || !nativeAllocationsZero) {
    throw StateError('bundled runtime did not complete the smoke trace');
  }

  return RuntimeSmokeReport(
    abiVersion: runtime.abiVersion,
    packageVersion: runtime.packageVersion,
    bindingSchema: runtime.bindingSchema,
    isFinalized: isFinalized,
    hasRootNode: hasRootNode,
    nativeAllocationsZero: nativeAllocationsZero,
  );
}
