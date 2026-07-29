# Extensions and Processors

mdstream has two extension boundaries with different responsibilities:
setup-only source framing and post-reduction content processors.

## Setup-Only Custom Blocks

`StreamEngineBuilder::custom_block` registers a versioned standalone block
grammar before input starts. Configuration is sealed after the first append.
Runtime grammar or transformer mutation is intentionally unavailable because it
would reinterpret stable history.

Custom delimiters are full physical lines at column zero, open only at document
start or after a blank line, and pair through one global LIFO stack. Markdown
code/HTML raw regions protect delimiter-looking text. The complete grammar is
specified by [ADR 0003](ADR_0003_STANDALONE_CUSTOM_BLOCKS.md).

`pulldown-cmark` remains the only CommonMark/GFM semantic compiler. LALRPOP is
not used in the Markdown path: a generated LR grammar would not supply
CommonMark semantics, streaming checkpoints, lifecycle, or stable identity. It
remains a possible implementation tool for a future independent closed DSL.

## Four Separate Extension Planes

A complete custom content feature composes four contracts instead of adding a
parser callback or renderer registry:

1. A setup-only `CustomBlockSpec` declares sealed source framing.
2. The compiler emits a typed `ContentKind::Custom` node with a namespace,
   name, opacity flag, and bounded string attributes.
3. An optional versioned processor derives an artifact from the typed node.
4. The host maps the typed node and artifact protocol to its own display code.

The first two planes are canonical. Processor artifacts and host display state
are derived. A host may replace its renderer or discard an artifact without
rewriting Markdown history, node identity, Content IR, or transition facts.
Custom nodes therefore do not carry framework component names, arbitrary JSON,
animation metadata, or executable callbacks.

## Content Processors

Processors consume typed `ContentNode` input after canonical reduction and
produce text, binary, or structured failure artifacts. Typical processors
include:

- citation resolution;
- Mermaid rendering through optional `mdstream-merman`;
- math or code compilation owned by an application;
- domain-specific custom block interpretation.

Artifacts never enter Content IR. A processor must declare a stable ID,
implementation version, configuration version, and provisional-input
capability. Expensive processors should default to stable nodes. Provisional
preview requires both processor capability and host policy.

## Scheduling Contract

`ArtifactHost` issues owned requests and validates completions; it does not run
an executor. Hosts may schedule requests on threads, tasks, workers, or
processes. They must propagate cancellation and submit completion through the
same request generation.

Registering a host-language processor after nodes already exist scans the
current typed tree. Registration snapshots descriptor/configuration identity,
so later mutable getters cannot change the artifact slot or break disposal.

## Safety and Limits

The host bounds processor input bytes, slots, in-flight work, pending change
bytes, retained artifacts, and retained artifact bytes. Processor-specific
limits may reject source/model complexity before expensive work. These limits
are accounting controls, not a sandbox. Merman constructs SVG before mdstream
can apply its retained-artifact cap, so adversarial diagrams require external
process isolation.

The default Rust, WASM, npm, Dart, and Flutter packages do not depend on
Merman. Applications opt into `mdstream-merman` on its separate Rust 1.95 lane.

## Merman Processor Recipe

[`render_golden`](../mdstream-merman/examples/render_golden.rs) replays the
packaged Golden AI Stream through `StreamEngine` and `TransitionReducer`, then
selects the typed stable Mermaid code node and issues an `ArtifactHost` request.
It does not construct protocol nodes or changes by hand. Run its executable
contract with:

```console
cargo +1.95.0 run --manifest-path mdstream-merman/Cargo.toml \
  --example render_golden -- --assert
```

The recipe reports the full processor request identity, artifact protocol, and
media type. The canonical snapshot is checked before and after rendering to
prove that the SVG remains derived host state. Request generations let the host
reject late A-to-B-to-A completions even when the first and final semantic
inputs are equal. Same-floor recovery retains eligible keyed work; reset and
advanced replacement clear it.

## SVG Trust Boundary

Merman returns an opaque `image/svg+xml` artifact. mdstream does not sanitize,
execute, mount, or inspect that markup. A web host must pass the bytes through a
named `sanitizeSvgArtifact` boundary that rejects active content and unwanted
external references, or render them in a separately isolated document/process.
An embedded host owns the equivalent allowlist and resource-loading policy.
Direct insertion into an unrestricted HTML sink is not an adoption pattern.

`sanitizeSvgArtifact` is a name for required application policy, not an API or
implementation exported by mdstream. The Golden recipe stops at that boundary
and never emits the SVG bytes to a display sink.

Source, model, label, edge, output, and retention limits make resource use
accountable, but they do not preempt synchronous parser/layout/render work or
bound allocator peaks. Cancellation is cooperative. Hosts accepting untrusted
diagram or custom processor input must own a timeout plus worker/process
isolation; an in-process byte limit is not a compute sandbox.
