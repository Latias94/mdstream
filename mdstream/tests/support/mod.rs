#![allow(dead_code)]

use mdstream::StreamEngine;
use mdstream_conformance::{ChunkSchedule, NormalizedSnapshot};
use mdstream_protocol::{ApplyOutcome, Reducer};

pub fn replay(chunks: impl IntoIterator<Item = String>) -> NormalizedSnapshot {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    for chunk in chunks {
        apply(&mut reducer, engine.append(&chunk).unwrap());
    }
    apply(&mut reducer, engine.finish().unwrap());
    NormalizedSnapshot::from(reducer.document().unwrap().snapshot())
}

fn apply(reducer: &mut Reducer, output: mdstream::EngineOutput) {
    for change in output.into_changes() {
        assert!(matches!(
            reducer.apply(change).unwrap(),
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
    }
}

pub fn chunk_whole(text: &str) -> Vec<String> {
    schedule_chunks(text, &ChunkSchedule::Whole)
}

pub fn chunk_lines(text: &str) -> Vec<String> {
    schedule_chunks(text, &ChunkSchedule::Lines)
}

pub fn chunk_chars(text: &str) -> Vec<String> {
    schedule_chunks(text, &ChunkSchedule::Characters)
}

pub fn chunk_pseudo_random(
    text: &str,
    seed_label: &str,
    trial: u64,
    max_bytes: usize,
) -> Vec<String> {
    schedule_chunks(
        text,
        &ChunkSchedule::Seeded {
            label: seed_label.to_string(),
            seed: 0,
            trial,
            max_bytes,
        },
    )
}

fn schedule_chunks(text: &str, schedule: &ChunkSchedule) -> Vec<String> {
    schedule
        .slices(text)
        .expect("test chunk schedules are valid")
        .into_iter()
        .map(str::to_string)
        .collect()
}
