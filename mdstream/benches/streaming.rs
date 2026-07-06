use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use mdstream::{MdStream, Options};
use std::hint::black_box;

const BASIC_MANY_BLOCKS: &str =
    include_str!("../tests/fixtures/streamdown_bench/basic_many_blocks_100.md");
const LARGE_TABLE: &str =
    include_str!("../tests/fixtures/streamdown_bench/table_large_100_rows.md");
const MIXED_CONTENT: &str =
    include_str!("../tests/fixtures/streamdown_bench/mixed_content_realistic.md");

fn large_code_fence() -> String {
    let mut text = String::from("```rust\n");
    for i in 0..1_000 {
        text.push_str("fn generated_");
        text.push_str(&i.to_string());
        text.push_str("() { println!(\"line ");
        text.push_str(&i.to_string());
        text.push_str("\"); }\n");
    }
    text.push_str("```\n");
    text
}

fn chunk_whole(text: &str) -> Vec<String> {
    vec![text.to_string()]
}

fn chunk_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_string).collect()
}

fn chunk_chars(text: &str) -> Vec<String> {
    text.chars().map(String::from).collect()
}

fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

fn chunk_pseudo_random(text: &str, seed_label: &str, max_bytes: usize) -> Vec<String> {
    assert!(max_bytes > 0);

    let mut state = fnv1a64(seed_label);
    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < text.len() {
        let want = (xorshift64(&mut state) as usize % max_bytes) + 1;
        let mut end = (start + want).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        chunks.push(text[start..end].to_string());
        start = end;
    }

    chunks
}

fn run_owned(chunks: &[String]) -> usize {
    let mut stream = MdStream::new(Options::default());
    let mut observed = 0usize;

    for chunk in chunks {
        let update = stream.append(chunk);
        observed = observed.wrapping_add(update.committed.len());
        if let Some(pending) = update.pending {
            observed = observed.wrapping_add(pending.display_or_raw().len());
        }
        observed = observed.wrapping_add(update.invalidated.len());
    }

    let update = stream.finalize();
    observed = observed.wrapping_add(update.committed.len());
    observed = observed.wrapping_add(update.invalidated.len());
    observed
}

fn run_borrowed(chunks: &[String]) -> usize {
    let mut stream = MdStream::new(Options::default());
    let mut observed = 0usize;

    for chunk in chunks {
        let update = stream.append_ref(chunk);
        observed = observed.wrapping_add(update.committed.len());
        if let Some(pending) = update.pending {
            observed = observed.wrapping_add(pending.display_or_raw().len());
        }
        observed = observed.wrapping_add(update.invalidated.len());
    }

    let update = stream.finalize_ref();
    observed = observed.wrapping_add(update.committed.len());
    observed = observed.wrapping_add(update.invalidated.len());
    observed
}

fn streaming_benchmarks(c: &mut Criterion) {
    let scenarios = vec![
        ("many_blocks_whole", chunk_whole(BASIC_MANY_BLOCKS)),
        ("many_blocks_lines", chunk_lines(BASIC_MANY_BLOCKS)),
        (
            "mixed_random_chunks",
            chunk_pseudo_random(MIXED_CONTENT, "mixed_content", 40),
        ),
        ("large_table_lines", chunk_lines(LARGE_TABLE)),
        ("large_table_chars", chunk_chars(LARGE_TABLE)),
        {
            let text = large_code_fence();
            ("large_code_fence_lines", chunk_lines(&text))
        },
        {
            let text = large_code_fence();
            (
                "large_code_fence_random",
                chunk_pseudo_random(&text, "large_code_fence", 80),
            )
        },
    ];

    let mut group = c.benchmark_group("streaming");

    for (name, chunks) in scenarios {
        let bytes: u64 = chunks.iter().map(|chunk| chunk.len() as u64).sum();
        group.throughput(Throughput::Bytes(bytes));

        group.bench_with_input(
            BenchmarkId::new("append_owned", name),
            &chunks,
            |b, chunks| {
                b.iter_batched(
                    || chunks.clone(),
                    |chunks| black_box(run_owned(&chunks)),
                    BatchSize::SmallInput,
                );
            },
        );

        group.bench_with_input(
            BenchmarkId::new("append_borrowed", name),
            &chunks,
            |b, chunks| {
                b.iter_batched(
                    || chunks.clone(),
                    |chunks| black_box(run_borrowed(&chunks)),
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, streaming_benchmarks);
criterion_main!(benches);
