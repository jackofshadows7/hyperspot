use modkit_macros::module;

#[module(name="x", caps=[stateful], ctor="X::new()")]
pub struct X;

fn main() {}
