# Changelog

## 0.4.0

- Breaking: `MdstreamEngine.createBatcher` now takes named byte and constituent limits, `flush()` returns an ordered collection, and the callback-based public `LosslessInputBatcher` plus ambiguous forwarding counters were removed.
- Added one live batching lease per engine, explicit `inspectPending`/`retryPending`/`takePending`/`discardPending` recovery, immutable partial-failure ownership, and R9 work counters. The test-only KTD3 gate selected constituent-first production appends because joined copying failed every workload budget; no joined/replay branch ships in the production queue.
- Added Flutter-independent Dart 3.8+ FFI bindings for owned mdstream engine and reducer sessions using a host-supplied compatible `mdstream-ffi` dynamic library.
- Added immutable root, node, resource, and exhaustive typed Content IR views plus bounded on-demand pending source with stable identity, precise invalidation, and explicit snapshot recovery.
- Added lossless input batching, validated session/custom-block options, processor leases and artifact views, cancellation, and stale-result rejection.
- Added opt-in immutable `mdstream.transitions/1` views plus ordered `EngineResult.transitionFacts` and `ReducerResult.transitionFacts` access for host-defined reveal, correction, and layout policy.
- Added ABI/package/schema checks, structured errors, exact-once native ownership, canonical decimal identifiers, and package verification that excludes native binaries and framework dependencies.
- Added validated `MdstreamProtocolLimits`, `MdstreamCompilerLimits`, `MdstreamEngineLimits`, `MdstreamProcessorLimits`, and `MdstreamWireLimits`; compiler-owned Markdown work and definition-registry budgets live only in `MdstreamCompilerLimits`, and unsupported option names fail at compile time.
- Added native-derived effective processor scheduler limits so framework adapters cannot drift from Rust defaults or option normalization.
- Added completion/state enums and sealed artifact change/payload variants that reject inconsistent wire values.
