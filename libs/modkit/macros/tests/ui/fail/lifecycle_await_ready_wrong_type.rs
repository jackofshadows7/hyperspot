use modkit_macros::module;

#[module(name="x", caps=[stateful], lifecycle(entry="serve", await_ready="true"))]
pub struct X;

fn main() {}
