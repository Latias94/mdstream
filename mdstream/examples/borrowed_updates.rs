use mdstream::{MdStream, Options};

fn main() {
    let mut s = MdStream::new(Options::default());

    let chunks = ["# Title\n\n", "```rs\nfn main() {\n", "    println!(\"hi\");\n", "}\n"];
    for (i, chunk) in chunks.iter().enumerate() {
        let u = s.append_ref(chunk);
        println!("== step {i} ==");
        println!("committed: {}", u.committed.len());
        if let Some(p) = u.pending {
            println!("pending kind={:?}", p.kind);
            println!("{}", p.display_or_raw());
        }
    }
}

