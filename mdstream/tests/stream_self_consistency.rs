use mdstream::{BlockKind, MdStream, Options};

fn snapshot_kinds_and_raw(stream: &mut MdStream) -> Vec<(BlockKind, String)> {
    stream
        .snapshot_blocks()
        .into_iter()
        .map(|block| (block.kind, block.raw))
        .collect()
}

#[test]
fn incremental_text_workload_matches_mdstream_scratch_parse_per_step() {
    // The workload shape comes from Streamdown's streaming benchmark. The
    // oracle is mdstream itself, so this is self-consistency rather than an
    // upstream compatibility claim.
    let base = "# Heading\n\n";
    let delta = "This is streaming text. ";

    let options = Options::default();
    let mut incremental = MdStream::new(options.clone());
    incremental.append(base);

    for index in 0..50 {
        if index > 0 {
            incremental.append(delta);
        }

        let step = format!("{base}{}", delta.repeat(index));
        let mut scratch = MdStream::new(options.clone());
        scratch.append(&step);

        let incremental_snapshot = snapshot_kinds_and_raw(&mut incremental);
        let scratch_snapshot = snapshot_kinds_and_raw(&mut scratch);
        assert_eq!(
            incremental_snapshot, scratch_snapshot,
            "step {index} mismatch"
        );
    }
}

#[test]
fn incremental_code_workload_matches_mdstream_scratch_parse_per_step() {
    // As above, only the workload is upstream-derived; both sides of the
    // equality are mdstream 0.3.
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

    let options = Options::default();
    let mut incremental = MdStream::new(options.clone());

    let mut previous = "";
    for (index, step) in steps.iter().enumerate() {
        let delta = step
            .strip_prefix(previous)
            .expect("step must extend previous");
        incremental.append(delta);

        let mut scratch = MdStream::new(options.clone());
        scratch.append(step);

        let incremental_snapshot = snapshot_kinds_and_raw(&mut incremental);
        let scratch_snapshot = snapshot_kinds_and_raw(&mut scratch);
        assert_eq!(
            incremental_snapshot, scratch_snapshot,
            "step {index} mismatch"
        );

        previous = step;
    }
}
