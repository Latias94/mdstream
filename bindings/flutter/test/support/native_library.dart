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
    '${Directory.current.parent.path}${Platform.pathSeparator}dart'
    '${Platform.pathSeparator}.dart_tool${Platform.pathSeparator}mdstream'
    '${Platform.pathSeparator}native-library.json',
  );
  if (!configuration.existsSync()) {
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
