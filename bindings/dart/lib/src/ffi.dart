import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'errors.dart';
import 'options.dart';
import 'protocol.dart' as protocol;

const int _expectedAbiVersion = 1;
const String _expectedBindingSchema = 'mdstream.bindings/0.4';
const String _expectedOptionsSchema = 'mdstream.bindings-options/0.4';

final class _NativeMdstreamEngine extends ffi.Opaque {}

final class _NativeMdstreamReducer extends ffi.Opaque {}

final class _NativeMdstreamOutput extends ffi.Opaque {}

final class _MdstreamBuffer extends ffi.Struct {
  external ffi.Pointer<ffi.Uint8> data;

  @ffi.Size()
  external int len;
}

final class _MdstreamAllocationMetrics extends ffi.Struct {
  @ffi.Uint64()
  external int engineHandles;

  @ffi.Uint64()
  external int reducerHandles;

  @ffi.Uint64()
  external int outputs;

  @ffi.Uint64()
  external int buffers;

  @ffi.Uint64()
  external int bufferBytes;
}

final class _MdstreamProcessorSchedulerLimits extends ffi.Struct {
  @ffi.Size()
  external int maxInFlightJobs;

  @ffi.Size()
  external int maxQueuedCandidates;
}

final class _MdstreamCallResult extends ffi.Struct {
  @ffi.Int32()
  external int status;

  external ffi.Pointer<_NativeMdstreamOutput> output;

  external _MdstreamBuffer error;
}

final class _MdstreamEngineResult extends ffi.Struct {
  @ffi.Int32()
  external int status;

  external ffi.Pointer<_NativeMdstreamEngine> engine;

  external _MdstreamBuffer error;
}

final class _MdstreamReducerResult extends ffi.Struct {
  @ffi.Int32()
  external int status;

  external ffi.Pointer<_NativeMdstreamReducer> reducer;

  external _MdstreamBuffer error;
}

final class _MdstreamPayloadResult extends ffi.Struct {
  @ffi.Int32()
  external int status;

  @ffi.Uint32()
  external int kind;

  external _MdstreamBuffer data;
}

typedef _AbiVersionNative = ffi.Uint32 Function();
typedef _AbiVersionDart = int Function();
typedef _StaticStringNative = ffi.Pointer<ffi.Char> Function();
typedef _StaticStringDart = ffi.Pointer<ffi.Char> Function();
typedef _StructSizeNative = ffi.Size Function();
typedef _StructSizeDart = int Function();
typedef _AllocationMetricsNative = _MdstreamAllocationMetrics Function();
typedef _AllocationMetricsDart = _MdstreamAllocationMetrics Function();
typedef _ReducerProcessorSchedulerLimitsNative =
    _MdstreamProcessorSchedulerLimits Function(
      ffi.Pointer<_NativeMdstreamReducer>,
    );
typedef _ReducerProcessorSchedulerLimitsDart =
    _MdstreamProcessorSchedulerLimits Function(
      ffi.Pointer<_NativeMdstreamReducer>,
    );
typedef _EngineNewNative =
    _MdstreamEngineResult Function(ffi.Pointer<ffi.Uint8>, ffi.Size);
typedef _EngineNewDart =
    _MdstreamEngineResult Function(ffi.Pointer<ffi.Uint8>, int);
typedef _ReducerNewNative =
    _MdstreamReducerResult Function(ffi.Pointer<ffi.Uint8>, ffi.Size);
typedef _ReducerNewDart =
    _MdstreamReducerResult Function(ffi.Pointer<ffi.Uint8>, int);
typedef _EngineFreeNative =
    ffi.Void Function(ffi.Pointer<_NativeMdstreamEngine>);
typedef _EngineFreeDart = void Function(ffi.Pointer<_NativeMdstreamEngine>);
typedef _EngineRawAppendByteCeilingNative =
    ffi.Size Function(ffi.Pointer<_NativeMdstreamEngine>);
typedef _EngineRawAppendByteCeilingDart =
    int Function(ffi.Pointer<_NativeMdstreamEngine>);
typedef _ReducerFreeNative =
    ffi.Void Function(ffi.Pointer<_NativeMdstreamReducer>);
