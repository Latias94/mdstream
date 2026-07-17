import 'dart:convert';
import 'dart:io';

import 'package:mdstream/mdstream.dart';

Directory get repositoryRoot => Directory.current.parent.parent;

Map<String, Object?> loadFixture(String relativePath) {
  final file = File(
    '${repositoryRoot.path}${Platform.pathSeparator}$relativePath',
  );
  return record(jsonDecode(file.readAsStringSync()), relativePath);
}

CanonicalChangeBytes encodeChange(Object? change) =>
    CanonicalChangeBytes(utf8.encode(jsonEncode(change)));

Map<String, Object?> record(Object? value, String field) {
  if (value is! Map) {
    throw FormatException('$field must be an object');
  }
  return Map<String, Object?>.from(value);
}

List<Object?> list(Object? value, String field) {
  if (value is! List<Object?>) {
    throw FormatException('$field must be an array');
  }
  return value;
}
