# ADR 0003: Standalone Custom Block Grammar

- Status: Accepted
- Date: 2026-07-14
- Scope: `mdstream` 0.4 custom Content IR extensions

## Context

Configured custom tags must coexist with CommonMark/GFM while preserving chunk
invariance, stable identity, and linear work. The previous scanner masked every
configured tag before asking Pulldown for Markdown context. A rejected tag could
therefore change paragraph or HTML-container classification and promote a later
tag into a root custom node. Per-tag stacks, backward line searches, and copied
pending lines also made canonical pairing and linear work difficult to prove.

LALRPOP does not address this boundary. It can generate an LR parser for a
closed grammar, but it does not supply CommonMark semantics, resumable streaming
state, stable node identity, or mdstream lifecycle transitions.

## Decision

Custom blocks use a versioned standalone grammar owned by mdstream framing:

1. Opening and closing tags occupy an entire physical line at column zero;
   trailing spaces or tabs are allowed.
2. An opening tag is structural only at document start or after a blank physical
   line. This deliberately forbids implicit paragraph interruption and tags
   nested inside Markdown indentation.
3. Closing tags pair strictly with the current global LIFO stack top. A
   mismatched or trailing-text closing tag remains source content.
4. Nested non-opaque blocks follow the same blank-boundary rule. Opaque blocks
   balance only standalone openings of their own name.
5. Fenced code, raw-text HTML, comments, CDATA, processing instructions, and
   declarations protect delimiter-looking content.
6. A delimiter at the end of an append is tentative because a later chunk can
   add trailing text. A physical line ending or `finish` confirms it.
7. The custom tokenizer and topology builder make one forward pass, share one
   bounded stack, and perform node/depth/children admission before allocation.
   Pulldown remains the only CommonMark/GFM semantic compiler for the
   non-overlapping Markdown gaps.

## Rejected Alternatives

- A global masked Pulldown pass: rejected because unaccepted candidates changed
  Markdown context.
- A second incremental Markdown grammar: rejected because it duplicates
  Pulldown semantics and makes conformance unmaintainable.
- LALRPOP in the Markdown path: rejected because grammar generation does not
  solve streaming lifecycle or runtime incremental reuse. It remains an option
  for a future independent, closed processor DSL.

## Consequences

This is intentionally breaking for 0.4. Tags that were indented, attached to a
paragraph, or inferred to interrupt paragraphs from their HTML name are no
longer custom blocks. Producers gain a small explicit grammar, and consumers get
deterministic topology, chunk-independent final output, and bounded scanner work.
