use std::{hint::black_box, time::Duration};

use criterion::{
    BatchSize, BenchmarkId, Criterion, SamplingMode, Throughput, criterion_group, criterion_main,
};
use mdstream::{EngineOutput, StreamEngine};
use mdstream_conformance::CanonicalPendingScenario;
use mdstream_protocol::{ApplyOutcome, Reducer};

const BASIC_MANY_BLOCKS: &str =
    include_str!("../tests/fixtures/streamdown_bench/basic_many_blocks_100.md");
const LARGE_TABLE: &str =
    include_str!("../tests/fixtures/streamdown_bench/table_large_100_rows.md");
const MIXED_CONTENT: &str =
    include_str!("../tests/fixtures/streamdown_bench/mixed_content_realistic.md");

fn apply(reducer: &mut Reducer, output: EngineOutput) -> usize {
    output
        .into_changes()
        .into_iter()
        .map(|change| {
            let cost = change.source().suffix.len() + change.operations().len();
            assert!(matches!(
                reducer.apply(change).unwrap(),
                ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
            ));
            cost
        })
        .sum()
}

fn run_engine_reducer(chunks: &[String]) -> usize {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut checksum = 0usize;
    for chunk in chunks {
        checksum = checksum.wrapping_add(apply(&mut reducer, engine.append(chunk).unwrap()));
    }
    checksum = checksum.wrapping_add(apply(&mut reducer, engine.finish().unwrap()));
    let document = reducer.document().unwrap();
    checksum
        .wrapping_add(document.source().len())
        .wrapping_add(document.nodes().len())
}

fn characters(text: &str) -> Vec<String> {
    text.chars().map(String::from).collect()
}

fn lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_string).collect()
}

fn streaming_benchmarks(criterion: &mut Criterion) {
    let scenarios = [
        ("many-blocks-lines", lines(BASIC_MANY_BLOCKS)),
        ("mixed-characters", characters(MIXED_CONTENT)),
        ("large-table-lines", lines(LARGE_TABLE)),
    ];
    let mut group = criterion.benchmark_group("stream_engine_reducer");
    for (name, chunks) in scenarios {
        group.throughput(Throughput::Bytes(
            chunks.iter().map(|chunk| chunk.len() as u64).sum(),
        ));
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &chunks,
            |bench, chunks| {
                bench.iter_batched(
                    || chunks.clone(),
                    |chunks| black_box(run_engine_reducer(&chunks)),
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn canonical_pending_benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("canonical_pending");
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_millis(100));
    group.measurement_time(Duration::from_secs(1));

    for scenario in CanonicalPendingScenario::ALL {
        let source = scenario.source();
        group.throughput(Throughput::Bytes(source.len() as u64));
        group.bench_with_input(
            BenchmarkId::new(scenario.shape().id(), scenario.target_bytes()),
            &source,
            |bench, source| {
                bench.iter(|| black_box(run_engine_reducer(&characters(black_box(source)))));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, streaming_benchmarks, canonical_pending_benchmarks);
criterion_main!(benches);
