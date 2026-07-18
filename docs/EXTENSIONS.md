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
