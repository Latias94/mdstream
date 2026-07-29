# mdstream-merman

`mdstream-merman` is the optional Rust 1.95 headless Mermaid adapter for
mdstream's renderer-neutral processor protocol. It is a standalone workspace
and is excluded from mdstream's Rust 1.85 core workspace.

The adapter accepts typed Mermaid code-block nodes. Stable nodes are supported
by default; provisional processing requires explicit capability and host policy.
Successful renders become derived `image/svg+xml` artifacts keyed and retained
by `ArtifactHost`. They never enter canonical Content IR or snapshots.
`DEFAULT_CONFIGURATION_VERSION` applies only to default options; callers that
change adapter options must issue requests with a distinct configuration
version so the host key remains complete.

## Golden stream recipe

This is a focused rich-content processor recipe in the repository's [example learning path](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#merman-artifact). It requires Rust 1.95 or newer and runs the packaged, provider-free AI stream through the normal Markdown path:

```console
cargo +1.95.0 run --manifest-path mdstream-merman/Cargo.toml \
  --example render_golden -- --assert
```

The recipe executes this ownership chain:

```text
Golden token chunks
  -> StreamEngine
  -> TransitionReducer
  -> typed stable Mermaid code node
  -> ArtifactHost request generation
  -> MermaidProcessor
  -> opaque image/svg+xml artifact
  -> host-owned sanitizeSvgArtifact boundary
```

Its output reports the host-owned artifact request key, artifact protocol,
media type, and required host handoff, then ends with
`mdstream-merman golden stream: ok`. It deliberately does not print, mount, or
execute the SVG. `sanitizeSvgArtifact` names the next application-owned
boundary; it is not a sanitizer supplied by this crate. A native application
may instead pass the opaque bytes to an isolated renderer with an equivalent
resource-loading policy. Continue with the generic [processor lifecycle recipe](https://github.com/Latias94/mdstream/blob/main/docs/EXAMPLES.md#processor-lifecycle) to isolate artifact freshness from Mermaid rendering.

`ArtifactHost` owns request generations and rejects late completions. A host
should propagate each request's cooperative cancellation signal to its
scheduler. Reset and advanced snapshot recovery clear prior derived state;
same-floor recovery may retain work whose complete request key is still
eligible. See `tests/adoption_rust.rs` and `tests/mermaid_processor.rs` for the
executable recovery and A-to-B-to-A generation contracts.

## Resource boundaries

- `max_source_bytes` is checked before Merman parses the semantic code value.
- Flowchart and class node, edge, subgraph, namespace, and label limits are
  checked after their semantic model exists and before layout. Other Mermaid
  families do not yet have equivalent model-stage hard caps and are trusted
  cooperative work.
- `max_svg_bytes` is a post-render, pre-retention limit. Merman 0.8.0-alpha.3
  has already materialized the complete SVG `String` when this adapter checks
  it. This limit does not bound renderer peak allocation.
- `ArtifactHost::max_artifact_bytes` additionally charges the artifact protocol
  and media-type envelope, not only the raw SVG payload.
- The reported live input/output byte proxy is `source.len() + svg.len()`. It is
  not allocator telemetry, process RSS, or a renderer peak-memory measurement.

Merman execution is synchronous. Cancellation is cooperative: it is checked
before and after the render call but cannot interrupt parsing, layout, or
rendering already in progress. Limits and cancellation are not a sandbox.
Treat processors as trusted cooperative code and use caller-managed process
isolation, timeouts, and operating-system resource controls for adversarial
diagrams.

SVG is untrusted derived output. Consumers must apply an embedding and
sanitization policy appropriate to their UI; do not pass it directly to an
unrestricted markup or script-capable sink.
