//! Compile-fail tests to verify type-safe API builder enforcement
//!
//! These tests ensure that the type-state pattern correctly prevents
//! invalid API operations from compiling.

#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/no_handler.rs");
    t.compile_fail("tests/ui/no_response.rs");
    t.compile_fail("tests/ui/no_handler_no_response.rs");
}
