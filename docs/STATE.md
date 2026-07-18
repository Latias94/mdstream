# State and Recovery

The protocol exposes every transition needed to reproduce a streaming
document. Hosts must not infer state from a `committed + pending` convention.

## Coordinates

Every accepted document state has:

- `Epoch`: document generation, changed only by reset;
- `Sequence`: monotonic change position within the epoch;
- `ChangeId`: opaque retry/fork discriminator;
- `SourceCursor`: accepted canonical source bytes;
- `projection_cursor`: source prefix represented by typed Content IR.

Decimal IDs and counters cross JavaScript and Dart bindings as decimal strings.
Opaque versions and change IDs remain strings.

## Projection Frontier

Accepted source may temporarily advance beyond `projection_cursor` when an
append cannot be projected exactly within the bounded work budget. The
uncovered range is canonical pending source. Native documents expose it
directly; foreign bindings expose a bounded, on-demand pending-source view.
Finalization closes this gap before the document becomes finalized.

Pending source is raw display state, not a second Markdown tree. Adapters may
show it while rich formatting catches up, but must not parse it, assign stable
node identities to it, or include the full text in every delta notification.

## Lifecycle

```text
uninitialized -> open -> finalized
                    \
                     reset -> open in a successor epoch
```

The first `finish` stabilizes remaining content and emits one terminal change.
Repeated finish is an idempotent no-op. Append after finish returns a typed
terminal error without changing state. Reset emits an `EpochStart` linked to
the predecessor coordinate and invalidates prior-epoch artifacts and keys.

Node stability is separate:

- `provisional`: projection may change as the source frontier advances;
- `stable`: structurally settled for processor policy purposes.

A stable node may still receive semantic correction, such as a late reference
definition. Correction preserves `NodeId` and changes `NodeVersion`.

## Reducer Outcomes

| Outcome | Mutation | Host action |
| --- | --- | --- |
| `Applied` | Accepts the next continuous change | Apply `ChangeImpact` |
| `Idempotent` | None | Ignore exact retry |
| `Stale` | None | Ignore an older sequence |
| `RecoveryRequired` | Status only; last-good document is retained | Request a snapshot |
| `Recovered` | Atomically replaces canonical state | Invalidate all materialized views |

A same-sequence change with a different ID is a fork. A future sequence is a
gap. Either moves the reducer to `NeedsSnapshot`. Ordinary deltas are rejected
until a validated current snapshot or predecessor-linked epoch start recovers
the reducer.

## Change Impact

`ChangeImpact` reports changed and removed node/resource IDs plus source,
projection, lifecycle, roots, and full-replacement flags. Changed IDs include
removed IDs because any cached view under that identity is invalid. A host
materializes only those views; a missing view removes its cached object.

On `full_replace`, hosts invalidate every retained canonical and derived view.
Flutter keys include both epoch and node ID. TypeScript stores and Flutter
controllers publish the new root snapshot before notifying focused listeners,
so callbacks observe one coherent transition.

## Processor Freshness

A processor request key includes epoch, node ID, node/input versions, processor
ID/version, configuration version, and host-issued request generation. A result
is accepted only if the complete key is still current. This rejects old A
results after an A-to-B-to-A input sequence and keeps artifacts outside
canonical snapshots.
