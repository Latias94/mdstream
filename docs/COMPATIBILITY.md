# Compatibility

mdstream defines one canonical 0.4 behavior and records selected upstream
behaviors as versioned compatibility evidence. Compatibility profiles are not
claims of renderer parity.

## Canonical Dialect

Markdown semantics follow `pulldown-cmark 0.13.x` CommonMark/GFM behavior plus
versioned mdstream custom blocks and `mdstream.citation/1`. Parser types and
renderer output are not part of the protocol.

Final source, normalized Content IR, stable IDs, node versions, and lifecycle
must be invariant across every legal UTF-8 chunk partition for small fixtures
and reproducible seeded schedules for larger fixtures. Intermediate sequence
counts and projection coverage may differ.

## Upstream Profiles

The conformance corpus contains pinned characterizations for:

- Streamdown block framing and selected HTML/table/footnote cases;
- Remend-style incomplete Markdown repair expectations;
- Incremark final-AST behavior for mixed content and late definitions.

Each fixture records repository, package/version, upstream path, pipeline, and
profile ID. A fixture derived from mdstream itself cannot be labeled upstream
parity. When an upstream changes, add or update a profile explicitly rather
than silently changing canonical behavior.

## Intentional Differences

- mdstream is a headless state engine, not a React renderer.
- Stable IDs are not source offsets, array positions, or React keys generated
  during rendering.
- Ordered changes and snapshot recovery replace two-state pending/stable APIs.
- Typed projection coverage may lag accepted source until a bounded checkpoint;
  the uncovered source remains observable.
- Renderer repair, themes, highlighting, layout, and browser policy remain host
  concerns.

## Versioning

`mdstream.protocol/0.4` and `mdstream.bindings/0.4` are separate contracts.
Protocol changes require fixture/schema updates and cross-runtime replay. A
binding-envelope change does not implicitly change canonical Content IR.
Breaking public changes require a new contract version; unknown schema or
discriminant values fail with typed errors.

Migration from the removed 0.3 block/update surface is documented in
[USAGE.md](USAGE.md) and the repository README. No compatibility aliases are
provided.
