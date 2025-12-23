use mdstream::{BlockKind, MdStream, Options};

fn snapshot_kinds_and_raw(s: &MdStream) -> Vec<(BlockKind, String)> {
    s.snapshot_blocks()
        .into_iter()
        .map(|b| (b.kind, b.raw))
        .collect()
}

#[test]
fn streamdown_benchmark_streaming_text_steps_match_full_parse_per_step() {
    // Matches Streamdown's parse-blocks benchmark ("Streaming Simulation").
    let base = "# Heading\n\n";
    let delta = "This is streaming text. ";

    let opts = Options::default();
    let mut incremental = MdStream::new(opts.clone());
    incremental.append(base);

    for i in 0..50 {
        if i > 0 {
            incremental.append(delta);
        }

        let step = format!("{base}{}", delta.repeat(i));
        let mut scratch = MdStream::new(opts.clone());
        scratch.append(&step);

        assert_eq!(
            snapshot_kinds_and_raw(&incremental),
            snapshot_kinds_and_raw(&scratch),
            "step {i} mismatch"
        );
    }
}

#[test]
fn streamdown_benchmark_streaming_code_steps_match_full_parse_per_step() {
    // Matches Streamdown's parse-blocks benchmark ("Streaming Simulation").
    let steps = [
        "```javascript",
        "```javascript\n",
        "```javascript\nconst",
        "```javascript\nconst x",
        "```javascript\nconst x =",
        "```javascript\nconst x = 1",
        "```javascript\nconst x = 1;",
        "```javascript\nconst x = 1;\n",
        "```javascript\nconst x = 1;\n```",
    ];

    let opts = Options::default();
    let mut incremental = MdStream::new(opts.clone());

    let mut prev = "";
    for (i, step) in steps.iter().enumerate() {
        let delta = step.strip_prefix(prev).expect("step must extend previous");
        incremental.append(delta);

        let mut scratch = MdStream::new(opts.clone());
        scratch.append(step);

        assert_eq!(
            snapshot_kinds_and_raw(&incremental),
            snapshot_kinds_and_raw(&scratch),
            "step {i} mismatch"
        );

        prev = step;
    }
}
