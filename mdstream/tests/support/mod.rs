#![allow(dead_code)]

use mdstream::{BlockKind, MdStream, Options, Update};
use mdstream_conformance::ChunkSchedule;

pub fn collect_final_blocks(
    chunks: impl IntoIterator<Item = String>,
    opts: Options,
) -> Vec<(BlockKind, String)> {
    let s = MdStream::new(opts);
    collect_final_blocks_with_stream(chunks, s)
}

pub fn collect_final_raw(chunks: impl IntoIterator<Item = String>, opts: Options) -> Vec<String> {
    collect_final_blocks(chunks, opts)
        .into_iter()
        .map(|(_, raw)| raw)
        .collect()
}

pub fn collect_final_blocks_with_stream(
    chunks: impl IntoIterator<Item = String>,
    mut s: MdStream,
) -> Vec<(BlockKind, String)> {
    let mut out = Vec::new();

    for chunk in chunks {
        let u = s.append(&chunk);
        apply_update(&mut out, u);
    }
    let u = s.finalize();
    apply_update(&mut out, u);
    out
}

pub fn collect_final_blocks_borrowed(
    chunks: impl IntoIterator<Item = String>,
    opts: Options,
) -> Vec<(BlockKind, String)> {
    let mut s = MdStream::new(opts);
    let mut out = Vec::new();

    for chunk in chunks {
        let u = s.append_ref(&chunk).to_owned();
        apply_update(&mut out, u);
    }
    let u = s.finalize_ref().to_owned();
    apply_update(&mut out, u);
    out
}

fn apply_update(out: &mut Vec<(BlockKind, String)>, update: Update) {
    if update.reset {
        out.clear();
    }
    out.extend(update.committed.into_iter().map(|b| (b.kind, b.raw)));
}

pub fn collect_final_raw_with_stream(
    chunks: impl IntoIterator<Item = String>,
    s: MdStream,
) -> Vec<String> {
    collect_final_blocks_with_stream(chunks, s)
        .into_iter()
        .map(|(_, raw)| raw)
        .collect()
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