typedef _ReducerFreeDart = void Function(ffi.Pointer<_NativeMdstreamReducer>);
typedef _EngineCallNative =
    _MdstreamCallResult Function(
      ffi.Pointer<_NativeMdstreamEngine>,
      ffi.Pointer<ffi.Uint8>,
      ffi.Size,
    );
typedef _EngineCallDart =
    _MdstreamCallResult Function(
      ffi.Pointer<_NativeMdstreamEngine>,
      ffi.Pointer<ffi.Uint8>,
      int,
    );
typedef _ReducerCallNative =
    _MdstreamCallResult Function(
      ffi.Pointer<_NativeMdstreamReducer>,
      ffi.Pointer<ffi.Uint8>,
      ffi.Size,
    );
typedef _ReducerCallDart =
    _MdstreamCallResult Function(
      ffi.Pointer<_NativeMdstreamReducer>,
      ffi.Pointer<ffi.Uint8>,
      int,
    );
typedef _OutputLenNative =
    ffi.Size Function(ffi.Pointer<_NativeMdstreamOutput>);
typedef _OutputLenDart = int Function(ffi.Pointer<_NativeMdstreamOutput>);
typedef _OutputTakeNative =
    _MdstreamPayloadResult Function(
      ffi.Pointer<_NativeMdstreamOutput>,
      ffi.Size,
    );
typedef _OutputTakeDart =
    _MdstreamPayloadResult Function(ffi.Pointer<_NativeMdstreamOutput>, int);
typedef _OutputFreeNative =
    ffi.Void Function(ffi.Pointer<_NativeMdstreamOutput>);
typedef _OutputFreeDart = void Function(ffi.Pointer<_NativeMdstreamOutput>);
typedef _BufferFreeNative = ffi.Void Function(_MdstreamBuffer);
typedef _BufferFreeDart = void Function(_MdstreamBuffer);

/// Package-internal fully copied payload returned by the native transport.
final class NativePayload {
  const NativePayload._(this.kind, this.bytes);

  /// Numeric payload discriminator defined by the binding schema.
  final int kind;

  /// Owned payload bytes copied out of native memory.
  final Uint8List bytes;
}

/// Package-internal process-wide allocations owned by `mdstream-ffi`.
final class NativeAllocationMetrics {
  const NativeAllocationMetrics._({
    required this.engineHandles,
    required this.reducerHandles,
    required this.outputs,
    required this.buffers,
    required this.bufferBytes,
  });

  /// Number of live native engine handles.
  final int engineHandles;

  /// Number of live native reducer handles.
  final int reducerHandles;

  /// Number of live native output containers.
  final int outputs;

  /// Number of live native byte buffers.
  final int buffers;

  /// Total bytes retained by live native byte buffers.
  final int bufferBytes;

  /// Whether every tracked native allocation count is zero.
  bool get isZero =>
      engineHandles == 0 &&
      reducerHandles == 0 &&
      outputs == 0 &&
      buffers == 0 &&
      bufferBytes == 0;
}

