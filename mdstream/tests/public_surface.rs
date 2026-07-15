#[test]
fn obsolete_zero_three_surface_is_not_available() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/obsolete_zero_three_surface.rs");
    cases.compile_fail("tests/ui/obsolete_helpers.rs");
    cases.compile_fail("tests/ui/runtime_mutators.rs");
}

#[test]
fn intentional_zero_four_surface_is_available() {
    let cases = trybuild::TestCases::new();
    cases.pass("tests/ui/intentional_zero_four_surface.rs");
}
