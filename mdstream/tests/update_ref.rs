use mdstream::{MdStream, Options};

#[test]
fn append_ref_returns_pending_for_plain_text() {
    let mut s = MdStream::new(Options::default());
    let u = s.append_ref("Hello");
    assert!(u.committed.is_empty());
    let p = u.pending.expect("pending must exist");
    assert_eq!(p.raw, "Hello");
    assert!(p.display_or_raw().contains("Hello"));
}

#[test]
fn append_ref_code_fence_pending_display_is_closed() {
    let mut s = MdStream::new(Options::default());

    let u1 = s.append_ref("```rs\nfn main() {\n");
    assert!(u1.committed.is_empty());
    let p1 = u1.pending.expect("pending must exist");
    let d1 = p1.display.expect("pending display must exist");
    assert!(d1.contains("fn main()"));
    assert!(d1.ends_with("```\n"), "display must end with a closing fence");

    let u2 = s.append_ref("}\n");
    assert!(u2.committed.is_empty());
    let p2 = u2.pending.expect("pending must exist");
    let d2 = p2.display.expect("pending display must exist");
    assert!(d2.contains("}\n"));
    assert!(d2.ends_with("```\n"), "display must end with a closing fence");
}