/// Package-internal checked symbol table for one `mdstream-ffi` library.
final class NativeBindings {
  NativeBindings._(this.library)
    : _abiVersion = library.lookupFunction<_AbiVersionNative, _AbiVersionDart>(
        'mdstream_abi_version',
      ),
      _packageVersion = library
          .lookupFunction<_StaticStringNative, _StaticStringDart>(
            'mdstream_package_version',
          ),
      _bindingSchema = library
          .lookupFunction<_StaticStringNative, _StaticStringDart>(
            'mdstream_binding_schema',
          ),
      _optionsSchema = library
          .lookupFunction<_StaticStringNative, _StaticStringDart>(
            'mdstream_binding_options_schema',
          ),
      _transitionSchema = library
          .lookupFunction<_StaticStringNative, _StaticStringDart>(
            'mdstream_transition_schema',
          ),
      _bufferSize = library.lookupFunction<_StructSizeNative, _StructSizeDart>(
        'mdstream_buffer_struct_size',
      ),
      _callResultSize = library
          .lookupFunction<_StructSizeNative, _StructSizeDart>(
            'mdstream_call_result_struct_size',
          ),
      _engineResultSize = library
          .lookupFunction<_StructSizeNative, _StructSizeDart>(
            'mdstream_engine_result_struct_size',
          ),
      _reducerResultSize = library
          .lookupFunction<_StructSizeNative, _StructSizeDart>(
            'mdstream_reducer_result_struct_size',
          ),
      _payloadResultSize = library
          .lookupFunction<_StructSizeNative, _StructSizeDart>(
            'mdstream_payload_result_struct_size',
          ),
      _allocationMetricsSize = library
          .lookupFunction<_StructSizeNative, _StructSizeDart>(
            'mdstream_allocation_metrics_struct_size',
          ),
      _processorSchedulerLimitsSize = library
          .lookupFunction<_StructSizeNative, _StructSizeDart>(
            'mdstream_processor_scheduler_limits_struct_size',
          ),
      _allocationMetrics = library
          .lookupFunction<_AllocationMetricsNative, _AllocationMetricsDart>(
            'mdstream_allocation_metrics',
          ),
      _engineNew = library.lookupFunction<_EngineNewNative, _EngineNewDart>(
        'mdstream_engine_new',
      ),
      _reducerNew = library.lookupFunction<_ReducerNewNative, _ReducerNewDart>(
        'mdstream_reducer_new',
      ),
      _reducerProcessorSchedulerLimits = library
          .lookupFunction<
            _ReducerProcessorSchedulerLimitsNative,
            _ReducerProcessorSchedulerLimitsDart
          >('mdstream_reducer_processor_scheduler_limits'),
      _engineFree = library.lookupFunction<_EngineFreeNative, _EngineFreeDart>(
        'mdstream_engine_free',
      ),
      _reducerFree = library
          .lookupFunction<_ReducerFreeNative, _ReducerFreeDart>(
            'mdstream_reducer_free',
          ),
      _engineAppend = library
          .lookupFunction<_EngineCallNative, _EngineCallDart>(
            'mdstream_engine_append',
          ),
      _engineRawAppendByteCeiling = library
          .lookupFunction<
            _EngineRawAppendByteCeilingNative,
            _EngineRawAppendByteCeilingDart
          >('mdstream_engine_raw_append_byte_ceiling'),
      _engineExecute = library
          .lookupFunction<_EngineCallNative, _EngineCallDart>(
            'mdstream_engine_execute',
          ),
      _reducerApply = library
          .lookupFunction<_ReducerCallNative, _ReducerCallDart>(
            'mdstream_reducer_apply_change',
          ),
      _reducerRecover = library
          .lookupFunction<_ReducerCallNative, _ReducerCallDart>(
            'mdstream_reducer_recover_snapshot',
          ),
      _reducerExecute = library
          .lookupFunction<_ReducerCallNative, _ReducerCallDart>(
            'mdstream_reducer_execute',
          ),
      _outputLen = library.lookupFunction<_OutputLenNative, _OutputLenDart>(
        'mdstream_output_len',
      ),
      _outputTake = library.lookupFunction<_OutputTakeNative, _OutputTakeDart>(
        'mdstream_output_take',
      ),
      _outputFree = library.lookupFunction<_OutputFreeNative, _OutputFreeDart>(
        'mdstream_output_free',
      ),
      _bufferFree = library.lookupFunction<_BufferFreeNative, _BufferFreeDart>(
        'mdstream_buffer_free',
      ),
      _engineFinalizer = ffi.NativeFinalizer(
        library
            .lookup<ffi.NativeFunction<_EngineFreeNative>>(
              'mdstream_engine_free',
            )
            .cast(),
      ),
      _reducerFinalizer = ffi.NativeFinalizer(
        library
            .lookup<ffi.NativeFunction<_ReducerFreeNative>>(
              'mdstream_reducer_free',
            )
            .cast(),
      );

