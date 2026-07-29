import 'dart:convert';
import 'dart:io';

String? nativeLibraryPath() {
  final environment =
      Platform.environment['MDSTREAM_NATIVE_LIBRARY'] ??
      Platform.environment['MDSTREAM_FFI_LIBRARY'];
  if (environment != null && environment.isNotEmpty) {
    return File(environment).absolute.path;
  }
  final configuration = File(
    '${Directory.current.path}${Platform.pathSeparator}.dart_tool'
    '${Platform.pathSeparator}mdstream${Platform.pathSeparator}'
    'native-library.json',
  );
  if (!configuration.existsSync()) {
    if (Platform.environment['MDSTREAM_REQUIRE_NATIVE'] == '1') {
      throw StateError(
        'required mdstream native library metadata is missing; '
        'run dart run tool/build_native.dart first',
      );
    }
    return null;
  }
  final decoded = jsonDecode(configuration.readAsStringSync());
  if (decoded is! Map<String, Object?> ||
      decoded['schema'] != 'mdstream.dart-native-library/1' ||
      decoded['library'] is! String) {
    throw const FormatException('invalid mdstream native library metadata');
  }
  return decoded['library']! as String;
}
