#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use mdstream::{
    BlockKind, DocumentState, FootnotesMode, MdStream, Options, ReferenceDefinitionsMode, Update,
};
use mdstream_conformance::{
    ProtocolTrace, assert_last_retry_idempotent, replay_protocol_trace,
    source_only_trace, utf8_ranges_from_target_widths,
};
use mdstream_protocol::{
    ApplyOutcome, ChangeId, ChangeSet, Epoch, ProjectionOp, RecoveryReason, Reducer, Sequence,
    SourceDelta,
};

#[derive(Arbitrary, Debug)]
struct StreamCase {
    input: String,
    split_bytes: Vec<u8>,
    mode_bits: u8,
    max_buffer_hint: Option<u16>,
}

fn options(case: &StreamCase) -> Options {
    Options {
        footnotes: if case.mode_bits & 0b0001 != 0 {
            FootnotesMode::Invalidate
        } else {
            FootnotesMode::SingleBlock
        },
        reference_definitions: if case.mode_bits & 0b0010 != 0 {
            ReferenceDefinitionsMode::Invalidate
        } else {
            ReferenceDefinitionsMode::StabilityFirst
        },
        max_buffer_bytes: (case.mode_bits & 0b0100 != 0)
            .then(|| usize::from(case.max_buffer_hint.unwrap_or(4096)).max(256)),
        ..Options::default()
    }
}

fn bounded_input(input: &str) -> String {
    input.chars().take(4096).collect()
}

fn chunks<'a>(input: &'a str, split_bytes: &[u8]) -> Vec<&'a str> {
    utf8_ranges_from_target_widths(
        input,
        split_bytes.iter().copied().map(usize::from).map(|width| width.max(1)),
        16,
    )
    .expect("fuzz widths and fallback are non-zero")
    .into_iter()
    .map(|range| &input[range])
    .collect()
}

fn apply_update(state: &mut DocumentState, update: Update) {
    state.apply(update);
}

fn collect(chunks: &[&str], opts: Options, borrowed: bool) -> (Vec<(BlockKind, String)>, DocumentState) {
    let mut stream = MdStream::new(opts);
    let mut state = DocumentState::default();

    for chunk in chunks {
        let update = if borrowed {
            stream.append_ref(chunk).to_owned()
        } else {
            stream.append(chunk)
        };
        apply_update(&mut state, update);
    }

    let update = if borrowed {
        stream.finalize_ref().to_owned()
    } else {
        stream.finalize()
    };
    apply_update(&mut state, update);

    let blocks = state
        .blocks()
        .map(|block| (block.kind, block.raw.clone()))
        .collect();
    (blocks, state)
}

fn change_id(value: String) -> ChangeId {
    ChangeId::new(value).expect("bounded fuzz change IDs are valid")
}

fn source_trace(chunks: &[&str]) -> ProtocolTrace {
    source_only_trace("fuzz", "fuzz-widths", Epoch::new(1), chunks.iter().copied())
        .expect("source-only fuzz trace is canonical")
}

fn assert_protocol_laws(input: &str, chunks: &[&str]) {
    let trace = source_trace(chunks);
    let replay = replay_protocol_trace(&trace).expect("constructed trace must replay");
    assert_eq!(replay.final_snapshot.source(), input);
    assert_last_retry_idempotent(&trace).expect("last change retry must be idempotent");

    let mut reducer = Reducer::new();
    for change in trace.changes {
        reducer.apply(change).expect("canonical fuzz trace must apply");
    }
    let document = reducer.document().expect("trace installs a document");
    let illegal_epoch = ChangeSet::new(
        Epoch::new(2),
        Sequence::new(1),
        change_id("fuzz:unannounced-epoch".to_string()),
        SourceDelta::unchanged(document.coordinate().source_cursor),
        vec![ProjectionOp::FinishDocument],
    )
    .expect("unannounced epoch is envelope-valid but state-invalid");
    assert!(matches!(
        reducer.apply(illegal_epoch).expect("routing returns a typed outcome"),
        ApplyOutcome::RecoveryRequired {
            reason: RecoveryReason::UnannouncedEpoch { .. },
            ..
        }
    ));
}

fuzz_target!(|case: StreamCase| {
    let input = bounded_input(&case.input);
    let opts = options(&case);
    let whole = [input.as_str()];
    let split = chunks(&input, &case.split_bytes);

    let (whole_owned, whole_state) = collect(&whole, opts.clone(), false);
    let (split_owned, split_state) = collect(&split, opts.clone(), false);
    assert_eq!(split_owned, whole_owned);

    let (split_borrowed, borrowed_state) = collect(&split, opts, true);
    assert_eq!(split_borrowed, whole_owned);
    assert_eq!(split_state.blocks().count(), borrowed_state.blocks().count());
    assert_eq!(whole_state.blocks().count(), whole_owned.len());
    assert_protocol_laws(&input, &split);
});
