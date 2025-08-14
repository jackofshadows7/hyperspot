use modkit_macros::module;

#[module(name="x", caps=[stateful], lifecycle(entry="serve", foo="bar"))]
pub struct X;

fn main() {}
