use mdstream::{BlockKind, MdStream, Options};

pub fn collect_final_blocks(
    chunks: impl IntoIterator<Item = String>,
    opts: Options,
) -> Vec<(BlockKind, String)> {
    let mut s = MdStream::new(opts);
    let mut out = Vec::new();

    for chunk in chunks {
        let u = s.append(&chunk);
        out.extend(u.committed.into_iter().map(|b| (b.kind, b.raw)));
    }
    let u = s.finalize();
    out.extend(u.committed.into_iter().map(|b| (b.kind, b.raw)));
    out
}

pub fn chunk_whole(text: &str) -> Vec<String> {
    vec![text.to_string()]
}

pub fn chunk_lines(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(|s| s.to_string()).collect()
}

pub fn chunk_chars(text: &str) -> Vec<String> {
    text.chars().map(|c| c.to_string()).collect()
}

fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

pub fn chunk_pseudo_random(
    text: &str,
    seed_label: &str,
    trial: u64,
    max_bytes: usize,
) -> Vec<String> {
    assert!(max_bytes > 0);
    let mut state = fnv1a64(seed_label) ^ (trial.wrapping_mul(0x9e3779b97f4a7c15));

    let mut out = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        let want = (xorshift64(&mut state) as usize % max_bytes) + 1;
        let mut end = (start + want).min(text.len());
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        out.push(text[start..end].to_string());
        start = end;
    }
    out
}
