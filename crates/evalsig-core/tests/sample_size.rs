//! `docs/sample-size.md` is written by `scripts/sample_size.py`; the Rust power module
//! must render the same file.

#[test]
fn rust_renders_the_committed_sample_size_table() {
    let committed = include_str!("../../../docs/sample-size.md");
    let rendered = evalsig_core::power::table();
    for (k, (a, b)) in committed.lines().zip(rendered.lines()).enumerate() {
        assert_eq!(a, b, "line {}", k + 1);
    }
    assert_eq!(committed, rendered);
}
