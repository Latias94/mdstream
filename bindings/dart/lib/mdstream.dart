/// Framework-neutral Dart bindings for the mdstream content engine.
library;

export 'src/batching.dart'
    show
        BatchMetrics,
        BatchOperationException,
        LosslessInputBatcher,
        utf8ByteLength,
        utf8ByteLengthAtMost;
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
    show
        bindingSchema,
        bindingOptionsSchema,
        transitionSchema,
        mdstreamAbiVersion,
        DecimalCounter,
        Epoch,
        Sequence,
        SourceCursor,
        RequestGeneration,
        ContinuityGeneration,
        NodeId,
        ResourceId,
        ChangeId,
        NodeVersion,
        ResourceVersion,
        StructureVersion,
        ProcessorInputVersion,
        CanonicalChangeBytes,
        CanonicalSnapshotBytes,
        BindingPayloadKind,
        BindingStatus;
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
