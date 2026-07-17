import 'dart:convert';
import 'dart:ffi';
import 'dart:typed_data';

import 'package:mdstream/src/errors.dart';
import 'package:mdstream/src/ffi.dart';
import 'package:mdstream/src/protocol.dart';
import 'package:test/test.dart';

import 'support/native_library.dart';

void main() {
  final libraryPath = nativeLibraryPath();

  test(
    'checked ABI streams, reduces, maps errors, and releases exact-once',
    () {
      final bindings = NativeBindings.fromDynamicLibrary(
        DynamicLibrary.open(libraryPath!),
      );
      expect(bindings.abiVersion, mdstreamAbiVersion);
      expect(bindings.bindingSchema, bindingSchema);
      expect(bindings.optionsSchema, bindingOptionsSchema);
      expect(bindings.packageVersion, '0.4.0');
      expect(bindings.allocationMetrics().isZero, isTrue);

      final engine = bindings.createEngine(Uint8List(0));
      final reducer = bindings.createReducer(Uint8List(0));
      try {
        final produced = engine.append(_bytes('# Hello\n'));
        expect(produced, isNotEmpty);
        for (final payload in produced) {
          expect(payload.kind, BindingPayloadKind.change.value);
          final reduced = reducer.apply(payload.bytes);
          expect(
            reduced.map((entry) => entry.kind),
            contains(BindingPayloadKind.reducerUpdate.value),
          );
        }

        final finished = engine.execute(_command('finish'));
        for (final payload in finished) {
          expect(payload.kind, BindingPayloadKind.change.value);
          reducer.apply(payload.bytes);
        }
        expect(
          () => engine.append(_bytes('after finish')),
          throwsA(
            isA<MdstreamException>()
                .having((error) => error.status, 'status', 6)
                .having(
                  (error) => error.statusName,
                  'statusName',
                  'MDSTREAM_TERMINAL',
                ),
          ),
        );

        final snapshots = reducer.execute(_command('snapshot'));
        expect(snapshots, hasLength(1));
        expect(snapshots.single.kind, BindingPayloadKind.snapshot.value);
        final snapshot = jsonDecode(utf8.decode(snapshots.single.bytes));
        expect(snapshot, isA<Map<String, Object?>>());
        expect(
          (snapshot as Map<String, Object?>)['schema'],
          'mdstream.content/0.4',
        );
      } finally {
        reducer.close();
        reducer.close();
        engine.close();
        engine.close();
      }
      expect(bindings.allocationMetrics().isZero, isTrue);

      expect(
        () => bindings.createEngine(_bytes('{"schema":"wrong","protocol":{}}')),
        throwsA(
          isA<MdstreamException>().having((error) => error.status, 'status', 5),
        ),
      );
      expect(bindings.allocationMetrics().isZero, isTrue);
    },
    skip: libraryPath == null
        ? 'run dart run tool/build_native.dart before native tests'
        : false,
  );
}

Uint8List _bytes(String value) => Uint8List.fromList(utf8.encode(value));

Uint8List _command(String kind) =>
    _bytes(jsonEncode(<String, String>{'schema': bindingSchema, 'kind': kind}));