  /// Loads and validates all required ABI symbols.
  factory NativeBindings.fromDynamicLibrary(ffi.DynamicLibrary library) {
    try {
      final bindings = NativeBindings._(library);
      bindings._validateAbi();
      _processLifetimes.add(bindings);
      return bindings;
    } on MdstreamException {
      rethrow;
    } catch (error) {
      throw MdstreamException(
        'failed to load the mdstream native ABI',
        detailCode: 'ffi.symbol_lookup',
        cause: error,
      );
    }
  }

  // NativeFinalizer callbacks and their libraries must outlive every Dart
  // wrapper, including wrappers that become unreachable together.
  static final List<NativeBindings> _processLifetimes = [];

  /// Package-internal library handle retained for native finalizer lifetime.
  final ffi.DynamicLibrary library;
  final _AbiVersionDart _abiVersion;
  final _StaticStringDart _packageVersion;
  final _StaticStringDart _bindingSchema;
  final _StaticStringDart _optionsSchema;
  final _StaticStringDart _transitionSchema;
  final _StructSizeDart _bufferSize;
  final _StructSizeDart _callResultSize;
  final _StructSizeDart _engineResultSize;
  final _StructSizeDart _reducerResultSize;
  final _StructSizeDart _payloadResultSize;
  final _StructSizeDart _allocationMetricsSize;
  final _StructSizeDart _processorSchedulerLimitsSize;
  final _AllocationMetricsDart _allocationMetrics;
  final _EngineNewDart _engineNew;
  final _ReducerNewDart _reducerNew;
  final _ReducerProcessorSchedulerLimitsDart _reducerProcessorSchedulerLimits;
  final _EngineFreeDart _engineFree;
  final _ReducerFreeDart _reducerFree;
  final _EngineRawAppendByteCeilingDart _engineRawAppendByteCeiling;
  final _EngineCallDart _engineAppend;
  final _EngineCallDart _engineExecute;
  final _ReducerCallDart _reducerApply;
  final _ReducerCallDart _reducerRecover;
  final _ReducerCallDart _reducerExecute;
  final _OutputLenDart _outputLen;
  final _OutputTakeDart _outputTake;
  final _OutputFreeDart _outputFree;
  final _BufferFreeDart _bufferFree;
  final ffi.NativeFinalizer _engineFinalizer;
  final ffi.NativeFinalizer _reducerFinalizer;

  /// Native ABI version reported by the loaded library.
  int get abiVersion => _abiVersion();

  /// mdstream package version reported by the loaded library.
  String get packageVersion => _readStaticString(_packageVersion(), 'version');

  /// Binding schema reported by the loaded library.
  String get bindingSchema => _readStaticString(_bindingSchema(), 'schema');

  /// Binding options schema reported by the loaded library.
  String get optionsSchema =>
      _readStaticString(_optionsSchema(), 'options schema');

  /// Transition schema reported by the loaded library.
  String get transitionSchema =>
      _readStaticString(_transitionSchema(), 'transition schema');

  /// Creates an exactly-once native engine handle from encoded options.
  NativeEngineHandle createEngine(Uint8List options) {
    final result = _withInput(options, _engineNew);
    if (result.status != 0) {
      if (result.engine.address != 0) {
        _engineFree(result.engine);
      }
      throw _exceptionFromBuffer(result.error, result.status);
    }
    try {
      _expectEmptyError(result.error, 'engine constructor');
      if (result.engine.address == 0) {
        throw _invalidNativeResult('engine constructor returned a null handle');
      }
    } catch (_) {
      if (result.engine.address != 0) {
        _engineFree(result.engine);
      }
      rethrow;
    }
    return NativeEngineHandle._(this, result.engine);
  }

