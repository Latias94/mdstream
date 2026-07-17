part of '../mdstream_flutter.dart';

/// Opens the native library bundled by the Flutter plugin.
final class MdstreamFlutterRuntime {
  MdstreamFlutterRuntime._();

  static MdstreamRuntime? _runtime;

  /// Returns the process-wide runtime backed by the bundled native library.
  static MdstreamRuntime open() =>
      _runtime ??= MdstreamRuntime.fromDynamicLibrary(_openBundledLibrary());

  static DynamicLibrary _openBundledLibrary() {
    if (Platform.isAndroid || Platform.isLinux) {
      return DynamicLibrary.open('libmdstream_ffi.so');
    }
    if (Platform.isWindows) {
      return DynamicLibrary.open('mdstream_ffi.dll');
    }
    if (Platform.isIOS || Platform.isMacOS) {
      return DynamicLibrary.process();
    }
    throw UnsupportedError(
      'mdstream_flutter does not support ${Platform.operatingSystem}',
    );
  }
}
