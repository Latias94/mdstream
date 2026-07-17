import 'dart:convert';
import 'dart:io';

import 'package:mdstream/mdstream.dart';

Directory get repositoryRoot => Directory.current.parent.parent;

Map<String, Object?> loadFixture(String relativePath) {
  final file = File(
    '${repositoryRoot.path}${Platform.pathSeparator}$relativePath',
  );
  return _record(jsonDecode(file.readAsStringSync()), relativePath);
}

CanonicalChangeBytes encodeChange(Object? change) =>
    CanonicalChangeBytes(utf8.encode(jsonEncode(change)));

Map<String, Object?> decodeSnapshot(CanonicalSnapshotBytes snapshot) =>
    _record(jsonDecode(utf8.decode(snapshot.bytes)), 'snapshot');

Map<String, Object?> normalizeSnapshot(Map<String, Object?> snapshot) {
  final coordinate = _record(snapshot['coordinate'], 'coordinate');
  return {
    'schema': snapshot['schema'],
    'maturity': snapshot['maturity'],
    'epoch': coordinate['epoch'],
    'lifecycle': snapshot['lifecycle'],
    'source': snapshot['source'],
    'projection_cursor': snapshot['projection_cursor'],
    'roots': snapshot['roots'],
    'nodes': snapshot['nodes'],
    'resources': snapshot['resources'],
  };
}

Map<String, Object?> record(Object? value, String field) =>
    _record(value, field);

List<Object?> list(Object? value, String field) {
  if (value is! List<Object?>) {
    throw FormatException('$field must be an array');
  }
  return value;
}

Map<String, Object?> _record(Object? value, String field) {
  if (value is! Map) {
    throw FormatException('$field must be an object');
  }
  return Map<String, Object?>.from(value);
}
