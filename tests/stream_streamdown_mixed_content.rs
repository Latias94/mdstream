mod support;

use mdstream::Options;

#[test]
fn streamdown_benchmark_realistic_ai_response_chunking_invariance() {
    // From Streamdown's parse-blocks benchmark ("Mixed Content").
    let markdown = r#"# AI Response Example

Here's a comprehensive example of markdown content:

## Code Example

```typescript
interface User {
  id: string;
  name: string;
  email: string;
}

function getUser(id: string): User {
  return { id, name: "John", email: "john@example.com" };
}
```

## Math Formula

The quadratic formula is:

$$
x = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}
$$

## Lists and Tables

| Feature | Status |
|---------|--------|
| Bold    | ✓      |
| Italic  | ✓      |
| Code    | ✓      |

### Checklist

- [x] Implement parser
- [ ] Add tests
- [ ] Write docs

> **Note**: This is a blockquote with **bold** text.

For more info, see [documentation](https://example.com).
"#;

    let opts = Options::default();
    let blocks_whole = support::collect_final_raw(support::chunk_whole(markdown), opts.clone());
    let blocks_lines = support::collect_final_raw(support::chunk_lines(markdown), opts.clone());
    let blocks_rand = support::collect_final_raw(
        support::chunk_pseudo_random(
            markdown,
            "streamdown_benchmark_realistic_ai_response_chunking_invariance",
            0,
            40,
        ),
        opts.clone(),
    );

    assert_eq!(blocks_lines, blocks_whole);
    assert_eq!(blocks_rand, blocks_whole);
}
