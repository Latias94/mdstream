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

Merman execution is synchronous. Cancellation is checked before and after the
render call but cannot interrupt parsing, layout, or rendering already in
progress. Treat processors as trusted cooperative code and use caller-managed
process isolation for adversarial diagrams.

SVG is untrusted derived output. Consumers must apply an embedding and
sanitization policy appropriate to their UI; do not pass it to an unrestricted
HTML `innerHTML` sink.
