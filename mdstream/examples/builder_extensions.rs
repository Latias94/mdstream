//! Extension-heavy `mdstream` usage with `MdStreamBuilder`.
//!
//! Run:
//!   cargo run -p mdstream --example builder_extensions

use mdstream::{ContainerBoundaryPlugin, IncompleteLinkPlaceholderTransformer, MdStream, Options};

fn main() {
    let mut stream = MdStream::builder(Options::default())
        .boundary_plugin(ContainerBoundaryPlugin::default())
        .pending_transformer(IncompleteLinkPlaceholderTransformer::default())
        .build();

    let chunks = [
        "::: note\n",
        "Use the builder when streams need plugins or transformers.\n",
        ":::\n\n",
        "See [docs](",
    ];

    for (i, chunk) in chunks.iter().enumerate() {
        println!("\n== tick {i} ==");
        let update = stream.append(chunk);

        for block in &update.committed {
            println!(
                "committed id={} kind={:?} text={:?}",
                block.id.0, block.kind, block.raw
            );
        }

        if let Some(pending) = &update.pending {
            println!(
                "pending id={} kind={:?} raw={:?}",
                pending.id.0, pending.kind, pending.raw
            );
            if let Some(display) = &pending.display {
                println!("pending display={display:?}");
            }
        } else {
            println!("pending: <none>");
        }
    }

    println!("\n== finalize ==");
    let update = stream.finalize();
    println!("final committed={}", update.committed.len());
}
