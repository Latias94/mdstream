# Document State (`DocumentState`)

`DocumentState` is a small, UI-friendly container that holds the renderable state of a streaming
Markdown document:

- `committed`: stable blocks (append-only, never change)
- `pending`: the only block that may change on each tick

It exists to make `Update.reset` handling hard to get wrong, and to support partial refresh via
`Update.invalidated` for adapters.

## Why it is separate from `MdStream`

`MdStream` is the parser. `DocumentState` is UI state.

Keeping them separate allows:

- render-agnostic integration (egui, gpui, TUI, etc.)
- pipeline flexibility (`AnalyzedStream`, adapters, custom transformers)
- predictable ownership and lifetimes

## Usage

```rust
use mdstream::{DocumentState, MdStream, Options};

let mut stream = MdStream::new(Options::default());
let mut state = DocumentState::new();

let update = stream.append("Hello **wor");
let applied = state.apply(update);

if applied.reset {
    // Drop any external caches derived from old blocks.
}
```

For `AnalyzedStream`, apply `u.update`:

```rust
use mdstream::{AnalyzedStream, BlockHintAnalyzer, CodeFenceAnalyzer, DocumentState, Options};

let analyzer = (CodeFenceAnalyzer::default(), BlockHintAnalyzer::default());
let mut stream = AnalyzedStream::new(Options::default(), analyzer);
let mut state = DocumentState::new();

let u = stream.append("```rust\nfn main() {}\n");
state.apply(u.update);
```

