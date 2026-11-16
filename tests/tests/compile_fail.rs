//! Misuse the macros must reject at compile time, each case checked
//! against its expected compiler output in tests/compile_fail/.

#[test]
fn compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