  /// Creates an exactly-once native reducer handle from encoded options.
  NativeReducerHandle createReducer(Uint8List options) {
    final result = _withInput(options, _reducerNew);
    if (result.status != 0) {
      if (result.reducer.address != 0) {
        _reducerFree(result.reducer);
      }
      throw _exceptionFromBuffer(result.error, result.status);
    }
    late final MdstreamProcessorSchedulerLimits processorSchedulerLimits;
    try {
      _expectEmptyError(result.error, 'reducer constructor');
      if (result.reducer.address == 0) {
        throw _invalidNativeResult(
          'reducer constructor returned a null handle',
        );
      }
      final nativeLimits = _reducerProcessorSchedulerLimits(result.reducer);
      if (nativeLimits.maxInFlightJobs < 1 ||
          nativeLimits.maxQueuedCandidates < 1) {
        throw _invalidNativeResult(
          'reducer returned non-positive processor scheduler limits',
        );
      }
      processorSchedulerLimits = MdstreamProcessorSchedulerLimits(
        maxInFlightJobs: nativeLimits.maxInFlightJobs,
        maxQueuedCandidates: nativeLimits.maxQueuedCandidates,
      );
    } catch (_) {
      if (result.reducer.address != 0) {
        _reducerFree(result.reducer);
      }
      rethrow;
    }
    return NativeReducerHandle._(
      this,
      result.reducer,
      processorSchedulerLimits,
    );
  }

  /// Reads current process-wide native allocation counters.
  NativeAllocationMetrics allocationMetrics() {
    final value = _allocationMetrics();
    return NativeAllocationMetrics._(
      engineHandles: value.engineHandles,
      reducerHandles: value.reducerHandles,
      outputs: value.outputs,
      buffers: value.buffers,
      bufferBytes: value.bufferBytes,
    );
  }

  List<NativePayload> _appendToEngine(
    ffi.Pointer<_NativeMdstreamEngine> engine,
    Uint8List bytes,
  ) => _drain(
    _withInput(bytes, (data, len) => _engineAppend(engine, data, len)),
  );

  int _rawAppendByteCeiling(ffi.Pointer<_NativeMdstreamEngine> engine) =>
      _engineRawAppendByteCeiling(engine);

  List<NativePayload> _executeEngineCommand(
    ffi.Pointer<_NativeMdstreamEngine> engine,
    Uint8List command,
  ) => _drain(
    _withInput(command, (data, len) => _engineExecute(engine, data, len)),
  );

  List<NativePayload> _applyToReducer(
    ffi.Pointer<_NativeMdstreamReducer> reducer,
    Uint8List change,
  ) => _drain(
    _withInput(change, (data, len) => _reducerApply(reducer, data, len)),
  );

  List<NativePayload> _recoverReducer(
    ffi.Pointer<_NativeMdstreamReducer> reducer,
    Uint8List snapshot,
  ) => _drain(
    _withInput(snapshot, (data, len) => _reducerRecover(reducer, data, len)),
  );

  List<NativePayload> _executeReducerCommand(
    ffi.Pointer<_NativeMdstreamReducer> reducer,
    Uint8List command,
  ) => _drain(
    _withInput(command, (data, len) => _reducerExecute(reducer, data, len)),
  );

  void _validateAbi() {
    if (abiVersion != _expectedAbiVersion) {
      throw MdstreamException(
        'unsupported mdstream ABI version $abiVersion',
        status: 5,
        statusName: 'MDSTREAM_UNSUPPORTED_SCHEMA',
        detailCode: 'ffi.abi_version',
      );
    }
    final actualBindingSchema = bindingSchema;
    final actualOptionsSchema = optionsSchema;
    if (actualBindingSchema != _expectedBindingSchema ||
        actualOptionsSchema != _expectedOptionsSchema) {
      throw MdstreamException(
        'unsupported mdstream binding schema',
        status: 5,
        statusName: 'MDSTREAM_UNSUPPORTED_SCHEMA',
        detailCode: 'ffi.binding_schema',
        schema: actualBindingSchema,
      );
    }
    validateNativeTransitionSchema(transitionSchema);
    final sizes = <String, (int, int)>{
      'MdstreamBuffer': (_bufferSize(), ffi.sizeOf<_MdstreamBuffer>()),
      'MdstreamCallResult': (
        _callResultSize(),
        ffi.sizeOf<_MdstreamCallResult>(),
      ),
      'MdstreamEngineResult': (
        _engineResultSize(),
        ffi.sizeOf<_MdstreamEngineResult>(),
      ),
      'MdstreamReducerResult': (
        _reducerResultSize(),
        ffi.sizeOf<_MdstreamReducerResult>(),
      ),
      'MdstreamPayloadResult': (
        _payloadResultSize(),
        ffi.sizeOf<_MdstreamPayloadResult>(),
      ),
      'MdstreamAllocationMetrics': (
        _allocationMetricsSize(),
        ffi.sizeOf<_MdstreamAllocationMetrics>(),
      ),
      'MdstreamProcessorSchedulerLimits': (
        _processorSchedulerLimitsSize(),
        ffi.sizeOf<_MdstreamProcessorSchedulerLimits>(),
      ),
    };
    for (final MapEntry(key: name, value: (native, dart)) in sizes.entries) {
      if (native != dart) {
        throw MdstreamException(
          '$name layout mismatch: native=$native dart=$dart',
          status: 5,
          statusName: 'MDSTREAM_UNSUPPORTED_SCHEMA',
          detailCode: 'ffi.struct_layout',
        );
      }
    }
  }

