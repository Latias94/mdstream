#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use mdstream::pending::{TerminatorOptions, terminate_markdown};

#[derive(Arbitrary, Debug)]
struct TerminatorCase {
    input: String,
    mode_bits: u8,
    window_hint: Option<u16>,
}

fn options(case: &TerminatorCase) -> TerminatorOptions {
    TerminatorOptions {
        setext_headings: case.mode_bits & 0b0000_0001 != 0,
        links: case.mode_bits & 0b0000_0010 != 0,
        images: case.mode_bits & 0b0000_0100 != 0,
        emphasis: case.mode_bits & 0b0000_1000 != 0,
        inline_code: case.mode_bits & 0b0001_0000 != 0,
        strikethrough: case.mode_bits & 0b0010_0000 != 0,
        katex_block: case.mode_bits & 0b0100_0000 != 0,
        window_bytes: usize::from(case.window_hint.unwrap_or(4096)).max(1),
        ..TerminatorOptions::default()
    }
}

fuzz_target!(|case: TerminatorCase| {
    let input: String = case.input.chars().take(4096).collect();
    let opts = options(&case);
    let output = terminate_markdown(&input, &opts);

    assert!(output.len() <= input.len() + opts.incomplete_link_url.len() + 128);
});
