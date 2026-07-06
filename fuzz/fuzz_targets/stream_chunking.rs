#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use mdstream::{
    BlockKind, DocumentState, FootnotesMode, MdStream, Options, ReferenceDefinitionsMode, Update,
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
    if input.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut start = 0usize;
    let mut split_index = 0usize;

    while start < input.len() {
        let width = split_bytes
            .get(split_index)
            .copied()
            .map(usize::from)
            .unwrap_or(16)
            .max(1);
        split_index += 1;

        let mut end = (start + width).min(input.len());
        while end < input.len() && !input.is_char_boundary(end) {
            end += 1;
        }

        out.push(&input[start..end]);
        start = end;
    }

    out
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
});