  List<NativePayload> _drain(_MdstreamCallResult result) {
    if (result.status != 0) {
      if (result.output.address != 0) {
        _outputFree(result.output);
      }
      throw _exceptionFromBuffer(result.error, result.status);
    }
    if (result.output.address == 0) {
      _expectEmptyError(result.error, 'successful call');
      throw _invalidNativeResult('successful call returned a null output');
    }
    try {
      _expectEmptyError(result.error, 'successful call');
      final length = _outputLen(result.output);
      final payloads = <NativePayload>[];
      for (var index = 0; index < length; index += 1) {
        final payload = _outputTake(result.output, index);
        if (payload.status != 0) {
          throw _exceptionFromBuffer(payload.data, payload.status);
        }
        payloads.add(NativePayload._(payload.kind, _copyAndFree(payload.data)));
      }
      return payloads;
    } finally {
      _outputFree(result.output);
    }
  }

  Uint8List _copyAndFree(_MdstreamBuffer buffer) {
    try {
      if (buffer.data.address == 0) {
        if (buffer.len != 0) {
          throw _invalidNativeResult('native buffer has a null data pointer');
        }
        return Uint8List(0);
      }
      if (buffer.len == 0) {
        throw _invalidNativeResult('native buffer has data with zero length');
      }
      return Uint8List.fromList(buffer.data.asTypedList(buffer.len));
    } finally {
      _bufferFree(buffer);
    }
  }

  MdstreamException _exceptionFromBuffer(_MdstreamBuffer buffer, int status) {
    final bytes = _copyAndFree(buffer);
    if (bytes.isEmpty) {
      return MdstreamException(
        'native call failed without an error payload',
        status: status,
        detailCode: 'ffi.missing_error',
      );
    }
    return MdstreamException.fromJsonBytes(bytes, fallbackStatus: status);
  }

  void _expectEmptyError(_MdstreamBuffer buffer, String operation) {
    if (buffer.data.address == 0 && buffer.len == 0) {
      return;
    }
    final bytes = _copyAndFree(buffer);
    throw _invalidNativeResult(
      '$operation returned an unexpected error payload: ${utf8.decode(bytes, allowMalformed: true)}',
    );
  }
}

/// Package-internal validation for the transition schema reported by native.
void validateNativeTransitionSchema(String actual) {
  if (actual != protocol.transitionSchema) {
    throw MdstreamException(
      'unsupported mdstream transition schema $actual',
      status: 5,
      statusName: 'MDSTREAM_UNSUPPORTED_SCHEMA',
      detailCode: 'ffi.transition_schema',
      schema: actual,
    );
  }
}

/// Package-internal exact-once engine ownership for the high-level runtime.
final class NativeEngineHandle implements ffi.Finalizable {
  NativeEngineHandle._(this._bindings, this._pointer) {
    _bindings._engineFinalizer.attach(this, _pointer.cast(), detach: this);
  }

  final NativeBindings _bindings;
  ffi.Pointer<_NativeMdstreamEngine> _pointer;

  /// Whether native ownership has already been released.
  bool get isClosed => _pointer.address == 0;

  /// Appends UTF-8 source bytes and returns copied native payloads.
  List<NativePayload> append(Uint8List bytes) =>
      _withPointer((pointer) => _bindings._appendToEngine(pointer, bytes));

