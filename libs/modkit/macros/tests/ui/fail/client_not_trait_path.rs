use modkit_macros::module;

#[module(name="x", caps=[stateful], client=123)]
pub struct X;

fn main() {}
