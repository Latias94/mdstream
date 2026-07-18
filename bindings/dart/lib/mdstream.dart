/// Framework-neutral Dart bindings for the mdstream content engine.
library;

export 'src/batching.dart'
    show
        BatchMetrics,
        BatchOperationException,
        LosslessInputBatcher,
        utf8ByteLength;
export 'src/engine.dart'
    show
        BatchedRecoverySnapshot,
        EngineResult,
        EngineTransportMetrics,
        MdstreamEngine,
        MdstreamInputBatcher;
export 'src/errors.dart';
export 'src/options.dart';
export 'src/protocol.dart'
    hide
        canonicalChangeBytesFromOwned,
        canonicalChangeBytesView,
        canonicalSnapshotBytesFromOwned,
        canonicalSnapshotBytesView,
        decodeDecimalU128,
        decodeDecimalU64,
        validateDecimalU128Input,
        validateDecimalU64Input;
export 'src/reducer_handle.dart'
    show
        ArtifactSlot,
        MdstreamReducer,
        MdstreamStateSnapshot,
        MdstreamStateView,
        ReducerResult,
        ReducerTransportMetrics;
export 'src/runtime.dart';
export 'src/views.dart';
