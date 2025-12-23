use mdstream::{
    AnalyzedStream, BlockHintAnalyzer, BlockHintMeta, BlockKind, CodeFenceAnalyzer, CodeFenceClass,
    FootnotesMode, IncompleteImageDropTransformer, IncompleteLinkPlaceholderTransformer, Options,
};

fn print_block(prefix: &str, id: u64, kind: BlockKind, text: &str) {
    let mut first = text.lines().next().unwrap_or("");
    if first.len() > 80 {
        first = &first[..80];
    }
    println!("{prefix} id={id} kind={kind:?} first_line={first:?}");
}

fn main() {
    let mut opts = Options {
        footnotes: FootnotesMode::SingleBlock,
        ..Options::default()
    };
    // For demo purposes, disable terminator link/image handling and enable the built-in
    // Streamdown-compatible pending transformers instead.
    opts.terminator.links = false;
    opts.terminator.images = false;

    // Chain analyzers: (code fence meta, pending hint meta)
    let analyzer = (CodeFenceAnalyzer::default(), BlockHintAnalyzer::default());
    let mut s = AnalyzedStream::new(opts, analyzer);
    s.inner_mut()
        .push_pending_transformer(IncompleteLinkPlaceholderTransformer::default());
    s.inner_mut()
        .push_pending_transformer(IncompleteImageDropTransformer::default());

    let chunks = [
        "# Streaming demo\n\n",
        "Normal text with **bold",
        " continued**.\n\n",
        "See [docs](",
        " and an image ![alt](",
        "...\n\n",
        "```mermaid\n",
        "graph TD;\nA-->B;\n",
        "```\n\n",
        "After code fence.\n",
        "\n$$\nE = mc^2\n",
        "$$\n",
    ];

    for (i, chunk) in chunks.iter().enumerate() {
        println!("\n== append step {i} ==");
        let u = s.append(chunk);

        for (block, meta) in u
            .update
            .committed
            .iter()
            .zip(u.committed_meta.iter().map(|m| &m.meta))
        {
            print_block("committed", block.id.0, block.kind, &block.raw);

            if let Some(code_meta) = &meta.0 {
                match code_meta.class {
                    CodeFenceClass::Mermaid => println!("  meta: code fence class=mermaid"),
                    CodeFenceClass::Json => println!("  meta: code fence class=json"),
                    CodeFenceClass::Other => {}
                }
            }
        }

        if let Some(p) = &u.update.pending {
            print_block("pending  ", p.id.0, p.kind, &p.raw);

            if let Some(pm) = &u.pending_meta {
                let hint = pm.meta.1.unwrap_or(mdstream::BlockHintMeta { flags: 0 });
                if hint.likely_incomplete() {
                    let mut flags = Vec::new();
                    if hint.has(BlockHintMeta::DISPLAY_TRANSFORMED) {
                        flags.push("display_transformed");
                    }
                    if hint.has(BlockHintMeta::UNCLOSED_CODE_FENCE) {
                        flags.push("unclosed_code_fence");
                    }
                    if hint.has(BlockHintMeta::UNBALANCED_MATH) {
                        flags.push("unbalanced_math");
                    }
                    println!("  hint: likely_incomplete flags={flags:?}");
                }
            }

            if let Some(display) = &p.display {
                if display != &p.raw {
                    println!("  display (pending only):");
                    println!("{display}");
                }
            }
        } else {
            println!("pending: <none>");
        }
    }

    println!("\n== finalize ==");
    let u = s.finalize();
    for b in &u.update.committed {
        print_block("committed", b.id.0, b.kind, &b.raw);
    }
    println!("pending: {:?}", u.update.pending.as_ref().map(|b| (b.id.0, b.kind)));
}
