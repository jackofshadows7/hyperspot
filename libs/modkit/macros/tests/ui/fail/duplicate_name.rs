use modkit_macros::module;

#[module(name="a", name="b", caps=[stateful])]
pub struct X;

fn main() {}
