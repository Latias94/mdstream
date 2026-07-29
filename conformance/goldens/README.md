# Transition goldens

`transition-v1.json` is intentionally outside `conformance/fixtures`. The generic protocol fixture
loader scans that directory, while this file has a binding-specific schema and dedicated consumers.

The Rust test builds every case with canonical protocol types, applies it through the real
`mdstream-bindings-core` reducer-update encoder, and compares both the exact JSON bytes and a
deterministic camelCase normalization. The TypeScript test sends those exact bytes through the
strict typed decoder and compares its result with the same normalized value.

Refresh the generated file only after an intentional wire-contract change:

```sh
UPDATE_TRANSITION_GOLDEN=1 cargo +1.85 nextest run \
  -p mdstream-bindings-core \
  --test transition_golden \
  -E 'test(transition_v1_golden_matches_the_real_binding_encoder)'
```

Verify both consumers:

```sh
cargo +1.85 nextest run -p mdstream-bindings-core --test transition_golden
pnpm --dir bindings/typescript exec vitest run tests/transition_golden.test.ts
pnpm --dir bindings/typescript typecheck
```

The fixture structs use `deny_unknown_fields`, and the TypeScript fixture reader enforces the same
closed field sets. Old-draft and future transition-schema cases are derived from valid generated
wire so negative tests do not maintain separate hand-written payloads.
