#![allow(dead_code)]

use mdstream::StreamEngine;
use mdstream_conformance::{ChunkSchedule, NormalizedSnapshot};
use mdstream_protocol::{ApplyOutcome, Reducer, TransitionReducer};

pub fn replay(chunks: impl IntoIterator<Item = String>) -> NormalizedSnapshot {
    let mut engine = StreamEngine::new();
    let mut reducer = Reducer::new();
    let mut transition_reducer = TransitionReducer::new();
    for chunk in chunks {
        apply(
            &mut reducer,
            &mut transition_reducer,
            engine.append(&chunk).unwrap(),
        );
    }
    apply(
        &mut reducer,
        &mut transition_reducer,
        engine.finish().unwrap(),
    );
    let snapshot = reducer.document().unwrap().snapshot();
    assert_eq!(
        Some(snapshot.clone()),
        transition_reducer
            .document()
            .map(|document| document.snapshot())
    );
    NormalizedSnapshot::from(snapshot)
}

fn apply(
    reducer: &mut Reducer,
    transition_reducer: &mut TransitionReducer,
    output: mdstream::EngineOutput,
) {
    for change in output.into_changes() {
        let outcome = reducer.apply(change.clone()).unwrap();
        assert!(matches!(
            outcome,
            ApplyOutcome::Applied { .. } | ApplyOutcome::Recovered { .. }
        ));
        let observed = transition_reducer.apply(change).unwrap();
        assert_eq!(observed.outcome, outcome);
        assert!(
            observed.facts.is_some(),
            "every state-changing engine change must emit transition facts"
        );
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
