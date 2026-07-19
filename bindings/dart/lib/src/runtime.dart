import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'engine.dart';
import 'errors.dart';
import 'ffi.dart';
import 'options.dart';
import 'reducer_handle.dart';

/// Point-in-time diagnostic counters for native resources owned by the ABI.
final class MdstreamNativeAllocations {
  /// Creates an immutable native allocation snapshot.
  const MdstreamNativeAllocations({
    required this.engineHandles,
    required this.reducerHandles,
    required this.outputs,
    required this.buffers,
    required this.bufferBytes,
  });

  /// Live engine handles.
  final int engineHandles;

  /// Live reducer handles.
  final int reducerHandles;

  /// Live output handles being drained.
  final int outputs;

  /// Live Rust-owned buffers transferred across FFI.
  final int buffers;

  /// Bytes retained by live Rust-owned buffers.
  final int bufferBytes;

  /// Whether every native resource counter is zero.
  bool get isZero =>
      engineHandles == 0 &&
      reducerHandles == 0 &&
      outputs == 0 &&
      buffers == 0 &&
      bufferBytes == 0;
}

/// Validated entry point for a host-supplied `mdstream-ffi` library.
final class MdstreamRuntime {
  MdstreamRuntime._(this._bindings);

  /// Loads a native library from an explicit host path.
  factory MdstreamRuntime.openPath(String path) {
    if (path.isEmpty) {
      throw ArgumentError.value(path, 'path', 'must not be empty');
    }
    final resolved = File(path).absolute.path;
    final DynamicLibrary library;
    try {
      library = DynamicLibrary.open(resolved);
    } catch (error) {
      throw MdstreamException(
        'failed to open mdstream native library at $resolved',
        detailCode: 'ffi.library_open',
        cause: error,
      );
    }
    try {
      return MdstreamRuntime.fromDynamicLibrary(library);
    } catch (_) {
      library.close();
      rethrow;
    }
  }

  /// Takes process-lifetime ownership of an already loaded library.
  ///
  /// This supports `DynamicLibrary.process()` on Apple plugin builds. The
  /// caller must not close [library] after passing it here because native
  /// handles and finalizer callbacks retain its function pointers.
  factory MdstreamRuntime.fromDynamicLibrary(DynamicLibrary library) =>
      MdstreamRuntime._(NativeBindings.fromDynamicLibrary(library));

  final NativeBindings _bindings;

  /// Stable native ABI version.
  int get abiVersion => _bindings.abiVersion;

  /// Native mdstream package version.
  String get packageVersion => _bindings.packageVersion;

  /// Binding command and view schema.
  String get bindingSchema => _bindings.bindingSchema;

  /// Binding session-options schema.
  String get bindingOptionsSchema => _bindings.optionsSchema;

  /// Transition-facts schema validated before any session is created.
  String get transitionSchema => _bindings.transitionSchema;

  /// Creates an independent canonical reducer session.
  MdstreamReducer createReducer({MdstreamSessionOptions? options}) {
    final bytes = _encodeOptions(options);
    return createNativeReducer(_bindings.createReducer(bytes), bindingSchema);
  }

  /// Creates a streaming engine paired with a private canonical reducer.
  MdstreamEngine createEngine({MdstreamSessionOptions? options}) {
    final bytes = _encodeOptions(options);
    final engine = _bindings.createEngine(bytes);
    MdstreamReducer? reducer;
    try {
      reducer = createNativeReducer(
        _bindings.createReducer(bytes),
        bindingSchema,
      );
      return createNativeEngine(engine, reducer, bindingSchema);
    } catch (_) {
      reducer?.close();
      engine.close();
      rethrow;
    }
  }

  /// Returns process-wide transport allocations for diagnostics and tests.
  MdstreamNativeAllocations get nativeAllocations {
    final metrics = _bindings.allocationMetrics();
    return MdstreamNativeAllocations(
      engineHandles: metrics.engineHandles,
      reducerHandles: metrics.reducerHandles,
      outputs: metrics.outputs,
      buffers: metrics.buffers,
      bufferBytes: metrics.bufferBytes,
    );
  }

  Uint8List _encodeOptions(MdstreamSessionOptions? options) {
    if (options == null) {
      return Uint8List(0);
    }
    return Uint8List.fromList(
      utf8.encode(jsonEncode(options.toJson(bindingOptionsSchema))),
    );
  }
}