  /// Conservative native raw-byte admission ceiling for the next append.
  ///
  /// A negative value is an unsigned `size_t` outside Dart's signed FFI range,
  /// so it represents no useful local bound and defers to native append.
  int? get rawAppendByteCeiling => _withPointer((pointer) {
    final ceiling = _bindings._rawAppendByteCeiling(pointer);
    return ceiling < 0 ? null : ceiling;
  });

  /// Executes an encoded engine command and returns copied native payloads.
  List<NativePayload> execute(Uint8List command) => _withPointer(
    (pointer) => _bindings._executeEngineCommand(pointer, command),
  );

  /// Releases native ownership; subsequent calls are rejected.
  void close() {
    final pointer = _pointer;
    if (pointer.address == 0) {
      return;
    }
    _pointer = ffi.nullptr;
    _bindings._engineFinalizer.detach(this);
    _bindings._engineFree(pointer);
  }

  T _withPointer<T>(T Function(ffi.Pointer<_NativeMdstreamEngine>) operation) {
    final pointer = _pointer;
    if (pointer.address == 0) {
      throw _closedException('engine');
    }
    return operation(pointer);
  }
}

/// Package-internal exact-once reducer ownership for the high-level runtime.
final class NativeReducerHandle implements ffi.Finalizable {
  NativeReducerHandle._(
    this._bindings,
    this._pointer,
    this.processorSchedulerLimits,
  ) {
    _bindings._reducerFinalizer.attach(this, _pointer.cast(), detach: this);
  }

  final NativeBindings _bindings;
  ffi.Pointer<_NativeMdstreamReducer> _pointer;

  /// Effective native capacity for host-side processor scheduling.
  final MdstreamProcessorSchedulerLimits processorSchedulerLimits;

  /// Whether native ownership has already been released.
  bool get isClosed => _pointer.address == 0;

  /// Applies an encoded change and returns copied native payloads.
  List<NativePayload> apply(Uint8List change) =>
      _withPointer((pointer) => _bindings._applyToReducer(pointer, change));

  /// Recovers from an encoded snapshot and returns copied native payloads.
  List<NativePayload> recover(Uint8List snapshot) =>
      _withPointer((pointer) => _bindings._recoverReducer(pointer, snapshot));

  /// Executes an encoded reducer command and returns copied native payloads.
  List<NativePayload> execute(Uint8List command) => _withPointer(
    (pointer) => _bindings._executeReducerCommand(pointer, command),
  );

  /// Releases native ownership; subsequent calls are rejected.
  void close() {
    final pointer = _pointer;
    if (pointer.address == 0) {
      return;
    }
    _pointer = ffi.nullptr;
    _bindings._reducerFinalizer.detach(this);
    _bindings._reducerFree(pointer);
  }

  T _withPointer<T>(T Function(ffi.Pointer<_NativeMdstreamReducer>) operation) {
    final pointer = _pointer;
    if (pointer.address == 0) {
      throw _closedException('reducer');
    }
    return operation(pointer);
  }
}

T _withInput<T>(
  Uint8List bytes,
  T Function(ffi.Pointer<ffi.Uint8>, int) operation,
) {
  if (bytes.isEmpty) {
    return operation(ffi.nullptr, 0);
  }
  final pointer = calloc<ffi.Uint8>(bytes.length);
  try {
    pointer.asTypedList(bytes.length).setAll(0, bytes);
    return operation(pointer, bytes.length);
  } finally {
    calloc.free(pointer);
  }
}

String _readStaticString(ffi.Pointer<ffi.Char> pointer, String field) {
  if (pointer.address == 0) {
    throw _invalidNativeResult('$field pointer is null');
  }
  return pointer.cast<Utf8>().toDartString();
}

MdstreamException _invalidNativeResult(String message) =>
    MdstreamException(message, detailCode: 'ffi.invalid_result');

MdstreamException _closedException(String handle) => MdstreamException(
  'mdstream $handle is closed',
  status: 1,
  statusName: 'MDSTREAM_INVALID_ARGUMENT',
  detailCode: 'bindings.closed',
);
